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

use std::collections::HashMap;
use std::sync::Arc;

use crate::descriptors::{AnyDescriptor, DescriptorLoop, DescriptorRegistry};
use crate::section::Section;
use dvb_common::Parse;

mod bat;
mod eit;
mod nit;
mod sdt;

pub use bat::*;
pub use eit::*;
pub use nit::*;
pub use sdt::*;

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
    pub(crate) fn parse(raw: DescriptorLoop<'a>, registry: Option<&'a DescriptorRegistry>) -> Self {
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
