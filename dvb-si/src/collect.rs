//! Multi-section table collection.
//!
//! Section parsers in [`crate::tables`] describe one wire section. This module
//! adds the next layer up: collect all sections in `0..=last_section_number`
//! for one logical version, then expose a complete table view.
//!
//! Collectors validate long-form section CRCs before retaining bytes. If the
//! input already came from [`crate::demux::SiDemux`], that validation has
//! already happened; direct section-byte callers get the same guard here.
//!
//! A collector error describes the section that was just pushed, not the whole
//! stream. Long-running consumers should normally log/drop that section and
//! continue feeding later sections; previous valid collector state is retained.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::descriptors::{AnyDescriptor, DescriptorLoop, DescriptorRegistry};
use crate::section::Section;
use crate::tables::{bat, eit, nit, sdt};
use dvb_common::Parse;

/// Result alias for collection operations.
pub type CollectResult<T> = core::result::Result<T, CollectError>;

/// Errors returned by multi-section collectors.
///
/// These errors are scoped to the current input section. They usually mean
/// "skip this section and keep going", especially on live streams where a
/// broadcaster may mutate section bytes without bumping `version_number`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CollectError {
    /// The section bytes did not parse as a generic PSI/SI section.
    #[error("section parse failed: {0}")]
    Section(#[from] crate::Error),

    /// A short-form section was fed to a multi-section collector.
    #[error(
        "table_id {table_id:#04x} is a short-form section and cannot be multi-section collected"
    )]
    ShortFormSection {
        /// Raw table_id byte.
        table_id: u8,
    },

    /// `section_number` was outside the advertised section range.
    #[error(
        "section_number {section_number} exceeds last_section_number {last_section_number} for table_id {table_id:#04x}"
    )]
    SectionNumberOutOfRange {
        /// Raw table_id byte.
        table_id: u8,
        /// Section number carried by the section.
        section_number: u8,
        /// Last section number carried by the section.
        last_section_number: u8,
    },

    /// A slot already contained different bytes for the same version.
    #[error("conflicting bytes for table_id {table_id:#04x} section {section_number}")]
    ConflictingSection {
        /// Raw table_id byte.
        table_id: u8,
        /// Section slot that conflicted.
        section_number: u8,
    },

    /// An EIT schedule section advertised an impossible table-id range.
    #[error(
        "EIT schedule table_id {table_id:#04x} is outside advertised range {first_table_id:#04x}..={last_table_id:#04x}"
    )]
    EitTableIdOutOfRange {
        /// Incoming EIT schedule table_id.
        table_id: u8,
        /// First table_id for this schedule kind.
        first_table_id: u8,
        /// Advertised last_table_id.
        last_table_id: u8,
    },
}

/// Logical key for one section sequence.
///
/// The key deliberately excludes `version_number` and `section_number`. Version
/// changes reset a collection; section numbers index into that collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectionSetKey {
    /// Optional PID context supplied by the caller.
    pub pid: Option<u16>,
    /// Raw `table_id`.
    pub table_id: u8,
    /// Long-form `table_id_extension`.
    pub extension_id: u16,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
}

/// Metadata shared by every section in a complete section set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionSetMeta {
    /// Logical section-set key.
    pub key: SectionSetKey,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// Last section number for this set.
    pub last_section_number: u8,
}

#[derive(Debug)]
struct PartialSectionSet {
    meta: SectionSetMeta,
    slots: Vec<Option<Arc<[u8]>>>,
    filled: usize,
    emitted: bool,
}

impl PartialSectionSet {
    fn new(meta: SectionSetMeta) -> Self {
        let len = meta.last_section_number as usize + 1;
        Self {
            meta,
            slots: vec![None; len],
            filled: 0,
            emitted: false,
        }
    }

    fn reset(&mut self, meta: SectionSetMeta) {
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
            .map(|slot| slot.as_ref().expect("complete set has no holes").clone())
            .collect();
        Some(CompleteSectionSet {
            meta: self.meta,
            sections,
        })
    }
}

