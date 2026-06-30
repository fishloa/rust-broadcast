//! Sample-timing and segment-index boxes — ISO/IEC 14496-12:2015 §8.6/§8.16.
//!
//! Typed containers for:
//!
//! | Box   | Name                     | Section    | Description                          |
//! |-------|--------------------------|------------|--------------------------------------|
//! | `stts`| TimeToSampleBox          | §8.6.1.2   | Decode-time delta table              |
//! | `ctts`| CompositionOffsetBox     | §8.6.1.3   | Composition offset (v0 unsigned / v1 signed) |
//! | `cslg`| CompositionToDecodeBox   | §8.6.1.4   | Composition–decode timeline shift summary (optional) |
//! | `elst`| EditListBox              | §8.6.6     | Presentation→media timeline mapping  |
//! | `sidx`| SegmentIndexBox          | §8.16.3    | Segment time/byte index for DASH/CMAF |
//!
//! All boxes are FullBoxes. Lengths/counts are computed from fields
//! (no `self.raw`).

use crate::error::{Error, Result};
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

const BOX_HEADER_SIZE: usize = 8;
const FULLBOX_EXTRA_SIZE: usize = 4;

// ---------------------------------------------------------------------------
// Constants: 4-byte box-type literals as u32
// ---------------------------------------------------------------------------

const STTS_TYPE: u32 = u32::from_be_bytes(*b"stts");
const CTTS_TYPE: u32 = u32::from_be_bytes(*b"ctts");
const CSLG_TYPE: u32 = u32::from_be_bytes(*b"cslg");
const ELST_TYPE: u32 = u32::from_be_bytes(*b"elst");
const SIDX_TYPE: u32 = u32::from_be_bytes(*b"sidx");

// ---------------------------------------------------------------------------
// TimeToSampleBox — stts (ISO/IEC 14496-12:2015 §8.6.1.2)
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

/// Time To Sample Box (`stts`) — ISO/IEC 14496-12:2015 §8.6.1.2.
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
        if body.len() < FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4,
                have: body.len(),
                what: "stts body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let entry_count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
        let mut c = FULLBOX_EXTRA_SIZE + 4;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if body.len() < c + 8 {
                return Err(Error::BufferTooShort {
                    need: c + 8,
                    have: body.len(),
                    what: "stts entry",
                });
            }
            let sample_count = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            let sample_delta =
                u32::from_be_bytes([body[c + 4], body[c + 5], body[c + 6], body[c + 7]]);
            entries.push(SttsEntry {
                sample_count,
                sample_delta,
            });
            c += 8;
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "stts box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != STTS_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected stts",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for TimeToSampleBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + self.entries.len() * 8
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stts");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.sample_count.to_be_bytes());
            buf[c + 4..c + 8].copy_from_slice(&entry.sample_delta.to_be_bytes());
            c += 8;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// CompositionOffsetBox — ctts (ISO/IEC 14496-12:2015 §8.6.1.3)
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

/// Composition Offset Box (`ctts`) — ISO/IEC 14496-12:2015 §8.6.1.3.
///
/// Present only when at least one sample has CT ≠ DT (§8.6.1.3 L3043).
/// v0: all offsets are non-negative (DT ≤ CT).
/// v1: offsets may be negative (earlier composition) or encode the
/// non-output-sample sentinel (-2^31) (§8.6.1.3 L3009).
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
        if body.len() < FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4,
                have: body.len(),
                what: "ctts body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let entry_count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
        let entry_size: usize = 8; // sample_count(32) + sample_offset(32)
        let mut c = FULLBOX_EXTRA_SIZE + 4;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if body.len() < c + entry_size {
                return Err(Error::BufferTooShort {
                    need: c + entry_size,
                    have: body.len(),
                    what: "ctts entry",
                });
            }
            let sample_count = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            let raw_offset =
                u32::from_be_bytes([body[c + 4], body[c + 5], body[c + 6], body[c + 7]]);
            let sample_offset = raw_offset as i32; // v0/v1 both stored as u32 on wire; v1 is signed int32
            entries.push(CttsEntry {
                sample_count,
                sample_offset,
            });
            c += entry_size;
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "ctts box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != CTTS_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected ctts",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for CompositionOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + self.entries.len() * 8
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"ctts");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.sample_count.to_be_bytes());
            buf[c + 4..c + 8].copy_from_slice(&(entry.sample_offset as u32).to_be_bytes());
            c += 8;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// CompositionToDecodeBox — cslg (ISO/IEC 14496-12:2015 §8.6.1.4)
