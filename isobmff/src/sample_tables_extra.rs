//! Sample-table boxes — ISO/IEC 14496-12:2026 §8.5.3, §8.7.6, §8.13.6.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`DegradationPriorityBox`] | `stdp` | §8.5.3 | `stbl` |
//! | [`PaddingBitsBox`] | `padb` | §8.7.6 | `stbl` |
//! | [`SubTrackSampleGroupBox`] | `stsg` | §8.13.6 | `strd` |
//!
//! The compact sample-to-group box (`csgp`, §8.9.5) lives with the other
//! sample-group syntax in [`crate::sample_groups`].

use alloc::vec::Vec;

use broadcast_core::len::{fit_u16, fit_u32, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
    box_len, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the Degradation Priority Box (`stdp`, ISO/IEC 14496-12:2026 §8.5.3).
pub const STDP: [u8; BOX_TYPE_SIZE] = *b"stdp";
/// Four-CC of the Padding Bits Box (`padb`, §8.7.6).
pub const PADB: [u8; BOX_TYPE_SIZE] = *b"padb";
/// Four-CC of the Sub Track Sample Group Box (`stsg`, §8.13.6).
pub const STSG: [u8; BOX_TYPE_SIZE] = *b"stsg";

/// Width of one `stdp` priority entry.
const STDP_ENTRY_LEN: usize = 2;
/// Number of 3-bit pad fields packed into one `padb` byte.
const PADB_PER_BYTE: usize = 2;
/// Bits of one `padb` pad field.
const PADB_PAD_BITS: u8 = 3;
/// Mask of one 3-bit `padb` pad field.
const PADB_PAD_MASK: u8 = (1 << PADB_PAD_BITS) - 1;
/// Shift of the first pad field in a `padb` byte.
const PADB_FIRST_SHIFT: u32 = 4;

// ---------------------------------------------------------------------------
// DegradationPriorityBox — stdp (§8.5.3)
// ---------------------------------------------------------------------------

/// Degradation Priority Box (`stdp`) — ISO/IEC 14496-12:2026 §8.5.3.
///
/// `priority[i]` is the degradation priority of the `i`-th sample; the sample
/// count comes from the track's `stsz`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DegradationPriorityBox {
    version: u8,
    flags: u32,
    priorities: Vec<u16>,
}

impl DegradationPriorityBox {
    /// Build one from its priorities.
    #[must_use]
    pub fn new(priorities: Vec<u16>) -> Self {
        Self {
            version: 0,
            flags: 0,
            priorities,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The per-sample priorities.
    #[must_use]
    pub fn priorities(&self) -> &[u16] {
        &self.priorities
    }

    /// The priority of the `index`-th sample, when present.
    #[must_use]
    pub fn priority(&self, index: usize) -> Option<u16> {
        self.priorities.get(index).copied()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "stdp body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let rest = &body[FULLBOX_EXTRA_SIZE..];
        let mut priorities = Vec::with_capacity(rest.len() / STDP_ENTRY_LEN);
        for chunk in rest.chunks_exact(STDP_ENTRY_LEN) {
            priorities.push(be_u16(chunk, 0, "stdp priority")?);
        }
        Ok(Self {
            version,
            flags,
            priorities,
        })
    }
}

impl<'a> Parse<'a> for DegradationPriorityBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "stdp box")? != u32::from_be_bytes(STDP) {
            return Err(Error::InvalidInput("expected stdp"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "stdp box",
        )?)
    }
}

impl Serialize for DegradationPriorityBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.priorities.len() * STDP_ENTRY_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &STDP, need, self.version, self.flags, false)?;
        for priority in &self.priorities {
            buf[c..c + STDP_ENTRY_LEN].copy_from_slice(&priority.to_be_bytes());
            c += STDP_ENTRY_LEN;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// PaddingBitsBox — padb (§8.7.6)
// ---------------------------------------------------------------------------

/// Padding Bits Box (`padb`) — ISO/IEC 14496-12:2026 §8.7.6.
///
/// Each sample carries the number of padding bits at the end of its last byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PaddingBitsBox {
    version: u8,
    flags: u32,
    padding_bits: Vec<u8>,
}

