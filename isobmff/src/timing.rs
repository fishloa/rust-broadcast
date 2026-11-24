//! Sample-timing and segment-index boxes — ISO/IEC 14496-12:2026 §8.6/§8.14.
//!
//! Typed containers for:
//!
//! | Box   | Name                     | Section    | Description                          |
//! |-------|--------------------------|------------|--------------------------------------|
//! | `stts`| TimeToSampleBox          | §8.6.1.2   | Decode-time delta table              |
//! | `ctts`| CompositionOffsetBox     | §8.6.1.3   | Composition offset (v0 unsigned / v1 signed) |
//! | `cslg`| CompositionToDecodeBox   | §8.6.1.4   | Composition–decode timeline shift summary (optional) |
//! | `elst`| EditListBox              | §8.6.6     | Presentation→media timeline mapping  |
//! | `sidx`| SegmentIndexBox          | §8.14.3    | Segment time/byte index for DASH/CMAF |
//!
//! All boxes are FullBoxes. Lengths/counts are computed from fields
//! (no `self.raw`).

use crate::box_support::{bounded_entry_count, flat_table_len, serialize_flat_table};
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::timescale::Timescale;

use broadcast_common::wire::{be_u32, be_u64, read_u16, read_u32};
use broadcast_common::{Parse, Serialize};

const U32_PAIR_LEN: usize = core::mem::size_of::<u32>() * 2;
const U64_PAIR_LEN: usize = core::mem::size_of::<u64>() * 2;
const CSLG_FIELD_COUNT: usize = 5;
const ELST_V0_ENTRY_LEN: usize = 3 * core::mem::size_of::<u32>();
const ELST_V1_ENTRY_LEN: usize = 2 * core::mem::size_of::<u64>() + core::mem::size_of::<u32>();
const SIDX_REFERENCE_ENTRY_LEN: usize = 3 * core::mem::size_of::<u32>();
const SIDX_FIXED_FIELD_COUNT: usize = 4;
const SIDX_REFERENCE_TYPE_SHIFT: u32 = u32::BITS - 1;
const SIDX_REFERENCED_SIZE_MASK: u32 = 0x7FFF_FFFF;
const SIDX_SAP_TYPE_SHIFT: u32 = u32::BITS - 4;
const SIDX_SAP_TYPE_MASK: u32 = 0x7;
const SIDX_SAP_DELTA_TIME_MASK: u32 = 0x0FFF_FFFF;
const SIDX_REFERENCED_SIZE_BITS: u32 = 31;
const SIDX_SAP_TYPE_BITS: u32 = 3;
const SIDX_SAP_DELTA_TIME_BITS: u32 = 28;

// ---------------------------------------------------------------------------
// Constants: 4-byte box-type literals as u32
// ---------------------------------------------------------------------------

const STTS_TYPE: u32 = u32::from_be_bytes(*b"stts");
const CTTS_TYPE: u32 = u32::from_be_bytes(*b"ctts");
const CSLG_TYPE: u32 = u32::from_be_bytes(*b"cslg");
const ELST_TYPE: u32 = u32::from_be_bytes(*b"elst");
const SIDX_TYPE: u32 = u32::from_be_bytes(*b"sidx");

// ---------------------------------------------------------------------------
// TimeToSampleBox — stts (ISO/IEC 14496-12:2026 §8.6.1.2)
// ---------------------------------------------------------------------------

/// Entry in the stts decode-time delta table (§8.6.1.2).
///
/// `sample_count` consecutive samples each have duration `sample_delta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SttsEntry {
    pub sample_count: u32,
    pub sample_delta: u32,
}

/// Time To Sample Box (`stts`) — ISO/IEC 14496-12:2026 §8.6.1.2.
///
/// Maps decode-time deltas to samples. The sum of `sample_count * sample_delta`
/// across all entries equals the track media duration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeToSampleBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<SttsEntry>,
}

impl TimeToSampleBox {
    /// Parse the body of an stts box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "stts body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "stts body")? & FULLBOX_FLAGS_MASK;
        let entry_count = be_u32(body, FULLBOX_EXTRA_SIZE, "stts body")? as usize;
        let mut c = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            U32_PAIR_LEN,
            entry_count,
        ));
        for _ in 0..entry_count {
            need_in(body, c + U32_PAIR_LEN, "stts entry")?;
            let sample_count = read_u32(body, &mut c, "stts entry")?;
            let sample_delta = read_u32(body, &mut c, "stts entry")?;
            entries.push(SttsEntry {
                sample_count,
                sample_delta,
            });
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl<'a> Parse<'a> for TimeToSampleBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "stts box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "stts box")?;
        if ty != STTS_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected stts",
            }));
        }
        Self::parse_body(&bytes[crate::box_types::box_header_len(bytes)?..])
    }
}