// ---------------------------------------------------------------------------

/// Composition To Decode Box (`cslg`) — ISO/IEC 14496-12:2015 §8.6.1.4.
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
        if body.len() < FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4,
                have: body.len(),
                what: "cslg body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        let (fld_size, have): (usize, &str) = if version == 0 {
            (4, "cslg v0 field")
        } else {
            (8, "cslg v1 field")
        };
        if payload.len() < fld_size * 5 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + fld_size * 5,
                have: body.len(),
                what: have,
            });
        }
        let mut c = 0usize;
        let read_i64 = |buf: &[u8], off: usize, sz: usize| -> i64 {
            if sz == 4 {
                i32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as i64
            } else {
                i64::from_be_bytes([
                    buf[off],
                    buf[off + 1],
                    buf[off + 2],
                    buf[off + 3],
                    buf[off + 4],
                    buf[off + 5],
                    buf[off + 6],
                    buf[off + 7],
                ])
            }
        };
        let composition_to_dts_shift = read_i64(payload, c, fld_size);
        c += fld_size;
        let least_decode_to_display_delta = read_i64(payload, c, fld_size);
        c += fld_size;
        let greatest_decode_to_display_delta = read_i64(payload, c, fld_size);
        c += fld_size;
        let composition_start_time = read_i64(payload, c, fld_size);
        c += fld_size;
        let composition_end_time = read_i64(payload, c, fld_size);
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "cslg box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != CSLG_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected cslg",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for CompositionToDecodeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let fld = if self.version == 0 { 4 } else { 8 };
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + fld * 5
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"cslg");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        let write_i64 = |buf: &mut [u8], off: usize, sz: usize, v: i64| {
            if sz == 4 {
                buf[off..off + 4].copy_from_slice(&(v as i32).to_be_bytes());
            } else {
                buf[off..off + 8].copy_from_slice(&v.to_be_bytes());
            }
        };
        let fld = if self.version == 0 { 4 } else { 8 };
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
// EditListBox — elst (ISO/IEC 14496-12:2015 §8.6.6)
// ---------------------------------------------------------------------------

/// Entry in the elst edit list.
///
/// `segment_duration` is in the `mvhd` timescale (presentation timeline).
/// `media_time` is in the media timescale (`mdhd.timescale`), or -1 for an
/// empty edit (§8.6.6 L3392).
/// `media_rate_integer` / `media_rate_fraction` = 0 for dwell, 1 for normal
/// playback (§8.6.6 L3392).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EditListEntry {
    pub segment_duration: u64,
    pub media_time: i64,
    pub media_rate_integer: i16,
    pub media_rate_fraction: i16,
}

