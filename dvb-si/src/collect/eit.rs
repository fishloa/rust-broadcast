use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::descriptors::DescriptorRegistry;
use crate::tables::RunningStatus;
use crate::tables::eit;
use broadcast_common::Parse;
use mpeg_ts::section::Section;

use super::{
    CollectError, CollectResult, CompleteSectionSet, ParsedDescriptorLoop, SectionSetKey,
    SectionSetMeta,
};

/// Default cap on the number of logical keys (in-progress AND completed
/// section sets + schedule ranges) retained by [`EitCollector`].
///
/// 256 is a floor for standalone use of this collector, bounding a hostile
/// stream that rotates `original_network_id` / `transport_stream_id` /
/// `service_id` (or `current_next_indicator`) to force unbounded map growth —
/// **not** a claim about real network size: EIT *other* (table_ids
/// `0x4F`/`0x60..=0x6F`) covers the whole network, and each service can use
/// one present/following key plus up to 16 schedule table_ids, so a
/// multiplex with 150+ services can need thousands of keys (issue #1002). A
/// caller with a realistic service count should size this explicitly — see
/// [`crate::epg::EpgStore`], which derives it from `max_services`. The cap
/// is applied independently to the sections map and the schedules map; each
/// is limited to `max_logical_keys` entries. When a map is full, incoming
/// sections for a *new* key evict the least-recently-touched existing key
/// in that map (see [`EitCollector`]'s type-level docs) rather than being
/// silently dropped; [`clear`](EitCollector::clear) or
/// [`retain_logical`](EitCollector::retain_logical) still free capacity for
/// an application-defined boundary.
pub const DEFAULT_MAX_LOGICAL_KEYS: usize = 256;

/// EIT-specific collector.
///
/// Present/following EITs complete as one normal section set. Schedule EITs
/// complete only when every schedule table_id from the kind's first table_id
/// through the advertised `last_table_id` has completed its own section set.
///
/// # Memory bounds
///
/// The collector is bounded by [`DEFAULT_MAX_LOGICAL_KEYS`] (configurable via
/// [`with_max_logical_keys`](Self::with_max_logical_keys)). When the sections
/// or schedules map is full, the new key evicts the least-recently-touched
/// existing key in that same map (issue #1002) — a completed entry no
/// longer holds capacity hostage forever; see the type-level docs on
/// [`crate::collect::SectionSetCollector`] for the rationale, which applies
/// identically here. [`sections_evicted_for_capacity`](Self::sections_evicted_for_capacity)
/// and [`schedules_evicted_for_capacity`](Self::schedules_evicted_for_capacity)
/// expose the eviction counts.
#[derive(Debug)]
pub struct EitCollector {
    sections: BTreeMap<EitSectionSetKey, PartialEitSectionSet>,
    sections_touch_order: BTreeMap<u64, EitSectionSetKey>,
    sections_tick: u64,
    sections_evicted_for_capacity: u64,
    schedules: BTreeMap<EitLogicalKey, PartialEitSchedule>,
    schedules_touch_order: BTreeMap<u64, EitLogicalKey>,
    schedules_tick: u64,
    schedules_evicted_for_capacity: u64,
    max_logical_keys: usize,
}

impl Default for EitCollector {
    fn default() -> Self {
        Self {
            sections: BTreeMap::new(),
            sections_touch_order: BTreeMap::new(),
            sections_tick: 0,
            sections_evicted_for_capacity: 0,
            schedules: BTreeMap::new(),
            schedules_touch_order: BTreeMap::new(),
            schedules_tick: 0,
            schedules_evicted_for_capacity: 0,
            max_logical_keys: DEFAULT_MAX_LOGICAL_KEYS,
        }
    }
}

impl EitCollector {
    /// Create an empty EIT collector with the default cap
    /// ([`DEFAULT_MAX_LOGICAL_KEYS`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the logical-key cap (default [`DEFAULT_MAX_LOGICAL_KEYS`]).
    /// The cap is applied independently to the sections and schedules maps.
    /// When a *new* key arrives at capacity, the least-recently-touched
    /// existing key in that map is evicted to admit it (see the type-level
    /// docs).
    #[must_use]
    pub fn with_max_logical_keys(mut self, max_logical_keys: usize) -> Self {
        self.max_logical_keys = max_logical_keys;
        self
    }