impl Serialize for TimeToSampleBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        flat_table_len(self.entries.len(), U32_PAIR_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_flat_table(
            buf,
            b"stts",
            (self.version, self.flags),
            &self.entries,
            U32_PAIR_LEN,
            |entry, out| {
                out[..core::mem::size_of::<u32>()]
                    .copy_from_slice(&entry.sample_count.to_be_bytes());
                out[core::mem::size_of::<u32>()..U32_PAIR_LEN]
                    .copy_from_slice(&entry.sample_delta.to_be_bytes());
                Ok(U32_PAIR_LEN)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// CompositionOffsetBox — ctts (ISO/IEC 14496-12:2026 §8.6.1.3)
// ---------------------------------------------------------------------------

/// Entry in the ctts composition-offset table.
///
/// For version 0: `sample_offset` is unsigned (DT < CT).
/// For version 1: `sample_offset` is signed (may be negative or a large positive
/// value encoded via wrapping; spec says the signed interpretation is used).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CttsEntry {
    pub sample_count: u32,
    /// Composition offset.
    ///
    /// v0 stores the raw u32.  CT = DT + `sample_offset`.
    /// v1 stores the signed i32.  The spec wraps u32 values through
    /// `sample_offset as i32` so the presentation is always signed.
    pub sample_offset: i32,
}

/// Composition Offset Box (`ctts`) — ISO/IEC 14496-12:2026 §8.6.1.3.
///
/// Present only when at least one sample has CT ≠ DT (§8.6.1.3).
/// v0: all offsets are non-negative (DT ≤ CT).
/// v1: offsets may be negative (earlier composition) or encode the
/// non-output-sample sentinel (-2^31) (§8.6.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompositionOffsetBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<CttsEntry>,
}

impl CompositionOffsetBox {
    /// Parse the body of a ctts box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "ctts body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "ctts body")? & FULLBOX_FLAGS_MASK;
        let entry_count = be_u32(body, FULLBOX_EXTRA_SIZE, "ctts body")? as usize;
        let entry_size: usize = U32_PAIR_LEN;
        let mut c = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            entry_size,
            entry_count,
        ));
        for _ in 0..entry_count {
            need_in(body, c + entry_size, "ctts entry")?;
            let sample_count = read_u32(body, &mut c, "ctts entry")?;
            let raw_offset = read_u32(body, &mut c, "ctts entry")?;
            let sample_offset = raw_offset as i32; // v0/v1 both stored as u32 on wire; v1 is signed int32
            entries.push(CttsEntry {
                sample_count,
                sample_offset,
            });
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl<'a> Parse<'a> for CompositionOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "ctts box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "ctts box")?;
        if ty != CTTS_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected ctts",
            }));
        }
        Self::parse_body(&bytes[crate::box_types::box_header_len(bytes)?..])
    }
}