/// Edit List Box (`elst`) — ISO/IEC 14496-12:2015 §8.6.6.
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
        if body.len() < FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4,
                have: body.len(),
                what: "elst body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let entry_count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
        let entry_size: usize = if version == 0 { 4 + 4 + 4 } else { 8 + 8 + 4 }; // sd(32)+mt(32)+rate(32) or sd(64)+mt(64)+rate(32)
        let mut c = FULLBOX_EXTRA_SIZE + 4;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if body.len() < c + entry_size {
                return Err(Error::BufferTooShort {
                    need: c + entry_size,
                    have: body.len(),
                    what: "elst entry",
                });
            }
            let (segment_duration, media_time) = if version == 0 {
                let sd =
                    u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as u64;
                let mt_raw =
                    u32::from_be_bytes([body[c + 4], body[c + 5], body[c + 6], body[c + 7]]);
                let mt = mt_raw as i32 as i64;
                c += 8;
                (sd, mt)
            } else {
                let sd = u64::from_be_bytes([
                    body[c],
                    body[c + 1],
                    body[c + 2],
                    body[c + 3],
                    body[c + 4],
                    body[c + 5],
                    body[c + 6],
                    body[c + 7],
                ]);
                let mt = i64::from_be_bytes([
                    body[c + 8],
                    body[c + 9],
                    body[c + 10],
                    body[c + 11],
                    body[c + 12],
                    body[c + 13],
                    body[c + 14],
                    body[c + 15],
                ]);
                c += 16;
                (sd, mt)
            };
            let mr_int = i16::from_be_bytes([body[c], body[c + 1]]);
            let mr_frac = i16::from_be_bytes([body[c + 2], body[c + 3]]);
            c += 4;
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "elst box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != ELST_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected elst",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for EditListBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entry_wire = if self.version == 0 { 12 } else { 20 };
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + self.entries.len() * entry_wire
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"elst");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            if self.version == 0 {
                buf[c..c + 4].copy_from_slice(&(entry.segment_duration as u32).to_be_bytes());
                buf[c + 4..c + 8].copy_from_slice(&(entry.media_time as u32).to_be_bytes());
                c += 8;
            } else {
                buf[c..c + 8].copy_from_slice(&entry.segment_duration.to_be_bytes());
                buf[c + 8..c + 16].copy_from_slice(&entry.media_time.to_be_bytes());
                c += 16;
            }
            buf[c..c + 2].copy_from_slice(&entry.media_rate_integer.to_be_bytes());
            buf[c + 2..c + 4].copy_from_slice(&entry.media_rate_fraction.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SegmentIndexBox — sidx (ISO/IEC 14496-12:2015 §8.16.3)
// ---------------------------------------------------------------------------

/// A single reference entry in the Segment Index Box (§8.16.3).
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
    /// SAP type (1–6) per §8.16.3.
    pub sap_type: u8,
    /// SAP delta time (0 if starts_with_sap==1 and SAP_type==1)
    pub sap_delta_time: u32,
}

/// Segment Index Box (`sidx`) — ISO/IEC 14496-12:2015 §8.16.3.
///
/// Provides a compact time/byte index for one reference stream within a segment.
/// v0: earliest_presentation_time / first_offset are 32-bit.
/// v1: earliest_presentation_time / first_offset are 64-bit.
///
/// Anchor for offsets = first byte after this sidx box (§8.16.3 L6413).
/// The sidx shall precede the material it documents (§8.16.3 L6467).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentIndexBox {
    pub version: u8,
    pub flags: u32,
    pub reference_id: u32,
    pub timescale: u32,
    pub earliest_presentation_time: u64,
    pub first_offset: u64,
    pub references: Vec<SidxReference>,
}

impl SegmentIndexBox {
    /// Parse the body of a sidx box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        if body.len() < FULLBOX_EXTRA_SIZE + 4 + 4 + 4 + 2 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4 + 4 + 4 + 2,
                have: body.len(),
                what: "sidx body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let mut c = FULLBOX_EXTRA_SIZE; // skip past version/flags to payload
        let reference_id = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
        c += 4;
        let timescale = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
        c += 4;
        let (ept, first_offset) = if version == 0 {
            let ept = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as u64;
            c += 4;
            let fo = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as u64;
            c += 4;
            (ept, fo)
        } else {
            let ept = u64::from_be_bytes([
                body[c],
                body[c + 1],
                body[c + 2],
                body[c + 3],
                body[c + 4],
                body[c + 5],
                body[c + 6],
                body[c + 7],
            ]);
            c += 8;
            let fo = u64::from_be_bytes([
                body[c],
                body[c + 1],
                body[c + 2],
                body[c + 3],
                body[c + 4],
                body[c + 5],
                body[c + 6],
                body[c + 7],
            ]);
            c += 8;
            (ept, fo)
        };
        if body.len() < c + 2 {
            return Err(Error::BufferTooShort {
                need: c + 2,
                have: body.len(),
                what: "sidx reserved+count",
            });
        }
        let _reserved = body[c] >> 4; // 16-bit field: reserved(16)
        let reference_count = u16::from_be_bytes([body[c], body[c + 1]]) as usize;
        c += 2;