/// Generic collector for long-form `section_number`/`last_section_number`
/// sequences.
#[derive(Debug, Default)]
pub struct SectionSetCollector {
    partial: HashMap<SectionSetKey, PartialSectionSet>,
}

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
            .map(|set| (set.meta.key.table_id, set.meta.version_number))
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

impl SectionSetCollector {
    /// Create an empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one complete section. Returns `Some` only when the logical section
    /// set has become complete for the first time at this version.
    ///
    /// # Errors
    ///
    /// Returns a [`CollectError`] if the bytes are not a valid long-form
    /// section or if the section set becomes internally inconsistent. Treat the
    /// error as applying to this section only unless your application wants
    /// strict stream-fail behavior.
    pub fn push_section(
        &mut self,
        bytes: impl AsRef<[u8]>,
    ) -> CollectResult<Option<CompleteSectionSet>> {
        self.push_section_with_pid(None, bytes)
    }

    /// Push one complete section with PID context.
    ///
    /// The PID is folded into the section-set key so tables with identical
    /// table id/extension on different PIDs do not collide.
    pub fn push_section_with_pid(
        &mut self,
        pid: Option<u16>,
        bytes: impl AsRef<[u8]>,
    ) -> CollectResult<Option<CompleteSectionSet>> {
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

        let key = SectionSetKey {
            pid,
            table_id: section.table_id,
            extension_id: section.extension_id,
            current_next_indicator: section.current_next_indicator,
        };
        let meta = SectionSetMeta {
            key,
            version_number: section.version_number,
            last_section_number: section.last_section_number,
        };
        let bytes: Arc<[u8]> = Arc::from(raw);

        let partial = self
            .partial
            .entry(key)
            .or_insert_with(|| PartialSectionSet::new(meta));

        if partial.meta.version_number != meta.version_number
            || partial.meta.last_section_number != meta.last_section_number
        {
            partial.reset(meta);
        }

        partial.insert(section.section_number, bytes)?;
        let complete = partial.to_complete();
        if complete.is_some() {
            partial.emitted = true;
        }
        Ok(complete)
    }

    /// Drop all retained partial section sets.
    pub fn clear(&mut self) {
        self.partial.clear();
    }

    /// Number of retained partial section-set states.
    #[must_use]
    pub fn len(&self) -> usize {
        self.partial.len()
    }

    /// Whether the collector currently has no retained state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.partial.is_empty()
    }
}

/// A complete owned set of original section bytes for one logical section
/// sequence.
#[derive(Debug, Clone)]
pub struct CompleteSectionSet {
    meta: SectionSetMeta,
    sections: Vec<Arc<[u8]>>,
}

/// Generic complete table view for one collected section set.
///
/// This is the all-table escape hatch: every long-form PSI/SI table with
/// `section_number`/`last_section_number` can be collected into a
/// [`CompleteSectionSet`] and parsed as `CompleteTable<T>`. Table-specific
/// complete views such as [`CompleteNit`] add flattened convenience fields where
/// the logical table shape is useful.
#[derive(Debug)]
pub struct CompleteTable<T> {
    meta: SectionSetMeta,
    sections: Vec<T>,
}

impl<T> CompleteTable<T> {
    /// Metadata shared by the section set.
    #[must_use]
    pub const fn meta(&self) -> SectionSetMeta {
        self.meta
    }

    /// Parsed sections in section-number order.
    #[must_use]
    pub fn sections(&self) -> &[T] {
        &self.sections
    }

    /// Consume the complete table and return the parsed sections.
    #[must_use]
    pub fn into_sections(self) -> Vec<T> {
        self.sections
    }
}

impl CompleteSectionSet {
    /// Metadata shared by the section set.
    #[must_use]
    pub const fn meta(&self) -> SectionSetMeta {
        self.meta
    }

    /// Complete section bytes in section-number order.
    #[must_use]
    pub fn section_bytes(&self) -> impl ExactSizeIterator<Item = &[u8]> {
        self.sections.iter().map(AsRef::as_ref)
    }