impl Serialize for CompositionOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        flat_table_len(self.entries.len(), U32_PAIR_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_flat_table(
            buf,
            b"ctts",
            (self.version, self.flags),
            &self.entries,
            U32_PAIR_LEN,
            |entry, out| {
                out[..core::mem::size_of::<u32>()]
                    .copy_from_slice(&entry.sample_count.to_be_bytes());
                out[core::mem::size_of::<u32>()..U32_PAIR_LEN]
                    .copy_from_slice(&(entry.sample_offset as u32).to_be_bytes());
                Ok(U32_PAIR_LEN)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// CompositionToDecodeBox — cslg (ISO/IEC 14496-12:2026 §8.6.1.4)
// ---------------------------------------------------------------------------

/// Composition To Decode Box (`cslg`) — ISO/IEC 14496-12:2026 §8.6.1.4.
///
/// Optional.  Present only when `ctts` version == 1 (signed offsets) or when
/// the author wishes to optimise composition→decode timeline conversion.
/// v0: fields are 32-bit signed; v1: fields are 64-bit signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompositionToDecodeBox {
    pub version: u8,
    pub flags: u32,
    pub composition_to_dts_shift: i64,
    pub least_decode_to_display_delta: i64,
    pub greatest_decode_to_display_delta: i64,
    pub composition_start_time: i64,
    pub composition_end_time: i64,
}

impl CompositionToDecodeBox {
    /// Parse the body of a cslg box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "cslg body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "cslg body")? & FULLBOX_FLAGS_MASK;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        let (fld_size, have): (usize, &str) = if version == 0 {
            (core::mem::size_of::<u32>(), "cslg v0 field")
        } else {
            (core::mem::size_of::<u64>(), "cslg v1 field")
        };
        if payload.len() < fld_size * CSLG_FIELD_COUNT {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + fld_size * CSLG_FIELD_COUNT,
                have: body.len(),
                what: have,
            }));
        }
        let mut c = 0usize;
        let read_i64 = |buf: &[u8], off: usize, sz: usize| -> Result<i64> {
            if sz == core::mem::size_of::<u32>() {
                Ok(i64::from(be_u32(buf, off, "cslg field")? as i32))
            } else {
                Ok(be_u64(buf, off, "cslg field")? as i64)
            }
        };
        let composition_to_dts_shift = read_i64(payload, c, fld_size)?;
        c += fld_size;
        let least_decode_to_display_delta = read_i64(payload, c, fld_size)?;
        c += fld_size;
        let greatest_decode_to_display_delta = read_i64(payload, c, fld_size)?;
        c += fld_size;
        let composition_start_time = read_i64(payload, c, fld_size)?;
        c += fld_size;
        let composition_end_time = read_i64(payload, c, fld_size)?;
        Ok(Self {
            version,
            flags,
            composition_to_dts_shift,
            least_decode_to_display_delta,
            greatest_decode_to_display_delta,
            composition_start_time,
            composition_end_time,
        })
    }
}

impl<'a> Parse<'a> for CompositionToDecodeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "cslg box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "cslg box")?;
        if ty != CSLG_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected cslg",
            }));
        }
        Self::parse_body(&bytes[crate::box_types::box_header_len(bytes)?..])
    }
}

impl Serialize for CompositionToDecodeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let fld = if self.version == 0 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u64>()
        };
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + fld * CSLG_FIELD_COUNT
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"cslg",
            need,
            self.version,
            self.flags,
            true,
        )?;
        let write_i64 = |buf: &mut [u8], off: usize, sz: usize, v: i64| {
            if sz == core::mem::size_of::<u32>() {
                buf[off..off + core::mem::size_of::<u32>()]
                    .copy_from_slice(&(v as i32).to_be_bytes());
            } else {
                buf[off..off + core::mem::size_of::<u64>()].copy_from_slice(&v.to_be_bytes());
            }
        };
        let fld = if self.version == 0 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u64>()
        };
        write_i64(buf, c, fld, self.composition_to_dts_shift);
        c += fld;
        write_i64(buf, c, fld, self.least_decode_to_display_delta);
        c += fld;
        write_i64(buf, c, fld, self.greatest_decode_to_display_delta);
        c += fld;
        write_i64(buf, c, fld, self.composition_start_time);
        c += fld;
        write_i64(buf, c, fld, self.composition_end_time);
        Ok(c + fld)
    }
}

// ---------------------------------------------------------------------------
// EditListBox — elst (ISO/IEC 14496-12:2026 §8.6.6)
// ---------------------------------------------------------------------------

/// Entry in the elst edit list.
///
/// `segment_duration` is in the `mvhd` timescale (presentation timeline).
/// `media_time` is in the media timescale (`mdhd.timescale`), or -1 for an
/// empty edit (§8.6.6).
/// `media_rate_integer` / `media_rate_fraction` = 0 for dwell, 1 for normal
/// playback (§8.6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EditListEntry {
    pub segment_duration: u64,
    pub media_time: i64,
    pub media_rate_integer: i16,
    pub media_rate_fraction: i16,
}

/// Edit List Box (`elst`) — ISO/IEC 14496-12:2026 §8.6.6.
///
/// Maps presentation timeline to media timeline. Absent → implicit 1:1 mapping.
/// v0: fields are 32-bit; v1: fields are 64-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EditListBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<EditListEntry>,
}