impl PaddingBitsBox {
    /// Build one from the per-sample padding-bit counts.
    #[must_use]
    pub fn new(padding_bits: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            padding_bits,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The number of samples this table describes.
    #[must_use]
    pub fn sample_count(&self) -> u32 {
        self.padding_bits.len() as u32
    }

    /// The per-sample padding-bit counts, in sample order.
    #[must_use]
    pub fn padding_bits(&self) -> &[u8] {
        &self.padding_bits
    }

    /// The padding-bit count of the `index`-th sample, when present.
    #[must_use]
    pub fn padding_for(&self, index: usize) -> Option<u8> {
        self.padding_bits.get(index).copied()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 4, "padb body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let sample_count = be_u32(body, FULLBOX_EXTRA_SIZE, "padb sample_count")? as usize;
        let bytes_needed = sample_count.div_ceil(PADB_PER_BYTE);
        let start = FULLBOX_EXTRA_SIZE + 4;
        need_in(body, start + bytes_needed, "padb padding table")?;
        let mut padding_bits = Vec::with_capacity(sample_count);
        for &packed in &body[start..start + bytes_needed] {
            padding_bits.push((packed >> PADB_FIRST_SHIFT) & PADB_PAD_MASK);
            if padding_bits.len() < sample_count {
                padding_bits.push(packed & PADB_PAD_MASK);
            }
        }
        Ok(Self {
            version,
            flags,
            padding_bits,
        })
    }
}

impl<'a> Parse<'a> for PaddingBitsBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "padb box")? != u32::from_be_bytes(PADB) {
            return Err(Error::InvalidInput("expected padb"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + 4,
            "padb box",
        )?)
    }
}

impl Serialize for PaddingBitsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + 4
                + self.padding_bits.len().div_ceil(PADB_PER_BYTE),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &PADB, need, self.version, self.flags, false)?;
        let count = fit_u32(self.padding_bits.len(), "sample_count")?;
        buf[c..c + 4].copy_from_slice(&count.to_be_bytes());
        c += 4;
        for pair in self.padding_bits.chunks(PADB_PER_BYTE) {
            let first = pair[0] & PADB_PAD_MASK;
            let second = pair.get(1).copied().unwrap_or(0) & PADB_PAD_MASK;
            buf[c] = (first << PADB_FIRST_SHIFT) | second;
            c += 1;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SubTrackSampleGroupBox — stsg (§8.13.6)
// ---------------------------------------------------------------------------

/// Sub Track Sample Group Box (`stsg`) — ISO/IEC 14496-12:2026 §8.13.6.
///
/// Defines a sub-track as one or more sample groups of `grouping_type`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubTrackSampleGroupBox {
    version: u8,
    flags: u32,
    grouping_type: u32,
    group_description_indices: Vec<u32>,
}

impl SubTrackSampleGroupBox {
    /// Build one from its grouping type and description indices.
    #[must_use]
    pub fn new(grouping_type: u32, group_description_indices: Vec<u32>) -> Self {
        Self {
            version: 0,
            flags: 0,
            grouping_type,
            group_description_indices,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The grouping type.
    #[must_use]
    pub const fn grouping_type(&self) -> u32 {
        self.grouping_type
    }

    /// The referenced `sgpd` entry indices.
    #[must_use]
    pub fn group_description_indices(&self) -> &[u32] {
        &self.group_description_indices
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 6, "stsg body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let grouping_type = be_u32(body, FULLBOX_EXTRA_SIZE, "stsg grouping_type")?;
        let item_count = be_u16(body, FULLBOX_EXTRA_SIZE + 4, "stsg item_count")? as usize;
        let mut c = FULLBOX_EXTRA_SIZE + 6;
        crate::box_support::check_entry_count(
            body.len().saturating_sub(c),
            4,
            item_count,
            "stsg group_description_index",
        )?;
        let mut indices = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len().saturating_sub(c),
            4,
            item_count,
        ));
        for _ in 0..item_count {
            indices.push(be_u32(body, c, "stsg group_description_index")?);
            c += 4;
        }
        Ok(Self {
            version,
            flags,
            grouping_type,
            group_description_indices: indices,
        })
    }
}

impl<'a> Parse<'a> for SubTrackSampleGroupBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "stsg box")? != u32::from_be_bytes(STSG) {
            return Err(Error::InvalidInput("expected stsg"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + 6,
            "stsg box",
        )?)
    }
}

impl Serialize for SubTrackSampleGroupBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + 4
                + 2
                + self.group_description_indices.len() * 4,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &STSG, need, self.version, self.flags, false)?;
        buf[c..c + 4].copy_from_slice(&self.grouping_type.to_be_bytes());
        c += 4;
        let count = fit_u16(self.group_description_indices.len(), "item_count")?;
        buf[c..c + 2].copy_from_slice(&count.to_be_bytes());
        c += 2;
        for index in &self.group_description_indices {
            buf[c..c + 4].copy_from_slice(&index.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}