        let mut references = Vec::with_capacity(reference_count);
        for _ in 0..reference_count {
            if body.len() < c + 12 {
                return Err(Error::BufferTooShort {
                    need: c + 12,
                    have: body.len(),
                    what: "sidx reference entry",
                });
            }
            // reference_type(1) + referenced_size(31)
            let raw_ref = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            let reference_type = ((raw_ref >> 31) & 1) as u8;
            let referenced_size = raw_ref & 0x7FFF_FFFF;
            c += 4;
            let subsegment_duration =
                u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            // starts_with_SAP(1) + SAP_type(3) + SAP_delta_time(28)
            let raw_sap = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            let starts_with_sap = ((raw_sap >> 31) & 1) as u8;
            let sap_type = ((raw_sap >> 28) & 0x7) as u8;
            let sap_delta_time = raw_sap & 0x0FFF_FFFF;
            c += 4;
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "sidx box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != SIDX_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected sidx",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for SegmentIndexBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let time_size: usize = if self.version == 0 { 4 } else { 8 };
        BOX_HEADER_SIZE
            + FULLBOX_EXTRA_SIZE
            + 4  // reference_id
            + 4  // timescale
            + time_size // earliest_presentation_time
            + time_size // first_offset
            + 2  // reserved(16) + reference_count(16)
            + self.references.len() * 12 // each ref: 4+4+4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"sidx");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.reference_id.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
        c += 4;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&(self.earliest_presentation_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.first_offset as u32).to_be_bytes());
            c += 4;
        } else {
            buf[c..c + 8].copy_from_slice(&self.earliest_presentation_time.to_be_bytes());
            c += 8;
            buf[c..c + 8].copy_from_slice(&self.first_offset.to_be_bytes());
            c += 8;
        }
        buf[c..c + 2].copy_from_slice(&(self.references.len() as u16).to_be_bytes());
        c += 2;
        for r in &self.references {
            let raw_ref = ((r.reference_type as u32) << 31) | (r.referenced_size & 0x7FFF_FFFF);
            buf[c..c + 4].copy_from_slice(&raw_ref.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&r.subsegment_duration.to_be_bytes());
            c += 4;
            let raw_sap = ((r.starts_with_sap as u32) << 31)
                | ((r.sap_type as u32) << 28)
                | (r.sap_delta_time & 0x0FFF_FFFF);
            buf[c..c + 4].copy_from_slice(&raw_sap.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
        let parsed = CompositionOffsetBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[1].sample_offset, 512);
    }

    #[test]
    fn ctts_parse_wrong_type() {
        let bytes = [0, 0, 0, 16, b'x', b'x', b'x', b'x', 0, 0, 0, 0, 0, 0, 0, 0];
        let result = CompositionOffsetBox::parse(&bytes);
        assert!(result.is_err());
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
        let parsed = EditListBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed, b);
    }

    // -----------------------------------------------------------------------
    // sidx unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn sidx_round_trip_v0() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: 90000,
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
        let bytes = b.to_bytes();
        let parsed = SegmentIndexBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn sidx_v1_round_trip() {
        let b = SegmentIndexBox {
            version: 1,
            flags: 0,
            reference_id: 3,
            timescale: 48000,
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
        let bytes = b.to_bytes();
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
            timescale: 90000,
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
        let bytes = b.to_bytes();
        let parsed = SegmentIndexBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn sidx_parse_body_api() {
        let b = SegmentIndexBox {
            version: 0,
            flags: 0,
            reference_id: 1,
            timescale: 90000,
            earliest_presentation_time: 0,
            first_offset: 0,
            references: vec![],
        };
        let bytes = b.to_bytes();
        let parsed = SegmentIndexBox::parse_body(&bytes[8..]).unwrap();
        assert_eq!(parsed, b);
    }
}
