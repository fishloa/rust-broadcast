//! Image-sequence track boxes — ISO/IEC 23008-12:2025 clause 7.
//!
//! An image sequence is an ISO base media track whose handler type is `pict`
//! (7.1, 7.2.2). The clause adds [`CodingConstraintsBox`] (`ccst`, 7.2.3) and
//! [`AuxiliaryTypeInfoBox`] (`auxi`, 7.5.3) to the sample entry and the
//! [`DirectReferenceSamplesList`] (`refs`) sample group (7.4.1). The track
//! itself is read by [`crate::reader`], which reports the `pict` handler; no
//! second track parser lives here.
//!
//! The track-header `matrix` constraints of 7.2.1 and the hidden-sample rule of
//! 7.3 are stated in prose over fields ISO/IEC 14496-12:2026 already types; they are
//! not a new structure and are reported as OPEN in the coverage table at
//! `private/specs-verified/mpeg/heif/isobmff-coverage-2025.md`.
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md` clause 7.

use alloc::vec::Vec;

use broadcast_core::len::{fit_u8, need_in, need_out};
use broadcast_core::wire::{be_u32, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, box_len, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the Coding Constraints box (7.2.3.2).
pub const CCST: [u8; BOX_TYPE_SIZE] = *b"ccst";
/// Four-CC of the Auxiliary Type Info box (7.5.3.2).
pub const AUXI: [u8; BOX_TYPE_SIZE] = *b"auxi";
/// Four-CC of the direct reference samples list sample group (7.4.1).
pub const REFS: [u8; BOX_TYPE_SIZE] = *b"refs";
/// `max_ref_per_pic` value meaning any number the sample entry permits
/// (7.2.3.3).
pub const MAX_REF_PER_PIC_UNRESTRICTED: u8 = 15;
/// Offset of `max_ref_per_pic`'s low bit in the 32-bit word (7.2.3.3).
const MAX_REF_PER_PIC_SHIFT: u32 = 26;
/// Mask of `max_ref_per_pic` in the 32-bit word.
const MAX_REF_PER_PIC_MASK: u32 = 0x0f;
/// Mask of the `reserved` run that follows `max_ref_per_pic` (7.2.3.3).
const CCST_RESERVED_MASK: u32 = (1 << MAX_REF_PER_PIC_SHIFT) - 1;
/// Offset of `intra_pred_used` in the 32-bit word.
const INTRA_PRED_USED_SHIFT: u32 = 30;
/// Offset of `all_ref_pics_intra` in the 32-bit word.
const ALL_REF_PICS_INTRA_SHIFT: u32 = 31;
/// Body length of the fixed `ccst` word (7.2.3.3).
const CCST_BODY_LEN: usize = core::mem::size_of::<u32>();
/// Width of `sample_id` / `direct_reference_sample_id` (7.4.1.2).
const REFS_ID_LEN: usize = core::mem::size_of::<u32>();
/// Width of `num_direct_reference_samples` (7.4.1.2).
const REFS_COUNT_LEN: usize = core::mem::size_of::<u8>();

/// Coding Constraints box (`ccst`) — ISO/IEC 23008-12:2025 7.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CodingConstraintsBox {
    version: u8,
    flags: u32,
    all_ref_pics_intra: bool,
    intra_pred_used: bool,
    max_ref_per_pic: u8,
    reserved: u32,
}

impl CodingConstraintsBox {
    /// Constraints for an image sequence.
    #[must_use]
    pub const fn new(all_ref_pics_intra: bool, intra_pred_used: bool, max_ref_per_pic: u8) -> Self {
        Self {
            version: 0,
            flags: 0,
            all_ref_pics_intra,
            intra_pred_used,
            max_ref_per_pic,
            reserved: 0,
        }
    }

    /// `all_ref_pics_intra`.
    #[must_use]
    pub const fn all_ref_pics_intra(&self) -> bool {
        self.all_ref_pics_intra
    }

    /// `intra_pred_used`.
    #[must_use]
    pub const fn intra_pred_used(&self) -> bool {
        self.intra_pred_used
    }

    /// `max_ref_per_pic`.
    #[must_use]
    pub const fn max_ref_per_pic(&self) -> u8 {
        self.max_ref_per_pic
    }

    /// The `reserved` run after `max_ref_per_pic`, kept verbatim.
    #[must_use]
    pub const fn reserved(&self) -> u32 {
        self.reserved
    }
}

impl<'a> Parse<'a> for CodingConstraintsBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "ccst")?;
        if u32_at(bytes, crate::box_types::BOX_TYPE_OFFSET) != u32::from_be_bytes(CCST) {
            return Err(Error::InvalidInput("expected ccst"));
        }
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE + CCST_BODY_LEN, "ccst")?;
        if body.len() != FULLBOX_EXTRA_SIZE + CCST_BODY_LEN {
            return Err(Error::InvalidInput("ccst trailing bytes"));
        }
        let word = be_u32(body, FULLBOX_EXTRA_SIZE, "ccst constraints")?;
        Ok(Self {
            version: body[0],
            flags: crate::image::full_flags(body),
            all_ref_pics_intra: (word >> ALL_REF_PICS_INTRA_SHIFT) & 1 != 0,
            intra_pred_used: (word >> INTRA_PRED_USED_SHIFT) & 1 != 0,
            max_ref_per_pic: ((word >> MAX_REF_PER_PIC_SHIFT) & MAX_REF_PER_PIC_MASK) as u8,
            reserved: word & CCST_RESERVED_MASK,
        })
    }
}

impl Serialize for CodingConstraintsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + CCST_BODY_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &CCST, need, self.version, self.flags, false)?;
        let mut word = u32::from(self.all_ref_pics_intra) << ALL_REF_PICS_INTRA_SHIFT;
        word |= u32::from(self.intra_pred_used) << INTRA_PRED_USED_SHIFT;
        word |=
            u32::from(self.max_ref_per_pic & (MAX_REF_PER_PIC_MASK as u8)) << MAX_REF_PER_PIC_SHIFT;
        word |= broadcast_core::len::fit_mask(self.reserved, CCST_RESERVED_MASK, "ccst reserved")?;
        buf[cursor..cursor + CCST_BODY_LEN].copy_from_slice(&word.to_be_bytes());
        cursor += CCST_BODY_LEN;
        Ok(cursor)
    }
}

/// Auxiliary Type Info box (`auxi`) — ISO/IEC 23008-12:2025 7.5.3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AuxiliaryTypeInfoBox {
    version: u8,
    flags: u32,
    aux_track_type: Vec<u8>,
}

impl AuxiliaryTypeInfoBox {
    /// An auxiliary type URI.
    #[must_use]
    pub fn new(aux_track_type: &[u8]) -> Self {
        Self {
            version: 0,
            flags: 0,
            aux_track_type: aux_track_type.to_vec(),
        }
    }

    /// `aux_track_type`.
    #[must_use]
    pub fn aux_track_type(&self) -> &[u8] {
        &self.aux_track_type
    }
}

impl<'a> Parse<'a> for AuxiliaryTypeInfoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "auxi")?;
        if u32_at(bytes, crate::box_types::BOX_TYPE_OFFSET) != u32::from_be_bytes(AUXI) {
            return Err(Error::InvalidInput("expected auxi"));
        }
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE + 1, "auxi")?;
        let rest = &body[FULLBOX_EXTRA_SIZE..];
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(Error::InvalidInput("auxi aux_track_type"))?;
        if end + 1 != rest.len() {
            return Err(Error::InvalidInput("auxi trailing bytes"));
        }
        Ok(Self {
            version: body[0],
            flags: crate::image::full_flags(body),
            aux_track_type: rest[..end].to_vec(),
        })
    }
}

impl Serialize for AuxiliaryTypeInfoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.aux_track_type.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &AUXI, need, self.version, self.flags, false)?;
        buf[cursor..cursor + self.aux_track_type.len()].copy_from_slice(&self.aux_track_type);
        cursor += self.aux_track_type.len();
        buf[cursor] = 0;
        cursor += 1;
        Ok(cursor)
    }
}

/// The direct reference samples list (`refs`) sample group entry —
/// ISO/IEC 23008-12:2025 7.4.1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DirectReferenceSamplesList {
    sample_id: u32,
    direct_reference_sample_ids: Vec<u32>,
}

impl DirectReferenceSamplesList {
    /// The entry for `sample_id` and its direct references.
    #[must_use]
    pub fn new(sample_id: u32, direct_reference_sample_ids: Vec<u32>) -> Self {
        Self {
            sample_id,
            direct_reference_sample_ids,
        }
    }

    /// `sample_id` (0 for a non-reference sample).
    #[must_use]
    pub const fn sample_id(&self) -> u32 {
        self.sample_id
    }

    /// `direct_reference_sample_id[i]`.
    #[must_use]
    pub fn direct_reference_sample_ids(&self) -> &[u32] {
        &self.direct_reference_sample_ids
    }

    /// Parse the sample group entry body.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let sample_id = be_u32(body, 0, "refs sample_id")?;
        need_in(body, REFS_ID_LEN + REFS_COUNT_LEN, "refs")?;
        let count = usize::from(body[REFS_ID_LEN]);
        crate::box_support::check_entry_count(
            body.len() - (REFS_ID_LEN + REFS_COUNT_LEN),
            REFS_ID_LEN,
            count,
            "refs",
        )?;
        let mut direct_reference_sample_ids = Vec::with_capacity(count);
        let mut cursor = REFS_ID_LEN + REFS_COUNT_LEN;
        for _ in 0..count {
            direct_reference_sample_ids.push(be_u32(body, cursor, "refs reference")?);
            cursor += REFS_ID_LEN;
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("refs trailing bytes"));
        }
        Ok(Self {
            sample_id,
            direct_reference_sample_ids,
        })
    }

    /// The entry body length.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        REFS_ID_LEN + REFS_COUNT_LEN + self.direct_reference_sample_ids.len() * REFS_ID_LEN
    }

    /// Write the entry body.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[..REFS_ID_LEN].copy_from_slice(&self.sample_id.to_be_bytes());
        buf[REFS_ID_LEN] = fit_u8(self.direct_reference_sample_ids.len(), "refs count")?;
        let mut cursor = REFS_ID_LEN + REFS_COUNT_LEN;
        for reference in &self.direct_reference_sample_ids {
            buf[cursor..cursor + REFS_ID_LEN].copy_from_slice(&reference.to_be_bytes());
            cursor += REFS_ID_LEN;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// MjpegSampleEntry — 'mjpg' (23008-12 H.3.3)
// ---------------------------------------------------------------------------

/// Four-CC of the JPEG image sequence sample entry (`mjpg`, H.3.3).
pub const MJPG: [u8; BOX_TYPE_SIZE] = *b"mjpg";

/// The JPEG image sequence sample entry (`mjpg`) — ISO/IEC 23008-12:2025 H.3.3.
///
/// A `VisualSampleEntry`-derived entry; its optional `jpgC` configuration box
/// and any other child are kept in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MjpegSampleEntry {
    visual: crate::sample_entries::VisualSampleEntryFields,
    children: Vec<crate::box_support::OpaqueBox>,
}

impl MjpegSampleEntry {
    /// Build an entry with default fixed fields and no children.
    #[must_use]
    pub fn new() -> Self {
        Self {
            visual: crate::sample_entries::VisualSampleEntryFields::default(),
            children: Vec::new(),
        }
    }

    /// Set the coded dimensions.
    #[must_use]
    pub fn with_dimensions(mut self, width: u16, height: u16) -> Self {
        self.visual.width = width;
        self.visual.height = height;
        self
    }

    /// The fixed sample description fields.
    #[must_use]
    pub const fn visual(&self) -> &crate::sample_entries::VisualSampleEntryFields {
        &self.visual
    }

    /// Every child box after the fixed part, verbatim.
    #[must_use]
    pub fn children(&self) -> &[crate::box_support::OpaqueBox] {
        &self.children
    }

    /// The `jpgC` configuration, when present (H.3.2).
    #[must_use]
    pub fn jpeg_configuration(&self) -> Option<crate::image::JpegConfigurationProperty> {
        let child = self
            .children
            .iter()
            .find(|child| child.box_type == crate::image::JPGC)?;
        let mut whole = alloc::vec![0u8; BOX_HEADER_MIN_SIZE + child.data.len()];
        let size = broadcast_core::len::fit_u32(whole.len(), "jpgC").ok()?;
        whole[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        whole[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE]
            .copy_from_slice(&crate::image::JPGC);
        whole[BOX_HEADER_MIN_SIZE..].copy_from_slice(&child.data);
        crate::image::JpegConfigurationProperty::parse_box(&whole).ok()
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&MJPG) {
            return Err(Error::UnexpectedBox { expected: "mjpg" });
        }
        let mut visual = crate::sample_entries::VisualSampleEntryFields::parse_body(bytes, "mjpg")?;
        let region =
            crate::sample_entries::VisualSampleEntryFields::children_region(bytes, "mjpg")?;
        let (children, trailing) = crate::box_support::capture_children(region).into_parts();
        visual.trailing = trailing.into_boxed_slice();
        Ok(Self { visual, children })
    }
}

impl Default for MjpegSampleEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Parse<'a> for MjpegSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for MjpegSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE
            + crate::sample_entries::VisualSampleEntryFields::serialized_len()
            + self.visual.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &MJPG, need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        let end = c + self.visual.trailing.len();
        need_out(buf, end)?;
        buf[c..end].copy_from_slice(&self.visual.trailing);
        Ok(end)
    }
}