    /// Number of section-set keys evicted so far to admit a new key at
    /// capacity. A nonzero, growing count means the cap is too small for the
    /// stream's distinct-key churn.
    #[must_use]
    pub fn sections_evicted_for_capacity(&self) -> u64 {
        self.sections_evicted_for_capacity
    }

    /// Number of schedule logical keys evicted so far to admit a new key at
    /// capacity.
    #[must_use]
    pub fn schedules_evicted_for_capacity(&self) -> u64 {
        self.schedules_evicted_for_capacity
    }

    /// Push one complete EIT section.
    ///
    /// Returns `Some` for a completed present/following table or a completed
    /// schedule table-id range.
    ///
    /// # Errors
    ///
    /// Returns a [`CollectError`] if the incoming section is malformed,
    /// inconsistent with already retained bytes, or not an EIT section. Treat
    /// the error as applying to this section only unless your application wants
    /// strict stream-fail behavior.
    pub fn push_section(&mut self, bytes: impl AsRef<[u8]>) -> CollectResult<Option<CompletedEit>> {
        self.push_section_with_pid(None, bytes)
    }

    /// Push one complete EIT section with PID context.
    pub fn push_section_with_pid(
        &mut self,
        pid: Option<u16>,
        bytes: impl AsRef<[u8]>,
    ) -> CollectResult<Option<CompletedEit>> {
        let raw = bytes.as_ref();
        let section = Section::parse(raw)?;
        if !section.section_syntax_indicator {
            return Err(CollectError::ShortFormSection {
                table_id: section.table_id,
            });
        }
        if section.section_number > section.last_section_number {
            return Err(CollectError::SectionNumberOutOfRange {
                table_id: section.table_id,
                section_number: section.section_number,
                last_section_number: section.last_section_number,
            });
        }
        section.validate_crc(raw)?;

        let eit = eit::EitSection::parse(raw)?;
        let logical_key = EitLogicalKey {
            pid,
            kind: eit.kind,
            service_id: eit.service_id,
            transport_stream_id: eit.transport_stream_id,
            original_network_id: eit.original_network_id,
            current_next_indicator: eit.current_next_indicator,
        };
        let key = EitSectionSetKey {
            logical_key,
            table_id: eit.table_id,
        };
        let meta = EitSectionSetMeta {
            key,
            version_number: eit.version_number,
            last_section_number: eit.last_section_number,
        };
        let bytes: Arc<[u8]> = Arc::from(raw);

        // Cap check: sections map — evict the least-recently-touched key if
        // `key` is new and the map is already full (issue #1002).
        self.sections_tick = self.sections_tick.wrapping_add(1).max(1);
        if !self.sections.contains_key(&key) && self.sections.len() >= self.max_logical_keys {
            if let Some((&oldest_tick, &oldest_key)) = self.sections_touch_order.iter().next() {
                self.sections_touch_order.remove(&oldest_tick);
                self.sections.remove(&oldest_key);
                self.sections_evicted_for_capacity += 1;
            } else {
                return Ok(None);
            }
        }
        let sections_now = self.sections_tick;

        let partial = self
            .sections
            .entry(key)
            .or_insert_with(|| PartialEitSectionSet::new(meta));
        if partial.meta.version_number != meta.version_number
            || partial.meta.last_section_number != meta.last_section_number
        {
            partial.reset(meta);
        }
        self.sections_touch_order.remove(&partial.last_touch);
        partial.last_touch = sections_now;
        self.sections_touch_order.insert(sections_now, key);

        partial.insert(eit.section_number, eit.segment_last_section_number, bytes)?;
        let complete = match partial.to_complete() {
            Some(complete) => complete,
            None => return Ok(None),
        };

        match eit.kind {
            eit::EitKind::PresentFollowingActual | eit::EitKind::PresentFollowingOther => {
                partial.emitted = true;
                Ok(Some(CompletedEit::PresentFollowing(complete)))
            }
            eit::EitKind::ScheduleActual | eit::EitKind::ScheduleOther => {
                let first_table_id = match eit.kind {
                    eit::EitKind::ScheduleActual => eit::TABLE_ID_SCHEDULE_ACTUAL_FIRST,
                    eit::EitKind::ScheduleOther => eit::TABLE_ID_SCHEDULE_OTHER_FIRST,
                    _ => unreachable!("matched schedule kind above"),
                };
                if eit.table_id < first_table_id || eit.table_id > eit.last_table_id {
                    return Err(CollectError::EitTableIdOutOfRange {
                        table_id: eit.table_id,
                        first_table_id,
                        last_table_id: eit.last_table_id,
                    });
                }

                // Cap check: schedules map (before marking the section set
                // emitted) — evict the least-recently-touched key if
                // `logical_key` is new and the map is already full.
                self.schedules_tick = self.schedules_tick.wrapping_add(1).max(1);
                if !self.schedules.contains_key(&logical_key)
                    && self.schedules.len() >= self.max_logical_keys
                {
                    match self.schedules_touch_order.iter().next() {
                        Some((&oldest_tick, &oldest_key)) => {
                            self.schedules_touch_order.remove(&oldest_tick);
                            self.schedules.remove(&oldest_key);
                            self.schedules_evicted_for_capacity += 1;
                        }
                        None => return Ok(None),
                    }
                }
                let schedules_now = self.schedules_tick;

                partial.emitted = true;

                let schedule_meta = EitScheduleMeta {
                    key: logical_key,
                    first_table_id,
                    last_table_id: eit.last_table_id,
                };

                // A schedule range reset (this section's `last_table_id`
                // differs from what the schedule previously tracked, a
                // non-conformant "flapping `last_table_id`" stream) used to
                // drop every other table_id's already-completed section set.
                // Those sub-tables are already marked `emitted` and won't
                // repeat under a new version, so the schedule could never
                // complete again. Snapshot them here (ignoring `emitted`)
                // before the reset so they can be re-fed.
                let retained_table_sets: Vec<(u8, CompleteSectionSet)> =
                    if self.schedules.get(&logical_key).is_some_and(|existing| {
                        existing.meta.last_table_id != schedule_meta.last_table_id
                    }) {
                        (first_table_id..=schedule_meta.last_table_id)
                            .filter(|&table_id| table_id != eit.table_id)
                            .filter_map(|table_id| {
                                self.sections
                                    .get(&EitSectionSetKey {
                                        logical_key,
                                        table_id,
                                    })
                                    .and_then(PartialEitSectionSet::snapshot)
                                    .map(|set| (table_id, set))
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };

                let schedule = self
                    .schedules
                    .entry(logical_key)
                    .or_insert_with(|| PartialEitSchedule::new(schedule_meta));
                if schedule.meta.last_table_id != schedule_meta.last_table_id {
                    schedule.reset(schedule_meta);
                    for (table_id, set) in retained_table_sets {
                        schedule.insert(table_id, set);
                    }
                }
                self.schedules_touch_order.remove(&schedule.last_touch);
                schedule.last_touch = schedules_now;
                self.schedules_touch_order
                    .insert(schedules_now, logical_key);
                schedule.insert(eit.table_id, complete);
                if let Some(complete) = schedule.to_complete() {
                    schedule.emitted = true;
                    Ok(Some(CompletedEit::Schedule(complete)))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Drop all retained EIT partial and completed schedule state.
    ///
    /// Long-running receivers that collect EPG data continuously can call this
    /// at an application-defined carousel boundary if they do not need older
    /// schedule state.
    pub fn clear(&mut self) {
        self.sections.clear();
        self.sections_touch_order.clear();
        self.schedules.clear();
        self.schedules_touch_order.clear();
    }

    /// Retain only logical EIT keys accepted by `keep`.
    ///
    /// This is the explicit pruning hook for long-running EIT schedule
    /// collection. Both in-progress section sets and completed schedule ranges
    /// for rejected keys are removed.
    pub fn retain_logical<F>(&mut self, mut keep: F)
    where
        F: FnMut(&EitLogicalKey) -> bool,
    {
        self.sections.retain(|key, _| keep(&key.logical_key));
        // Drop touch_order entries left dangling by the retain above — a
        // stale entry pointing at a now-removed key would otherwise let a
        // future capacity eviction "succeed" against a key that is already
        // gone, silently leaving the map one over max_logical_keys.
        let sections = &self.sections;
        self.sections_touch_order
            .retain(|_, key| sections.contains_key(key));

        self.schedules.retain(|key, _| keep(key));
        let schedules = &self.schedules;
        self.schedules_touch_order
            .retain(|_, key| schedules.contains_key(key));
    }

    /// Number of retained EIT section-set states.
    #[must_use]
    pub fn section_set_len(&self) -> usize {
        self.sections.len()
    }

    /// Number of retained EIT logical schedule states.
    #[must_use]
    pub fn schedule_len(&self) -> usize {
        self.schedules.len()
    }
}

/// Completed EIT collection result.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CompletedEit {
    /// One completed present/following EIT section set.
    PresentFollowing(CompleteSectionSet),
    /// A completed schedule EIT range spanning one or more table IDs.
    Schedule(CompleteEitSchedule),
}

impl CompletedEit {
    /// Parse the completed EIT table(s) without a descriptor registry.
    pub fn tables(&self) -> crate::Result<Vec<CompleteEit<'_>>> {
        self.tables_with_registry(None)
    }

    /// Parse the completed EIT table(s) with an optional descriptor registry.
    pub fn tables_with_registry<'a>(
        &'a self,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Vec<CompleteEit<'a>>> {
        match self {
            Self::PresentFollowing(set) => Ok(vec![CompleteEit::parse(set, registry)?]),
            Self::Schedule(schedule) => schedule.tables_with_registry(registry),
        }
    }
}

/// Logical EIT table key used by [`EitCollector`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EitLogicalKey {
    /// Optional PID context supplied by the caller.
    pub pid: Option<u16>,
    /// EIT kind derived from table_id.
    pub kind: eit::EitKind,
    /// service_id.
    pub service_id: u16,
    /// transport_stream_id.
    pub transport_stream_id: u16,
    /// original_network_id.
    pub original_network_id: u16,
    /// current_next_indicator.
    pub current_next_indicator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct EitSectionSetKey {
    logical_key: EitLogicalKey,
    table_id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EitScheduleMeta {
    key: EitLogicalKey,
    first_table_id: u8,
    last_table_id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EitSectionSetMeta {
    key: EitSectionSetKey,
    version_number: u8,
    last_section_number: u8,
}

/// Number of section numbers grouped into one 3-hour EIT schedule segment
/// (ETSI EN 300 468 §5.2.4: `segment_last_section_number`).
const EIT_SEGMENT_SIZE: u8 = 8;

#[derive(Debug)]
struct PartialEitSectionSet {
    meta: EitSectionSetMeta,
    slots: Vec<Option<Arc<[u8]>>>,
    filled: usize,
    emitted: bool,
    /// Per-segment (`section_number / 8`) `segment_last_section_number` as
    /// observed from any section received from that segment.
    ///
    /// Present/following EITs and small schedules never need more than one
    /// entry here; full EIT-schedule sub-tables span up to 32 segments. A
    /// sparse segment (no events) still transmits exactly one section — with
    /// `segment_last_section_number` equal to that segment's first section —
    /// so this map always gets populated for every segment that will ever
    /// exist, without waiting for 8 sections that will never arrive.
    segment_ends: BTreeMap<u8, u8>,
    /// The collector's `sections_tick` at the last touch (push) of this key;
    /// `0` is a sentinel meaning "never touched yet" (the first real tick is
    /// `1`), used to find the least-recently-touched entry to evict.
    last_touch: u64,
}

impl PartialEitSectionSet {
    fn new(meta: EitSectionSetMeta) -> Self {
        let len = meta.last_section_number as usize + 1;
        Self {
            meta,
            slots: vec![None; len],
            filled: 0,
            emitted: false,
            segment_ends: BTreeMap::new(),
            last_touch: 0,
        }
    }

    fn reset(&mut self, meta: EitSectionSetMeta) {
        let last_touch = self.last_touch;
        *self = Self::new(meta);
        self.last_touch = last_touch;
    }

    fn insert(
        &mut self,
        section_number: u8,
        segment_last_section_number: u8,
        bytes: Arc<[u8]>,
    ) -> CollectResult<bool> {
        let index = section_number as usize;
        if let Some(existing) = &self.slots[index] {
            if existing.as_ref() == bytes.as_ref() {
                return Ok(false);
            }
            return Err(CollectError::ConflictingSection {
                table_id: self.meta.key.table_id,
                section_number,
            });
        }

        self.slots[index] = Some(bytes);
        self.filled += 1;
        self.emitted = false;
        self.segment_ends.insert(
            section_number / EIT_SEGMENT_SIZE,
            segment_last_section_number,
        );
        Ok(true)
    }

    /// Whether every section that this section set's observed segments
    /// declare (via `segment_last_section_number`) has been received.
    ///
    /// Unlike a plain PSI/SI table, an EIT schedule sub-table need not fill
    /// every slot in `0..=last_section_number` — a segment that carries no
    /// events transmits only its first section, with
    /// `segment_last_section_number` naming that same section (ETSI EN 300
    /// 468 §5.2.4). So completeness is judged per 8-section segment: once any
    /// section from a segment has arrived, its `segment_last_section_number`
    /// tells us exactly which sections in that segment to expect, rather than
    /// assuming all 8.
    fn complete(&self) -> bool {
        let last_segment = self.meta.last_section_number / EIT_SEGMENT_SIZE;
        for segment in 0..=last_segment {
            let Some(&segment_last_raw) = self.segment_ends.get(&segment) else {
                // No section from this segment has arrived yet, so we don't
                // even know how many sections to expect from it.
                return false;
            };
            let segment_start = segment * EIT_SEGMENT_SIZE;
            let segment_last = segment_start
                .saturating_add(segment_last_raw % EIT_SEGMENT_SIZE)
                .min(self.meta.last_section_number);
            for section_number in segment_start..=segment_last {
                if self.slots[section_number as usize].is_none() {
                    return false;
                }
            }
        }
        true
    }

    fn to_complete(&self) -> Option<CompleteSectionSet> {
        if self.emitted {
            return None;
        }
        self.snapshot()
    }

    /// Build a [`CompleteSectionSet`] from this section set's slots if
    /// complete, regardless of whether it has already been emitted once.
    ///
    /// Used to re-feed an already-completed sub-table into a
    /// [`PartialEitSchedule`] whose range just reset, without losing it.
    fn snapshot(&self) -> Option<CompleteSectionSet> {
        if !self.complete() {
            return None;
        }

        // Only the sections that actually exist on the wire are collected —
        // a sparse segment legitimately never transmits some of the slots in
        // `0..=last_section_number`, so holes here are expected, not a bug.
        let sections = self.slots.iter().filter_map(Clone::clone).collect();
        Some(CompleteSectionSet {
            meta: SectionSetMeta {
                key: SectionSetKey {
                    pid: self.meta.key.logical_key.pid,
                    table_id: self.meta.key.table_id,
                    extension_id: self.meta.key.logical_key.service_id,
                    current_next_indicator: self.meta.key.logical_key.current_next_indicator,
                },
                version_number: self.meta.version_number,
                last_section_number: self.meta.last_section_number,
            },
            sections,
        })
    }
}

#[derive(Debug)]
struct PartialEitSchedule {
    meta: EitScheduleMeta,
    table_sets: BTreeMap<u8, CompleteSectionSet>,
    emitted: bool,
    /// The collector's `schedules_tick` at the last touch of this key; `0`
    /// is a sentinel meaning "never touched yet".
    last_touch: u64,
}

impl PartialEitSchedule {
    fn new(meta: EitScheduleMeta) -> Self {
        Self {
            meta,
            table_sets: BTreeMap::new(),
            emitted: false,
            last_touch: 0,
        }
    }

    fn reset(&mut self, meta: EitScheduleMeta) {
        let last_touch = self.last_touch;
        *self = Self::new(meta);
        self.last_touch = last_touch;
    }

    fn insert(&mut self, table_id: u8, set: CompleteSectionSet) {
        self.table_sets.insert(table_id, set);
        self.emitted = false;
    }

    fn complete(&self) -> bool {
        (self.meta.first_table_id..=self.meta.last_table_id)
            .all(|table_id| self.table_sets.contains_key(&table_id))
    }

    fn to_complete(&self) -> Option<CompleteEitSchedule> {
        if !self.complete() || self.emitted {
            return None;
        }
        let table_sets = (self.meta.first_table_id..=self.meta.last_table_id)
            .map(|table_id| {
                self.table_sets
                    .get(&table_id)
                    .expect("complete EIT schedule has no missing table IDs")
                    .clone()
            })
            .collect();
        Some(CompleteEitSchedule {
            first_table_id: self.meta.first_table_id,
            last_table_id: self.meta.last_table_id,
            table_sets,
        })
    }
}

/// Completed EIT schedule spanning all schedule table IDs through
/// `last_table_id`.
#[derive(Debug, Clone)]
pub struct CompleteEitSchedule {
    first_table_id: u8,
    last_table_id: u8,
    table_sets: Vec<CompleteSectionSet>,
}

impl CompleteEitSchedule {
    /// First schedule table_id in this range.
    #[must_use]
    pub const fn first_table_id(&self) -> u8 {
        self.first_table_id
    }

    /// Last schedule table_id in this range.
    #[must_use]
    pub const fn last_table_id(&self) -> u8 {
        self.last_table_id
    }

    /// Completed section sets, one per schedule table_id in order.
    #[must_use]
    pub fn table_sets(&self) -> &[CompleteSectionSet] {
        &self.table_sets
    }

    /// Per-table_id 5-bit version numbers in schedule table_id order.
    ///
    /// DVB EIT schedule sub-tables version independently, so there is no single
    /// schedule-wide version number.
    pub fn table_versions(&self) -> impl ExactSizeIterator<Item = (u8, u8)> + '_ {
        self.table_sets
            .iter()
            .map(|set| (set.meta().key.table_id, set.meta().version_number))
    }

    /// Parse each completed schedule table-id set.
    pub fn tables(&self) -> crate::Result<Vec<CompleteEit<'_>>> {
        self.tables_with_registry(None)
    }

    /// Parse each completed schedule table-id set with an optional descriptor
    /// registry.
    pub fn tables_with_registry<'a>(
        &'a self,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Vec<CompleteEit<'a>>> {
        self.table_sets
            .iter()
            .map(|set| CompleteEit::parse(set, registry))
            .collect()
    }
}

/// Event entry in a complete EIT.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompleteEitEvent<'a> {
    /// 16-bit event_id.
    pub event_id: u16,
    /// 40-bit start time.
    pub start_time_raw: [u8; 5],
    /// 24-bit duration.
    pub duration_raw: [u8; 3],
    /// 3-bit running status (EN 300 468 Table 6).
    pub running_status: RunningStatus,
    /// free_CA_mode.
    pub free_ca_mode: bool,
    /// Typed descriptor loop for this event.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

impl CompleteEitEvent<'_> {
    /// Decode the 24-bit BCD `duration` (HHMMSS) to a [`core::time::Duration`].
    ///
    /// Returns `None` if the BCD nibbles are out of range.
    #[must_use]
    pub fn duration(&self) -> Option<core::time::Duration> {
        broadcast_common::time::decode_bcd_duration(self.duration_raw)
    }

    /// Decode `start_time_raw` (16-bit MJD + 24-bit BCD UTC) to a UTC datetime.
    ///
    /// Returns `None` if the date/time fields are out of range. MJD→calendar
    /// conversion per ETSI EN 300 468 Annex C.
    #[cfg(feature = "chrono")]
    #[must_use]
    pub fn start_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        broadcast_common::time::decode_mjd_bcd_utc(self.start_time_raw)
    }
}

/// Complete EIT for one exact table_id/extension section sequence.
///
/// EIT schedule collection across `last_table_id` is intentionally represented
/// as multiple complete section sets: one per schedule table_id. That preserves
/// the DVB schedule sub-table structure while still exposing flattened events.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompleteEit<'a> {
    /// Variant based on table_id.
    pub kind: eit::EitKind,
    /// Raw table_id byte.
    pub table_id: u8,
    /// service_id.
    pub service_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// transport_stream_id.
    pub transport_stream_id: u16,
    /// original_network_id.
    pub original_network_id: u16,
    /// segment_last_section_number from section 0.
    pub segment_last_section_number: u8,
    /// last_table_id.
    pub last_table_id: u8,
    /// Events from all sections in wire order.
    pub events: Vec<CompleteEitEvent<'a>>,
}

impl<'a> CompleteEit<'a> {
    pub(crate) fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<eit::EitSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteEit sections",
        })?;
        let mut events = Vec::new();
        for section in &sections {
            events.extend(section.events.iter().map(|event| CompleteEitEvent {
                event_id: event.event_id,
                start_time_raw: event.start_time_raw,
                duration_raw: event.duration_raw,
                running_status: event.running_status,
                free_ca_mode: event.free_ca_mode,
                descriptors: ParsedDescriptorLoop::parse(event.descriptors, registry),
            }));
        }
        Ok(Self {
            kind: first.kind,
            table_id: first.table_id,
            service_id: first.service_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            transport_stream_id: first.transport_stream_id,
            original_network_id: first.original_network_id,
            segment_last_section_number: first.segment_last_section_number,
            last_table_id: first.last_table_id,
            events,
        })
    }
}
