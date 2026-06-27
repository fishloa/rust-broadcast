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

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::descriptors::{AnyDescriptor, DescriptorLoop, DescriptorRegistry};
use dvb_common::Parse;
use mpeg_ts::section::Section;

mod bat;
mod eit;
mod nit;
mod sdt;

pub use bat::*;
pub use eit::*;
pub use nit::*;
pub use sdt::*;

/// Default cap on the number of in-progress logical keys retained by
/// [`SectionSetCollector`].
///
/// 256 concurrent collections is generous while bounding a hostile stream that
/// rotates table_id / extension / current_next_indicator across PIDs to force
/// unbounded map growth. The cap is applied to the partial-sections map. When
/// the map is full, incoming sections for new keys are skipped until
/// [`clear`](SectionSetCollector::clear) frees capacity.
pub const DEFAULT_MAX_PARTIAL_KEYS: usize = 256;

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

impl From<mpeg_ts::Error> for CollectError {
    fn from(e: mpeg_ts::Error) -> Self {
        CollectError::Section(crate::Error::from(e))
    }
}

/// Logical key for one section sequence.
///
/// The key deliberately excludes `version_number` and `section_number`. Version
/// changes reset a collection; section numbers index into that collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
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
#[non_exhaustive]
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
///
/// The constructor [`SectionSetCollector::new`] uses the default cap
/// [`DEFAULT_MAX_PARTIAL_KEYS`]; the cap is configurable via
/// [`with_max_partial_keys`](Self::with_max_partial_keys).
#[derive(Debug)]
pub struct SectionSetCollector {
    partial: BTreeMap<SectionSetKey, PartialSectionSet>,
    max_partial_keys: usize,
}

impl Default for SectionSetCollector {
    fn default() -> Self {
        Self {
            partial: BTreeMap::new(),
            max_partial_keys: DEFAULT_MAX_PARTIAL_KEYS,
        }
    }
}

impl SectionSetCollector {
    /// Create an empty collector with the default cap
    /// ([`DEFAULT_MAX_PARTIAL_KEYS`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the partial-key cap (default [`DEFAULT_MAX_PARTIAL_KEYS`]).
    /// Sections for new keys are skipped when the map is full, until
    /// [`clear`](Self::clear) frees capacity.
    #[must_use]
    pub fn with_max_partial_keys(mut self, max_partial_keys: usize) -> Self {
        self.max_partial_keys = max_partial_keys;
        self
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

        // Cap check: skip new keys when the map is full
        if !self.partial.contains_key(&key) && self.partial.len() >= self.max_partial_keys {
            return Ok(None);
        }

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
    pub(crate) fn parse(raw: DescriptorLoop<'a>, registry: Option<&'a DescriptorRegistry>) -> Self {
        let descriptors = match registry {
            Some(registry) => registry.parse_loop(raw.raw()).collect(),
            None => raw.iter().collect(),
        };
        Self { raw, descriptors }
    }

    /// Raw descriptor-loop bytes.
    ///
    /// Use `raw().iter_with_extensions(&desc_reg, &ext_reg)` to recover custom
    /// extension bodies from a `Complete*` view.
    #[must_use]
    pub const fn raw(&self) -> DescriptorLoop<'a> {
        self.raw
    }

    /// Typed descriptor parse results in wire order.
    pub fn descriptors(&self) -> &[crate::Result<AnyDescriptor<'a>>] {
        &self.descriptors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TABLE_ID: u8 = 0x42;

    fn min_section(extension_id: u16) -> Vec<u8> {
        let section_length: u16 = 9; // 5 (ext_header) + 0 (payload) + 4 (crc)
        let mut buf = vec![0u8; 12];
        buf[0] = TEST_TABLE_ID;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&extension_id.to_be_bytes());
        buf[5] = 0xC1;
        buf[6] = 0;
        buf[7] = 0;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..8]);
        buf[8..12].copy_from_slice(&crc.to_be_bytes());
        buf
    }

    #[test]
    fn collect_single_section_is_complete() {
        let mut c = SectionSetCollector::new();
        let sec = min_section(0);
        let result = c.push_section(&sec).unwrap();
        assert!(result.is_some());
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn partial_keys_cap_skips_new_keys() {
        let mut c = SectionSetCollector::new().with_max_partial_keys(3);

        // Push sections for 3 distinct extension IDs — fills the cap.
        for eid in 0..3u16 {
            let sec = min_section(eid);
            let result = c.push_section(&sec).unwrap();
            assert!(
                result.is_some(),
                "single-section set for eid {eid} completes"
            );
        }
        assert_eq!(c.len(), 3);

        // Push a 4th distinct key — should be skipped (cap full).
        let sec4 = min_section(3);
        let result = c.push_section(&sec4).unwrap();
        assert!(result.is_none(), "new key beyond cap must be skipped");
        assert_eq!(c.len(), 3);

        // Clear frees space — 4th key can now enter.
        c.clear();
        assert!(c.is_empty());
        let result = c.push_section(&sec4).unwrap();
        assert!(result.is_some());
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn partial_keys_cap_does_not_skip_existing_key() {
        let mut c = SectionSetCollector::new().with_max_partial_keys(1);

        // Fill the cap with one multi-section NIT-like extension (section 0 of 1).
        let sec0 = {
            let mut buf = min_section(0xAB);
            // Make section 0 of 1 be incomplete: change last_section_number to 1
            buf[7] = 1;
            // Recompute CRC
            let crc = dvb_common::crc32_mpeg2::compute(&buf[..8]);
            buf[8..12].copy_from_slice(&crc.to_be_bytes());
            buf
        };
        let result = c.push_section(&sec0).unwrap();
        assert!(result.is_none(), "incomplete section set yields None");

        // Push section 1 of 1 for the same key — cap is full but key already
        // exists, so it must NOT be skipped.
        let mut sec1 = min_section(0xAB);
        sec1[6] = 1; // section_number = 1
        sec1[7] = 1; // last_section_number = 1
        let crc = dvb_common::crc32_mpeg2::compute(&sec1[..8]);
        sec1[8..12].copy_from_slice(&crc.to_be_bytes());

        let result = c.push_section(&sec1).unwrap();
        assert!(
            result.is_some(),
            "existing key must NOT be skipped when cap full"
        );
        assert_eq!(c.len(), 1);
    }
}