impl EditListBox {
    /// Parse the body of an elst box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "elst body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "elst body")? & FULLBOX_FLAGS_MASK;
        let entry_count = be_u32(body, FULLBOX_EXTRA_SIZE, "elst body")? as usize;
        let entry_size = if version == 0 {
            ELST_V0_ENTRY_LEN
        } else {
            ELST_V1_ENTRY_LEN
        };
        let mut c = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            entry_size,
            entry_count,
        ));
        for _ in 0..entry_count {
            need_in(body, c + entry_size, "elst entry")?;
            let (segment_duration, media_time) = if version == 0 {
                let sd = u64::from(read_u32(body, &mut c, "elst entry")?);
                let mt_raw = read_u32(body, &mut c, "elst entry")?;
                let mt = i64::from(mt_raw as i32);
                (sd, mt)
            } else {
                let sd = be_u64(body, c, "elst entry")?;
                c += core::mem::size_of::<u64>();
                let mt = be_u64(body, c, "elst entry")? as i64;
                c += core::mem::size_of::<u64>();
                (sd, mt)
            };
            let mr_int = read_u16(body, &mut c, "elst entry")? as i16;
            let mr_frac = read_u16(body, &mut c, "elst entry")? as i16;
            entries.push(EditListEntry {
                segment_duration,
                media_time,
                media_rate_integer: mr_int,
                media_rate_fraction: mr_frac,
            });
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl<'a> Parse<'a> for EditListBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "elst box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "elst box")?;
        if ty != ELST_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected elst",
            }));
        }
        Self::parse_body(&bytes[crate::box_types::box_header_len(bytes)?..])
    }
}

impl EditListBox {
    fn entry_wire_len(version: u8) -> usize {
        if version == 0 {
            ELST_V0_ENTRY_LEN
        } else {
            ELST_V1_ENTRY_LEN
        }
    }
}

impl Serialize for EditListBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        flat_table_len(self.entries.len(), Self::entry_wire_len(self.version))
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let version = self.version;
        serialize_flat_table(
            buf,
            b"elst",
            (version, self.flags),
            &self.entries,
            Self::entry_wire_len(version),
            |entry, out| {
                let mut c = 0;
                if version == 0 {
                    let segment_duration = broadcast_common::len::fit_bits(
                        entry.segment_duration,
                        u32::BITS,
                        "segment_duration",
                    )? as u32;
                    // `media_time` is signed (-1 = empty edit, §8.6.6): version 0
                    // stores it as i32, so the check is range membership, not an
                    // unsigned bit-width fit.
                    let media_time = i32::try_from(entry.media_time).map_err(|_| {
                        Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                            field: "media_time",
                            value: entry.media_time as u64,
                            max: i64::from(i32::MAX) as u64,
                        })
                    })?;
                    out[..core::mem::size_of::<u32>()]
                        .copy_from_slice(&segment_duration.to_be_bytes());
                    out[core::mem::size_of::<u32>()..U32_PAIR_LEN]
                        .copy_from_slice(&(media_time as u32).to_be_bytes());
                    c += U32_PAIR_LEN;
                } else {
                    out[..core::mem::size_of::<u64>()]
                        .copy_from_slice(&entry.segment_duration.to_be_bytes());
                    out[core::mem::size_of::<u64>()..U64_PAIR_LEN]
                        .copy_from_slice(&entry.media_time.to_be_bytes());
                    c += U64_PAIR_LEN;
                }
                out[c..c + core::mem::size_of::<u16>()]
                    .copy_from_slice(&entry.media_rate_integer.to_be_bytes());
                out[c + core::mem::size_of::<u16>()..c + core::mem::size_of::<u32>()]
                    .copy_from_slice(&entry.media_rate_fraction.to_be_bytes());
                Ok(c + core::mem::size_of::<u32>())
            },
        )
    }
}

// ---------------------------------------------------------------------------
// SegmentIndexBox — sidx (ISO/IEC 14496-12:2026 §8.14.3)
// ---------------------------------------------------------------------------

/// A single reference entry in the Segment Index Box (§8.14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SidxReference {
    /// 0 = the reference points to a media segment (moof);
    /// 1 = the reference points to a sub-segment index (child sidx).
    pub reference_type: u8,
    /// Byte count from the first byte of the referenced item to the first byte
    /// of the next referenced item (or to the end of the segment).
    pub referenced_size: u32,
    /// Duration of the referenced (sub)segment on the timeline (same timescale
    /// as the sidx).
    pub subsegment_duration: u32,
    /// 1 = the referenced (sub)segment starts with a SAP.
    pub starts_with_sap: u8,
    /// SAP type (1–6) per §8.14.3.
    pub sap_type: u8,
    /// SAP delta time (0 if starts_with_sap==1 and SAP_type==1)
    pub sap_delta_time: u32,
}