    /// Parse every section in this set as `T`.
    ///
    /// The parsed values borrow from this [`CompleteSectionSet`], so callers can
    /// retain the set and use borrowed typed views without copying table loops.
    pub fn parse_sections<'a, T>(&'a self) -> crate::Result<Vec<T>>
    where
        T: Parse<'a, Error = crate::Error>,
    {
        self.section_bytes().map(T::parse).collect()
    }

    /// Parse this set as a generic complete table.
    ///
    /// Use this for any long-form table that does not need a specialised
    /// flattened logical view.
    pub fn table<'a, T>(&'a self) -> crate::Result<CompleteTable<T>>
    where
        T: Parse<'a, Error = crate::Error>,
    {
        Ok(CompleteTable {
            meta: self.meta,
            sections: self.parse_sections()?,
        })
    }

    /// Build a complete NIT view from this section set.
    pub fn nit(&self) -> crate::Result<CompleteNit<'_>> {
        CompleteNit::parse(self, None)
    }

    /// Build a complete NIT view using a descriptor registry.
    pub fn nit_with_registry<'a>(
        &'a self,
        registry: &'a DescriptorRegistry,
    ) -> crate::Result<CompleteNit<'a>> {
        CompleteNit::parse(self, Some(registry))
    }

    /// Build a complete BAT view from this section set.
    pub fn bat(&self) -> crate::Result<CompleteBat<'_>> {
        CompleteBat::parse(self, None)
    }

    /// Build a complete BAT view using a descriptor registry.
    pub fn bat_with_registry<'a>(
        &'a self,
        registry: &'a DescriptorRegistry,
    ) -> crate::Result<CompleteBat<'a>> {
        CompleteBat::parse(self, Some(registry))
    }

    /// Build a complete SDT view from this section set.
    pub fn sdt(&self) -> crate::Result<CompleteSdt<'_>> {
        CompleteSdt::parse(self, None)
    }

    /// Build a complete SDT view using a descriptor registry.
    pub fn sdt_with_registry<'a>(
        &'a self,
        registry: &'a DescriptorRegistry,
    ) -> crate::Result<CompleteSdt<'a>> {
        CompleteSdt::parse(self, Some(registry))
    }

    /// Build a complete EIT view from this section set.
    pub fn eit(&self) -> crate::Result<CompleteEit<'_>> {
        CompleteEit::parse(self, None)
    }

    /// Build a complete EIT view using a descriptor registry.
    pub fn eit_with_registry<'a>(
        &'a self,
        registry: &'a DescriptorRegistry,
    ) -> crate::Result<CompleteEit<'a>> {
        CompleteEit::parse(self, Some(registry))
    }
}

/// Parsed descriptor loop retaining the raw bytes and the typed descriptor
/// results.
#[derive(Debug)]
pub struct ParsedDescriptorLoop<'a> {
    raw: DescriptorLoop<'a>,
    descriptors: Vec<crate::Result<AnyDescriptor<'a>>>,
}

impl<'a> ParsedDescriptorLoop<'a> {
    fn parse(raw: DescriptorLoop<'a>, registry: Option<&'a DescriptorRegistry>) -> Self {
        let descriptors = match registry {
            Some(registry) => registry.parse_loop(raw.raw()).collect(),
            None => raw.iter().collect(),
        };
        Self { raw, descriptors }
    }

    /// Raw descriptor-loop bytes.
    #[must_use]
    pub const fn raw(&self) -> DescriptorLoop<'a> {
        self.raw
    }

    /// Typed descriptor parse results in wire order.
    pub fn descriptors(&self) -> &[crate::Result<AnyDescriptor<'a>>] {
        &self.descriptors
    }
}

/// Transport-stream entry in a complete NIT.
#[derive(Debug)]
pub struct CompleteNitTransportStream<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Typed descriptor loop for this transport stream.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

/// Complete logical Network Information Table.
#[derive(Debug)]
pub struct CompleteNit<'a> {
    /// Variant discriminator.
    pub kind: nit::NitKind,
    /// Network identifier.
    pub network_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// Network-wide descriptors from section 0.
    pub network_descriptors: ParsedDescriptorLoop<'a>,
    /// Transport-stream loop entries from all sections in wire order.
    pub transport_streams: Vec<CompleteNitTransportStream<'a>>,
}

