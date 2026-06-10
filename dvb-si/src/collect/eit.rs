use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::descriptors::DescriptorRegistry;
use crate::section::Section;
use crate::tables::eit;
use dvb_common::Parse;

use super::{
    CollectError, CollectResult, CompleteSectionSet, ParsedDescriptorLoop, SectionSetKey,
    SectionSetMeta,
};

/// Default cap on the number of in-progress logical keys (section sets +
/// schedule ranges) retained by [`EitCollector`].
///
/// 256 concurrent collections is generous — a real DVB network has at most a
/// few dozen services per transponder — while bounding a hostile stream that
/// rotates `original_network_id` / `transport_stream_id` / `service_id` (or
/// `current_next_indicator`) to force unbounded map growth. The cap is applied
/// independently to the sections map and the schedules map; each is limited to
/// `max_logical_keys` entries. When a map is full, incoming sections for new
/// keys are skipped until [`clear`](EitCollector::clear) or
/// [`retain_logical`](EitCollector::retain_logical) frees capacity.
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
/// or schedules map is full, incoming sections for new keys are skipped until
/// space frees — the same skip-until-space policy as
/// [`crate::carousel::ModuleReassembler`].
#[derive(Debug)]
pub struct EitCollector {
    sections: HashMap<EitSectionSetKey, PartialEitSectionSet>,
    schedules: HashMap<EitLogicalKey, PartialEitSchedule>,
    max_logical_keys: usize,
}

impl Default for EitCollector {
    fn default() -> Self {
        Self {
            sections: HashMap::new(),
            schedules: HashMap::new(),
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
    /// Sections for new keys are skipped when the relevant map is full, until
    /// [`clear`](Self::clear) or [`retain_logical`](Self::retain_logical)
    /// frees capacity.
    #[must_use]
    pub fn with_max_logical_keys(mut self, max_logical_keys: usize) -> Self {
        self.max_logical_keys = max_logical_keys;
        self
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

        // Cap check: sections map
        if !self.sections.contains_key(&key) && self.sections.len() >= self.max_logical_keys {
            return Ok(None);
        }

        let partial = self
            .sections
            .entry(key)
            .or_insert_with(|| PartialEitSectionSet::new(meta));
        if partial.meta.version_number != meta.version_number
            || partial.meta.last_section_number != meta.last_section_number
        {
            partial.reset(meta);
        }

        partial.insert(eit.section_number, bytes)?;
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

                // Cap check: schedules map (before marking the section set emitted)
                if !self.schedules.contains_key(&logical_key)
                    && self.schedules.len() >= self.max_logical_keys
                {
                    return Ok(None);
                }

                partial.emitted = true;

                let schedule_meta = EitScheduleMeta {
                    key: logical_key,
                    first_table_id,
                    last_table_id: eit.last_table_id,
                };
                let schedule = self
                    .schedules
                    .entry(logical_key)
                    .or_insert_with(|| PartialEitSchedule::new(schedule_meta));
                if schedule.meta.last_table_id != schedule_meta.last_table_id {
                    schedule.reset(schedule_meta);
                }
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
        self.schedules.clear();
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
        self.schedules.retain(|key, _| keep(key));
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug)]
struct PartialEitSectionSet {
    meta: EitSectionSetMeta,
    slots: Vec<Option<Arc<[u8]>>>,
    filled: usize,
    emitted: bool,
}

impl PartialEitSectionSet {
    fn new(meta: EitSectionSetMeta) -> Self {
        let len = meta.last_section_number as usize + 1;
        Self {
            meta,
            slots: vec![None; len],
            filled: 0,
            emitted: false,
        }
    }

    fn reset(&mut self, meta: EitSectionSetMeta) {
        *self = Self::new(meta);
    }

    fn insert(&mut self, section_number: u8, bytes: Arc<[u8]>) -> CollectResult<bool> {
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
        Ok(true)
    }

    fn complete(&self) -> bool {
        self.filled == self.slots.len()
    }

    fn to_complete(&self) -> Option<CompleteSectionSet> {
        if !self.complete() || self.emitted {
            return None;
        }

        let sections = self
            .slots
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .expect("complete EIT set has no holes")
                    .clone()
            })
            .collect();
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
}

impl PartialEitSchedule {
    fn new(meta: EitScheduleMeta) -> Self {
        Self {
            meta,
            table_sets: BTreeMap::new(),
            emitted: false,
        }
    }

    fn reset(&mut self, meta: EitScheduleMeta) {
        *self = Self::new(meta);
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
pub struct CompleteEitEvent<'a> {
    /// 16-bit event_id.
    pub event_id: u16,
    /// 40-bit start time.
    pub start_time_raw: [u8; 5],
    /// 24-bit duration.
    pub duration_raw: [u8; 3],
    /// 3-bit running status.
    pub running_status: u8,
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
        dvb_common::time::decode_bcd_duration(self.duration_raw)
    }

    /// Decode `start_time_raw` (16-bit MJD + 24-bit BCD UTC) to a UTC datetime.
    ///
    /// Returns `None` if the date/time fields are out of range. MJD→calendar
    /// conversion per ETSI EN 300 468 Annex C.
    #[cfg(feature = "chrono")]
    #[must_use]
    pub fn start_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        dvb_common::time::decode_mjd_bcd_utc(self.start_time_raw)
    }
}

/// Complete EIT for one exact table_id/extension section sequence.
///
/// EIT schedule collection across `last_table_id` is intentionally represented
/// as multiple complete section sets: one per schedule table_id. That preserves
/// the DVB schedule sub-table structure while still exposing flattened events.
#[derive(Debug)]
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