/// Segment Index Box (`sidx`) — ISO/IEC 14496-12:2026 §8.14.3.
///
/// Provides a compact time/byte index for one reference stream within a segment.
/// v0: earliest_presentation_time / first_offset are 32-bit.
/// v1: earliest_presentation_time / first_offset are 64-bit.
///
/// Anchor for offsets = first byte after this sidx box (§8.14.3).
/// The sidx shall precede the material it documents (§8.14.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentIndexBox {
    pub version: u8,
    pub flags: u32,
    pub reference_id: u32,
    pub timescale: Timescale,
    pub earliest_presentation_time: u64,
    pub first_offset: u64,
    pub references: Vec<SidxReference>,
}

impl SegmentIndexBox {
    /// Parse the body of a sidx box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() * SIDX_FIXED_FIELD_COUNT,
            "sidx body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "sidx body")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE; // skip past version/flags to payload
        let reference_id = read_u32(body, &mut c, "sidx body")?;
        let timescale = Timescale::try_new(read_u32(body, &mut c, "sidx body")?)?;
        let (ept, first_offset) = if version == 0 {
            let ept = u64::from(read_u32(body, &mut c, "sidx body")?);
            let fo = u64::from(read_u32(body, &mut c, "sidx body")?);
            (ept, fo)
        } else {
            let ept = be_u64(body, c, "sidx body")?;
            c += core::mem::size_of::<u64>();
            let fo = be_u64(body, c, "sidx body")?;
            c += core::mem::size_of::<u64>();
            (ept, fo)
        };
        need_in(body, c + core::mem::size_of::<u32>(), "sidx reserved+count")?;
        // ISO/IEC 14496-12:2026 §8.14.3.2 (Table): `reserved(16) = 0` comes BEFORE
        // `reference_count(16)`, not folded into it — before this fix, parse
        // read `reference_count` from the `reserved` field's own 2 bytes,
        // so every conformant sidx (`reserved == 0`) parsed as
        // `reference_count == 0` (C9, #1010).
        let _reserved = read_u16(body, &mut c, "sidx reserved+count")?;
        let reference_count = read_u16(body, &mut c, "sidx reserved+count")? as usize;

        let mut references = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            SIDX_REFERENCE_ENTRY_LEN,
            reference_count,
        ));
        for _ in 0..reference_count {
            need_in(body, c + SIDX_REFERENCE_ENTRY_LEN, "sidx reference entry")?;
            let raw_ref = read_u32(body, &mut c, "sidx reference entry")?;
            let reference_type = ((raw_ref >> SIDX_REFERENCE_TYPE_SHIFT) & 1) as u8;
            let referenced_size = raw_ref & SIDX_REFERENCED_SIZE_MASK;
            let subsegment_duration = read_u32(body, &mut c, "sidx reference entry")?;
            let raw_sap = read_u32(body, &mut c, "sidx reference entry")?;
            let starts_with_sap = ((raw_sap >> SIDX_REFERENCE_TYPE_SHIFT) & 1) as u8;
            let sap_type = ((raw_sap >> SIDX_SAP_TYPE_SHIFT) & SIDX_SAP_TYPE_MASK) as u8;
            let sap_delta_time = raw_sap & SIDX_SAP_DELTA_TIME_MASK;
            references.push(SidxReference {
                reference_type,
                referenced_size,
                subsegment_duration,
                starts_with_sap,
                sap_type,
                sap_delta_time,
            });
        }
        Ok(Self {
            version,
            flags,
            reference_id,
            timescale,
            earliest_presentation_time: ept,
            first_offset,
            references,
        })
    }
}

impl<'a> Parse<'a> for SegmentIndexBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "sidx box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "sidx box")?;
        if ty != SIDX_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected sidx",
            }));
        }
        Self::parse_body(&bytes[crate::box_types::box_header_len(bytes)?..])
    }
}