impl<'a> CompleteNit<'a> {
    fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<nit::NitSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteNit sections",
        })?;
        let mut transport_streams = Vec::new();
        for section in &sections {
            transport_streams.extend(section.transport_streams.iter().map(|ts| {
                CompleteNitTransportStream {
                    transport_stream_id: ts.transport_stream_id,
                    original_network_id: ts.original_network_id,
                    descriptors: ParsedDescriptorLoop::parse(ts.descriptors, registry),
                }
            }));
        }
        Ok(Self {
            kind: first.kind,
            network_id: first.network_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            // The network descriptor loop is carried in section 0; completed
            // sets are stored in section-number order, so `first` is
            // authoritative for table-wide descriptors.
            network_descriptors: ParsedDescriptorLoop::parse(first.network_descriptors, registry),
            transport_streams,
        })
    }
}

/// Transport-stream entry in a complete BAT.
#[derive(Debug)]
pub struct CompleteBatTransportStream<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Typed descriptor loop for this transport stream.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

/// Complete logical Bouquet Association Table.
#[derive(Debug)]
pub struct CompleteBat<'a> {
    /// Bouquet identifier.
    pub bouquet_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// Bouquet descriptors from section 0.
    pub bouquet_descriptors: ParsedDescriptorLoop<'a>,
    /// Transport-stream loop entries from all sections in wire order.
    pub transport_streams: Vec<CompleteBatTransportStream<'a>>,
}

impl<'a> CompleteBat<'a> {
    fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<bat::BatSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteBat sections",
        })?;
        let mut transport_streams = Vec::new();
        for section in &sections {
            transport_streams.extend(section.transport_streams.iter().map(|ts| {
                CompleteBatTransportStream {
                    transport_stream_id: ts.transport_stream_id,
                    original_network_id: ts.original_network_id,
                    descriptors: ParsedDescriptorLoop::parse(ts.descriptors, registry),
                }
            }));
        }
        Ok(Self {
            bouquet_id: first.bouquet_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            // The bouquet descriptor loop is carried in section 0; completed
            // sets are stored in section-number order, so `first` is
            // authoritative for table-wide descriptors.
            bouquet_descriptors: ParsedDescriptorLoop::parse(first.bouquet_descriptors, registry),
            transport_streams,
        })
    }
}

/// Service entry in a complete SDT.
#[derive(Debug)]
pub struct CompleteSdtService<'a> {
    /// service_id.
    pub service_id: u16,
    /// EIT schedule flag.
    pub eit_schedule_flag: bool,
    /// EIT present/following flag.
    pub eit_present_following_flag: bool,
    /// 3-bit running status.
    pub running_status: u8,
    /// free_CA_mode.
    pub free_ca_mode: bool,
    /// Typed descriptor loop for this service.
    pub descriptors: ParsedDescriptorLoop<'a>,
}

/// Complete logical Service Description Table.
#[derive(Debug)]
pub struct CompleteSdt<'a> {
    /// Variant discriminator.
    pub kind: sdt::SdtKind,
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Services from all sections in wire order.
    pub services: Vec<CompleteSdtService<'a>>,
}

impl<'a> CompleteSdt<'a> {
    fn parse(
        set: &'a CompleteSectionSet,
        registry: Option<&'a DescriptorRegistry>,
    ) -> crate::Result<Self> {
        let sections: Vec<sdt::SdtSection<'a>> = set.parse_sections()?;
        let first = sections.first().ok_or(crate::Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "CompleteSdt sections",
        })?;
        let mut services = Vec::new();
        for section in &sections {
            services.extend(section.services.iter().map(|svc| CompleteSdtService {
                service_id: svc.service_id,
                eit_schedule_flag: svc.eit_schedule_flag,
                eit_present_following_flag: svc.eit_present_following_flag,
                running_status: svc.running_status,
                free_ca_mode: svc.free_ca_mode,
                descriptors: ParsedDescriptorLoop::parse(svc.descriptors, registry),
            }));
        }
        Ok(Self {
            kind: first.kind,
            transport_stream_id: first.transport_stream_id,
            version_number: first.version_number,
            current_next_indicator: first.current_next_indicator,
            original_network_id: first.original_network_id,
            services,
        })
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
    fn parse(
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