impl Serialize for SegmentIndexBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let time_size = if self.version == 0 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u64>()
        };
        BOX_HEADER_MIN_SIZE
            + FULLBOX_EXTRA_SIZE
            + core::mem::size_of::<u32>()
            + core::mem::size_of::<u32>()
            + time_size
            + time_size
            + core::mem::size_of::<u32>()
            + self.references.len() * SIDX_REFERENCE_ENTRY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"sidx",
            need,
            self.version,
            self.flags,
            true,
        )?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.reference_id.to_be_bytes());
        c += core::mem::size_of::<u32>();
        buf[c..c + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.timescale.get().to_be_bytes());
        c += core::mem::size_of::<u32>();
        if self.version == 0 {
            let ept = broadcast_common::len::fit_bits(
                self.earliest_presentation_time,
                u32::BITS,
                "earliest_presentation_time",
            )? as u32;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&ept.to_be_bytes());
            c += core::mem::size_of::<u32>();
            let first_offset =
                broadcast_common::len::fit_bits(self.first_offset, u32::BITS, "first_offset")?
                    as u32;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&first_offset.to_be_bytes());
            c += core::mem::size_of::<u32>();
        } else {
            buf[c..c + core::mem::size_of::<u64>()]
                .copy_from_slice(&self.earliest_presentation_time.to_be_bytes());
            c += core::mem::size_of::<u64>();
            buf[c..c + core::mem::size_of::<u64>()]
                .copy_from_slice(&self.first_offset.to_be_bytes());
            c += core::mem::size_of::<u64>();
        }
        // reserved(16) = 0, THEN reference_count(16) (see the parse-side
        // comment) — before the fix, this wrote only `reference_count` at
        // the position a reader expects `reserved`, and never wrote
        // `reference_count`'s own bytes at all.
        buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&0u16.to_be_bytes());
        c += core::mem::size_of::<u16>();
        let reference_count =
            broadcast_common::len::fit_u16(self.references.len(), "reference_count")?;
        buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&reference_count.to_be_bytes());
        c += core::mem::size_of::<u16>();
        for r in &self.references {
            let reference_type =
                broadcast_common::len::fit_bits(u64::from(r.reference_type), 1, "reference_type")?
                    as u32;
            let referenced_size = broadcast_common::len::fit_bits(
                u64::from(r.referenced_size),
                SIDX_REFERENCED_SIZE_BITS,
                "referenced_size",
            )? as u32;
            let raw_ref = (reference_type << SIDX_REFERENCE_TYPE_SHIFT) | referenced_size;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&raw_ref.to_be_bytes());
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u32>()]
                .copy_from_slice(&r.subsegment_duration.to_be_bytes());
            c += core::mem::size_of::<u32>();
            let starts_with_sap = broadcast_common::len::fit_bits(
                u64::from(r.starts_with_sap),
                1,
                "starts_with_sap",
            )? as u32;
            let sap_type = broadcast_common::len::fit_bits(
                u64::from(r.sap_type),
                SIDX_SAP_TYPE_BITS,
                "sap_type",
            )? as u32;
            let sap_delta_time = broadcast_common::len::fit_bits(
                u64::from(r.sap_delta_time),
                SIDX_SAP_DELTA_TIME_BITS,
                "sap_delta_time",
            )? as u32;
            let raw_sap = (starts_with_sap << SIDX_REFERENCE_TYPE_SHIFT)
                | (sap_type << SIDX_SAP_TYPE_SHIFT)
                | sap_delta_time;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&raw_sap.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    use super::*;
    use broadcast_common::Serialize;

    // -----------------------------------------------------------------------
    // stts unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn stts_round_trip_empty() {
        let b = TimeToSampleBox {
            version: 0,
            flags: 0,
            entries: vec![],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = TimeToSampleBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn stts_round_trip_single_entry() {
        let b = TimeToSampleBox {
            version: 0,
            flags: 0,
            entries: vec![SttsEntry {
                sample_count: 50,
                sample_delta: 512,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 8);
        let parsed = TimeToSampleBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].sample_count, 50);
        assert_eq!(parsed.entries[0].sample_delta, 512);
    }

    #[test]
    fn stts_parse_body_api() {
        let b = TimeToSampleBox {
            version: 0,
            flags: 0,
            entries: vec![SttsEntry {
                sample_count: 10,
                sample_delta: 300,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = TimeToSampleBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed, b);
    }

    // -----------------------------------------------------------------------
    // ctts unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn ctts_round_trip_v0() {
        let b = CompositionOffsetBox {
            version: 0,
            flags: 0,
            entries: vec![CttsEntry {
                sample_count: 1,
                sample_offset: 1024,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = CompositionOffsetBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn ctts_parse_body_v0_multi() {
        let b = CompositionOffsetBox {
            version: 0,
            flags: 0,
            entries: vec![
                CttsEntry {
                    sample_count: 1,
                    sample_offset: 1024,
                },
                CttsEntry {
                    sample_count: 2,
                    sample_offset: 512,
                },
            ],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = CompositionOffsetBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[1].sample_offset, 512);
    }

    #[test]
    fn ctts_parse_wrong_type() {
        let bytes = [0, 0, 0, 16, b'x', b'x', b'x', b'x', 0, 0, 0, 0, 0, 0, 0, 0];
        let result = CompositionOffsetBox::parse(&bytes);
        assert!(matches!(result, Err(crate::error::Error::InvalidValue(..))));
    }

    // -----------------------------------------------------------------------
    // cslg unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn cslg_round_trip_v0() {
        let b = CompositionToDecodeBox {
            version: 0,
            flags: 0,
            composition_to_dts_shift: 0,
            least_decode_to_display_delta: 0,
            greatest_decode_to_display_delta: 1024,
            composition_start_time: 0,
            composition_end_time: 25500,
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 * 5);
        let parsed = CompositionToDecodeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn cslg_round_trip_v1() {
        let b = CompositionToDecodeBox {
            version: 1,
            flags: 0,
            composition_to_dts_shift: 0,
            least_decode_to_display_delta: -100,
            greatest_decode_to_display_delta: 100500,
            composition_start_time: 0,
            composition_end_time: 9999999999,
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 8 * 5);
        let parsed = CompositionToDecodeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    // -----------------------------------------------------------------------
    // elst unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn elst_round_trip_v0_single() {
        let b = EditListBox {
            version: 0,
            flags: 0,
            entries: vec![EditListEntry {
                segment_duration: 2000,
                media_time: 1024,
                media_rate_integer: 1,
                media_rate_fraction: 0,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 12); // header + fullbox + ec + entry
        let parsed = EditListBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].segment_duration, 2000);
        assert_eq!(parsed.entries[0].media_time, 1024);
        assert_eq!(parsed.entries[0].media_rate_integer, 1);
    }

    #[test]
    fn elst_v0_empty_edit() {
        // media_time = -1 → empty edit
        let b = EditListBox {
            version: 0,
            flags: 0,
            entries: vec![
                EditListEntry {
                    segment_duration: 500,
                    media_time: -1,
                    media_rate_integer: 0,
                    media_rate_fraction: 0,
                },
                EditListEntry {
                    segment_duration: 2000,
                    media_time: 0,
                    media_rate_integer: 1,
                    media_rate_fraction: 0,
                },
            ],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = EditListBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].media_time, -1);
        // Verify serialized size: 8+4+4 + 2*12 = 40
        assert_eq!(bytes.len(), 40);
    }

    #[test]
    fn elst_round_trip_v1() {
        let b = EditListBox {
            version: 1,
            flags: 0,
            entries: vec![EditListEntry {
                segment_duration: 0x1_0000_0000,
                media_time: 0x2_0000_0000,
                media_rate_integer: 1,
                media_rate_fraction: 0,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 20); // header + fullbox + ec + v1 entry
        let parsed = EditListBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn elst_parse_body_api() {
        let b = EditListBox {
            version: 0,
            flags: 0,
            entries: vec![EditListEntry {
                segment_duration: 100,
                media_time: 50,
                media_rate_integer: 1,
                media_rate_fraction: 0,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = EditListBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed, b);
    }

    // -----------------------------------------------------------------------
    // sidx unit tests
    // -----------------------------------------------------------------------

    /// C9 (#1010): a real `sidx` box from `MP4Box -dash` (onDemand profile),
    /// extracted verbatim from `in_dashinit.mp4` — see
    /// `fixtures/mp4/PROVENANCE.md`. Oracle: GPAC's own `MP4Box -diso` XML dump
    /// of the same file (`reference_ID=1`, `timescale=12800`, 2 references).
    /// Before the fix, parse read `reference_count == 0` (mistaking the
    /// `reserved` field's bytes for it) and returned zero references; every
    /// sidx this type serialized was 2 bytes short of a conformant one.
    #[test]
    fn sidx_real_fixture_oracle() {
        let path = test_fixtures::path("mp4/sidx-real.bin");
        let data = std::fs::read(path).expect("fixtures/mp4/sidx-real.bin must exist");

        let sidx = SegmentIndexBox::parse(&data).expect("parse real sidx");
        assert_eq!(sidx.version, 0);
        assert_eq!(sidx.reference_id, 1);
        assert_eq!(sidx.timescale, timescale(12800));
        assert_eq!(sidx.earliest_presentation_time, 0);
        assert_eq!(sidx.first_offset, 0);
        assert_eq!(
            sidx.references.len(),
            2,
            "MP4Box -diso oracle: 2 <Reference> entries"
        );
        assert_eq!(sidx.references[0].referenced_size, 11401);
        assert_eq!(sidx.references[0].subsegment_duration, 12800);
        assert_eq!(sidx.references[0].starts_with_sap, 1);
        assert_eq!(sidx.references[0].sap_type, 1);
        assert_eq!(sidx.references[1].referenced_size, 10306);
        assert_eq!(sidx.references[1].subsegment_duration, 12800);

        // Byte-exact round-trip against the real fixture bytes.
        let mut out = alloc::vec![0u8; sidx.serialized_len()];
        let n = sidx.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], &data[..], "byte-exact round-trip vs real sidx");
    }

    #[test]
    fn sidx_round_trip_v0() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: timescale(90000),
            earliest_presentation_time: 0,
            first_offset: 68,
            references: vec![
                SidxReference {
                    reference_type: 0,
                    referenced_size: 1000,
                    subsegment_duration: 180000,
                    starts_with_sap: 1,
                    sap_type: 1,
                    sap_delta_time: 0,
                },
                SidxReference {
                    reference_type: 0,
                    referenced_size: 1200,
                    subsegment_duration: 180000,
                    starts_with_sap: 1,
                    sap_type: 1,
                    sap_delta_time: 0,
                },
            ],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SegmentIndexBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    /// A version-0 sidx with `earliest_presentation_time` past `u32::MAX`
    /// cannot fit that form's 32-bit field (#1129): unfixed,
    /// `(value as u32)` silently truncated it.
    #[test]
    fn sidx_v0_oversized_earliest_presentation_time_errors() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: timescale(90000),
            earliest_presentation_time: u64::from(u32::MAX) + 1,
            first_offset: 0,
            references: vec![],
        };
        let err = b.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "earliest_presentation_time",
                    ..
                })
            ),
            "expected FieldOverflow for earliest_presentation_time, got {err:?}"
        );
    }

    /// The boundary: exactly `u32::MAX` still round-trips in version 0.
    #[test]
    fn sidx_v0_max_earliest_presentation_time_round_trips() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: timescale(90000),
            earliest_presentation_time: u64::from(u32::MAX),
            first_offset: 0,
            references: vec![],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SegmentIndexBox::parse(&bytes).unwrap();
        assert_eq!(parsed.earliest_presentation_time, u64::from(u32::MAX));
    }

    /// A `referenced_size` past the 31-bit field's max cannot be masked
    /// without error (#1129): unfixed, `& 0x7FFF_FFFF` silently kept only
    /// the low 31 bits.
    #[test]
    fn sidx_reference_oversized_referenced_size_errors() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: timescale(90000),
            earliest_presentation_time: 0,
            first_offset: 0,
            references: vec![SidxReference {
                reference_type: 0,
                referenced_size: 1 << 31,
                subsegment_duration: 1000,
                starts_with_sap: 1,
                sap_type: 1,
                sap_delta_time: 0,
            }],
        };
        let err = b.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "referenced_size",
                    ..
                })
            ),
            "expected FieldOverflow for referenced_size, got {err:?}"
        );
    }

    #[test]
    fn sidx_v1_round_trip() {
        let b = SegmentIndexBox {
            version: 1,
            flags: 0,
            reference_id: 3,
            timescale: timescale(48000),
            earliest_presentation_time: 0x1_0000_0000,
            first_offset: 0x2_0000_0000,
            references: vec![SidxReference {
                reference_type: 1,
                referenced_size: 500,
                subsegment_duration: 96000,
                starts_with_sap: 1,
                sap_type: 1,
                sap_delta_time: 0,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SegmentIndexBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn sidx_field_bit_boundaries() {
        // Verify sub-byte field packing for sidx references
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: timescale(90000),
            earliest_presentation_time: 0,
            first_offset: 0,
            references: vec![SidxReference {
                reference_type: 1,            // 1<<31 in packed u32
                referenced_size: 0x7FFF_FFFF, // max 31-bit
                subsegment_duration: 0xFFFF_FFFF,
                starts_with_sap: 1,          // 1<<31
                sap_type: 7,                 // 7<<28
                sap_delta_time: 0x0FFF_FFFF, // max 28-bit
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SegmentIndexBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn sidx_parse_body_api() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: timescale(90000),
            earliest_presentation_time: 0,
            first_offset: 0,
            references: vec![],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SegmentIndexBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed, b);
    }
}
