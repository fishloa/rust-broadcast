//! Compact SCTE 35 Encoding Format — ETSI TS 103 752-1 V1.2.1 §8.3.3,
//! Tables 5–10 (PDF pp.26–28).
//!
//! **NEW binary syntax** (DVB Targeted Advertising Part 1). Watermark carriers
//! can have very limited data capacity, so the spec defines an optional compact
//! alternative to the full SCTE 35 message for conveying certain SCTE 35 messages
//! via watermark messages (§8.3.3). It encodes only the fields that vary; all
//! other base SCTE 35 / `segmentation_descriptor()` / `DVB_DAS_descriptor()`
//! fields are presumed to take the values fixed by the §5.3 profile (the
//! implied-value Tables 7, 9, 10).
//!
//! The dispatch byte ([`compact_SCTE_35()`](CompactScte35)) selects between
//! [`CompactTimeSignal`] (`message_type == 0x00`, Table 6) and
//! [`CompactSpliceInsert`] (`message_type == 0x01`, Table 8).
//!
//! # Bit layout (#1102 W5)
//!
//! Tables 6 and 8 are **bit-packed with no padding other than the explicit
//! `reserved` rows**: `compact_time_signal()` is 1+6+8+33 = 48 bits to
//! `pts_time` and byte-aligned by construction, and `compact_splice_insert()`
//! closes its fixed part with `DAS_descriptor_flag` (1 bit) + `reserved`
//! (6 bits) to land on a byte boundary (152 bits). The PDF render
//! (ETSI TS 103 752-1 V1.2.1, pp.27-28) offsets the "No. of Bits" column by one
//! row from `cw_index` down; re-aligning it row-for-row (re-derived from the
//! page image and the text layer, see `docs/dvb_ta/compact-scte35.md`) gives
//! the widths implemented here, and every layout then sums to a whole number of
//! bytes - a cross-check the earlier reconstruction (which inserted
//! SCTE 35-style reserved bits the table does not list) did not satisfy.
//! The one remaining oddity is `E_CRC_32`, whose rendered width is `8` against
//! the name and `rpchof` mnemonic: it is read as the 32-bit CRC.
//!
//! No real compact-watermark capture has been obtained, so every test here is
//! built from the spec table's own widths rather than a third-party oracle.

use alloc::vec::Vec;

use crate::dvb_ta::das_descriptor::{DasNibbleOrder, EquivalentSegmentationType};
use crate::error::{Error, Result};
use broadcast_common::bits::{BitError, BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};

/// `message_type` for [`CompactTimeSignal`] (§8.3.3, Table 5).
pub const MESSAGE_TYPE_TIME_SIGNAL: u8 = 0x00;

/// `message_type` for [`CompactSpliceInsert`] (§8.3.3, Table 5).
pub const MESSAGE_TYPE_SPLICE_INSERT: u8 = 0x01;

/// Mask for the 33-bit `pts_time` / `duration` fields.
const MASK_33: u64 = broadcast_common::clock33::WRAP_33BIT - 1;

/// Mask for the 40-bit `segmentation_duration`.
const MASK_40: u64 = (1 << 40) - 1;

/// Bits of a `compact_time_signal()` up to and including
/// `segmentation_upid_length` (Table 6): 1+6+8+33+32+40+8+8.
const TS_FIXED_BITS: usize = 136;

/// Bits of a `compact_splice_insert()` fixed part through the `reserved` row
/// (Table 8): 1+6+8+33+32+33+16+8+8+1+6.
const SI_FIXED_BITS: usize = 152;

/// Width of the `DAS_descriptor_flag`-trailing `reserved` row (Table 8).
const SI_RESERVED_BITS: u32 = 6;

/// Width of the DAS-body `reserved` row (Table 8).
const DAS_RESERVED_BITS: u32 = 4;

/// All-ones value of the 6-bit reserved row (the SCTE 35 reserved-bit
/// convention, for hand-built values).
pub const SI_RESERVED_DEFAULT: u8 = 0x3F;

/// All-ones value of the 4-bit DAS reserved row.
pub const DAS_RESERVED_DEFAULT: u8 = 0x0F;

/// Map a bit-reader/writer failure to a crate error carrying byte counts.
fn bit_err(e: BitError, have: usize, what: &'static str) -> Error {
    match e {
        BitError::OutOfBounds {
            needed_bits,
            remaining_bits,
        } => Error::BufferTooShort {
            need: have + needed_bits.saturating_sub(remaining_bits).div_ceil(8),
            have,
            what,
        },
        _ => Error::InvalidValue {
            field: what,
            reason: "value does not fit its bit width",
        },
    }
}

/// `compact_SCTE_35()` — §8.3.3, Table 5.
///
/// A `message_type` byte selecting one of the two compact payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum CompactScte35 {
    /// `message_type == 0x00` — [`CompactTimeSignal`] (Table 6).
    TimeSignal(CompactTimeSignal),
    /// `message_type == 0x01` — [`CompactSpliceInsert`] (Table 8).
    SpliceInsert(CompactSpliceInsert),
}

impl CompactScte35 {
    /// The `message_type` byte for this payload.
    #[must_use]
    pub const fn message_type(&self) -> u8 {
        match self {
            Self::TimeSignal(_) => MESSAGE_TYPE_TIME_SIGNAL,
            Self::SpliceInsert(_) => MESSAGE_TYPE_SPLICE_INSERT,
        }
    }
}

impl<'a> Parse<'a> for CompactScte35 {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let &message_type = bytes.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "compact_SCTE_35 message_type",
        })?;
        let body = &bytes[1..];
        match message_type {
            MESSAGE_TYPE_TIME_SIGNAL => Ok(Self::TimeSignal(CompactTimeSignal::parse(body)?)),
            MESSAGE_TYPE_SPLICE_INSERT => Ok(Self::SpliceInsert(CompactSpliceInsert::parse(body)?)),
            _ => Err(Error::InvalidValue {
                field: "compact_SCTE_35.message_type",
                reason: "unknown / reserved message_type",
            }),
        }
    }
}

impl Serialize for CompactScte35 {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1 + match self {
            Self::TimeSignal(t) => t.serialized_len(),
            Self::SpliceInsert(s) => s.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.message_type();
        let written = match self {
            Self::TimeSignal(t) => t.serialize_into(&mut buf[1..])?,
            Self::SpliceInsert(s) => s.serialize_into(&mut buf[1..])?,
        };
        Ok(1 + written)
    }
}

/// `compact_time_signal()` — §8.3.3, Table 6.
///
/// Unless otherwise noted, every field shares the semantic of the corresponding
/// `time_signal()` / `segmentation_descriptor()` field, subject to the §5.3
/// constraints. The fields NOT carried here take the implied values of Table 7
/// (e.g. `segmentation_upid_type = 0x0F`, `segmentation_duration_flag = 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompactTimeSignal {
    /// `encrypted_packet` (1 bit). When set, an `E_CRC_32` trailer is present.
    pub encrypted_packet: bool,
    /// `encryption_algorithm` (6 bits, §11.3 Table 29).
    pub encryption_algorithm: u8,
    /// `cw_index` (8 bits).
    pub cw_index: u8,
    /// `pts_time` (33 bits, 90 kHz ticks).
    pub pts_time: u64,
    /// `segmentation_event_id` (32 bits).
    pub segmentation_event_id: u32,
    /// `segmentation_duration` (40 bits, 90 kHz ticks).
    pub segmentation_duration: u64,
    /// `segmentation_type_id` (8 bits, SCTE 35 Table 23).
    pub segmentation_type_id: u8,
    /// `segmentation_upid` (`N*8`) — the URI bytes (`segmentation_upid_type` is
    /// the implied `0x0F`, Table 7).
    pub segmentation_upid: Vec<u8>,
    /// `segments_num` (8 bits).
    pub segments_num: u8,
    /// `segments_expected` (8 bits).
    pub segments_expected: u8,
    /// `E_CRC_32` (32 bits, `rpchof`) — present iff `encrypted_packet`. Carried
    /// verbatim so the structure round-trips even though this crate does not
    /// decrypt.
    pub e_crc_32: Option<u32>,
}

impl<'a> Parse<'a> for CompactTimeSignal {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        const WHAT: &str = "compact_time_signal";
        let have = bytes.len();
        if have * 8 < TS_FIXED_BITS {
            return Err(Error::BufferTooShort {
                need: TS_FIXED_BITS / 8,
                have,
                what: "compact_time_signal fixed prefix",
            });
        }
        let e = |e| bit_err(e, have, WHAT);
        let mut r = BitReader::new(bytes);
        let encrypted_packet = r.read_bool().map_err(e)?;
        let encryption_algorithm = r.read_bits(6).map_err(e)? as u8;
        let cw_index = r.read_bits(8).map_err(e)? as u8;
        let pts_time = r.read_bits(33).map_err(e)?;
        let segmentation_event_id = r.read_bits(32).map_err(e)? as u32;
        let segmentation_duration = r.read_bits(40).map_err(e)?;
        let segmentation_type_id = r.read_bits(8).map_err(e)? as u8;
        let upid_len = r.read_bits(8).map_err(e)? as usize;
        let mut segmentation_upid = Vec::new();
        for _ in 0..upid_len {
            segmentation_upid.push(r.read_bits(8).map_err(e)? as u8);
        }
        let segments_num = r.read_bits(8).map_err(e)? as u8;
        let segments_expected = r.read_bits(8).map_err(e)? as u8;
        let e_crc_32 = if encrypted_packet {
            Some(r.read_bits(32).map_err(e)? as u32)
        } else {
            None
        };
        Ok(Self {
            encrypted_packet,
            encryption_algorithm,
            cw_index,
            pts_time,
            segmentation_event_id,
            segmentation_duration,
            segmentation_type_id,
            segmentation_upid,
            segments_num,
            segments_expected,
            e_crc_32,
        })
    }
}

impl Serialize for CompactTimeSignal {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        TS_FIXED_BITS / 8
            + self.segmentation_upid.len()
            + 2
            + if self.encrypted_packet { 4 } else { 0 }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        const WHAT: &str = "compact_time_signal";
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let upid_len = broadcast_common::len::fit_u8(
            self.segmentation_upid.len(),
            "compact_time_signal.segmentation_upid_length",
        )?;
        if self.pts_time > MASK_33 {
            return Err(Error::InvalidValue {
                field: "compact_time_signal.pts_time",
                reason: "exceeds 33-bit range",
            });
        }
        if self.segmentation_duration > MASK_40 {
            return Err(Error::InvalidValue {
                field: "compact_time_signal.segmentation_duration",
                reason: "exceeds 40-bit range",
            });
        }
        if self.encryption_algorithm > 0x3F {
            return Err(Error::InvalidValue {
                field: "compact_time_signal.encryption_algorithm",
                reason: "exceeds 6-bit range",
            });
        }
        let have = buf.len();
        let e = |e| bit_err(e, have, WHAT);
        let mut w = BitWriter::new(&mut buf[..need]);
        w.write_bool(self.encrypted_packet).map_err(e)?;
        w.write_bits(u64::from(self.encryption_algorithm), 6)
            .map_err(e)?;
        w.write_bits(u64::from(self.cw_index), 8).map_err(e)?;
        w.write_bits(self.pts_time, 33).map_err(e)?;
        w.write_bits(u64::from(self.segmentation_event_id), 32)
            .map_err(e)?;
        w.write_bits(self.segmentation_duration, 40).map_err(e)?;
        w.write_bits(u64::from(self.segmentation_type_id), 8)
            .map_err(e)?;
        w.write_bits(u64::from(upid_len), 8).map_err(e)?;
        for &b in &self.segmentation_upid {
            w.write_bits(u64::from(b), 8).map_err(e)?;
        }
        w.write_bits(u64::from(self.segments_num), 8).map_err(e)?;
        w.write_bits(u64::from(self.segments_expected), 8)
            .map_err(e)?;
        if self.encrypted_packet {
            w.write_bits(u64::from(self.e_crc_32.unwrap_or(0)), 32)
                .map_err(e)?;
        }
        debug_assert_eq!(w.bits_written(), need * 8);
        Ok(need)
    }
}

/// `compact_splice_insert()` — §8.3.3, Table 8.
///
/// Bit widths are those of Table 8 (see module docs): a 16-bit
/// `unique_program_id`, 8-bit `avail_num`/`avails_expected`, a 1-bit
/// `DAS_descriptor_flag` followed by a 6-bit `reserved` row and — when the flag
/// is set — an inline DAS-descriptor body. The implied SCTE 35 / DAS field
/// values are Tables 9 and 10.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompactSpliceInsert {
    /// `encrypted_packet` (1 bit).
    pub encrypted_packet: bool,
    /// `encryption_algorithm` (6 bits).
    pub encryption_algorithm: u8,
    /// `cw_index` (8 bits).
    pub cw_index: u8,
    /// `pts_time` (33 bits, 90 kHz ticks).
    pub pts_time: u64,
    /// `splice_event_id` (32 bits).
    pub splice_event_id: u32,
    /// `duration` (33 bits, 90 kHz ticks).
    pub duration: u64,
    /// `unique_program_id` (16 bits, §5.3.5.11).
    pub unique_program_id: u16,
    /// `avail_num` (8 bits).
    pub avail_num: u8,
    /// `avails_expected` (8 bits).
    pub avails_expected: u8,
    /// The 6-bit `reserved` row after `DAS_descriptor_flag`, preserved as
    /// parsed ([`SI_RESERVED_DEFAULT`] when hand-built).
    pub reserved: u8,
    /// Inline `DVB_DAS_descriptor` body, present iff `DAS_descriptor_flag == 1`
    /// (Table 10 implies `splice_descriptor_tag = 0xF0`, `identifier = "DVB_"`).
    pub das: Option<CompactDas>,
    /// `E_CRC_32` (32 bits) — present iff `encrypted_packet`. Carried verbatim.
    pub e_crc_32: Option<u32>,
}

/// The inline DAS-descriptor body inside a [`CompactSpliceInsert`] (Table 8 `if
/// (DAS_descriptor_flag)` block). Mirrors Table 1 of the `DVB_DAS_descriptor()`
/// minus the framing (tag / identifier are implied by Table 10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompactDas {
    /// `break_num` (8 bits).
    pub break_num: u8,
    /// `breaks_expected` (8 bits).
    pub breaks_expected: u8,
    /// `equivalent_segmentation_type` (4 bits, Table 2).
    pub equivalent_segmentation_type: EquivalentSegmentationType,
    /// The 4-bit `reserved` row after `equivalent_segmentation_type`,
    /// preserved as parsed ([`DAS_RESERVED_DEFAULT`] when hand-built).
    pub reserved: u8,
    /// `upid()` — variable-length URI bytes.
    pub upid: Vec<u8>,
}

impl<'a> Parse<'a> for CompactSpliceInsert {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_order(bytes, DasNibbleOrder::TABLE_8)
    }
}

impl CompactSpliceInsert {
    /// [`Parse::parse`] with an explicit [`DasNibbleOrder`] for the inline DAS
    /// `equivalent_segmentation_type`/`reserved` byte. `parse` uses
    /// [`DasNibbleOrder::TABLE_8`] (the order Table 8 prints); Table 1 prints
    /// the opposite order for the same body - see the AMBIGUITY note on
    /// [`DasNibbleOrder`].
    ///
    /// # Errors
    /// As [`Parse::parse`].
    pub fn parse_with_order(bytes: &[u8], order: DasNibbleOrder) -> Result<Self> {
        const WHAT: &str = "compact_splice_insert";
        let have = bytes.len();
        if have * 8 < SI_FIXED_BITS {
            return Err(Error::BufferTooShort {
                need: SI_FIXED_BITS / 8,
                have,
                what: "compact_splice_insert fixed fields",
            });
        }
        let e = |e| bit_err(e, have, WHAT);
        let mut r = BitReader::new(bytes);
        let encrypted_packet = r.read_bool().map_err(e)?;
        let encryption_algorithm = r.read_bits(6).map_err(e)? as u8;
        let cw_index = r.read_bits(8).map_err(e)? as u8;
        let pts_time = r.read_bits(33).map_err(e)?;
        let splice_event_id = r.read_bits(32).map_err(e)? as u32;
        let duration = r.read_bits(33).map_err(e)?;
        let unique_program_id = r.read_bits(16).map_err(e)? as u16;
        let avail_num = r.read_bits(8).map_err(e)? as u8;
        let avails_expected = r.read_bits(8).map_err(e)? as u8;
        let das_flag = r.read_bool().map_err(e)?;
        let reserved = r.read_bits(SI_RESERVED_BITS).map_err(e)? as u8;
        let das = if das_flag {
            let desc_len = r.read_bits(8).map_err(e)? as usize;
            // descriptor_length (N) counts break_num + breaks_expected + the
            // type/reserved byte (3) plus the upid.
            if desc_len < 3 {
                return Err(Error::InvalidValue {
                    field: "compact_splice_insert.descriptor_length",
                    reason: "DAS body length must be at least 3 (break_num, breaks_expected, type)",
                });
            }
            if r.bits_remaining() < desc_len * 8 {
                return Err(Error::LengthOverflow {
                    declared: desc_len,
                    available: r.bits_remaining() / 8,
                    what: "compact_splice_insert DAS body",
                });
            }
            let break_num = r.read_bits(8).map_err(e)? as u8;
            let breaks_expected = r.read_bits(8).map_err(e)? as u8;
            let first = r.read_bits(4).map_err(e)? as u8;
            let second = r.read_bits(DAS_RESERVED_BITS).map_err(e)? as u8;
            // AMBIGUITY: nibble order per `order` (Table 8 vs Table 1).
            let (type_nibble, das_reserved) = order.split((first << 4) | second);
            let equivalent_segmentation_type = EquivalentSegmentationType::from_bits(type_nibble);
            let mut upid = Vec::with_capacity(desc_len - 3);
            for _ in 0..desc_len - 3 {
                upid.push(r.read_bits(8).map_err(e)? as u8);
            }
            Some(CompactDas {
                break_num,
                breaks_expected,
                equivalent_segmentation_type,
                reserved: das_reserved,
                upid,
            })
        } else {
            None
        };
        let e_crc_32 = if encrypted_packet {
            Some(r.read_bits(32).map_err(e)? as u32)
        } else {
            None
        };
        Ok(Self {
            encrypted_packet,
            encryption_algorithm,
            cw_index,
            pts_time,
            splice_event_id,
            duration,
            unique_program_id,
            avail_num,
            avails_expected,
            reserved,
            das,
            e_crc_32,
        })
    }
}

impl Serialize for CompactSpliceInsert {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = SI_FIXED_BITS / 8;
        if let Some(das) = &self.das {
            n += 1 + 3 + das.upid.len();
        }
        if self.encrypted_packet {
            n += 4;
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_into_with_order(buf, DasNibbleOrder::TABLE_8)
    }
}

impl CompactSpliceInsert {
    /// [`Serialize::serialize_into`] with an explicit [`DasNibbleOrder`] for the
    /// inline DAS type/reserved byte (`serialize_into` uses
    /// [`DasNibbleOrder::TABLE_8`]; see the AMBIGUITY note on [`DasNibbleOrder`]).
    ///
    /// # Errors
    /// As [`Serialize::serialize_into`].
    pub fn serialize_into_with_order(
        &self,
        buf: &mut [u8],
        order: DasNibbleOrder,
    ) -> Result<usize> {
        const WHAT: &str = "compact_splice_insert";
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let desc_len = match &self.das {
            Some(das) => Some(broadcast_common::len::fit_u8(
                3 + das.upid.len(),
                "compact_splice_insert.das.descriptor_length",
            )?),
            None => None,
        };
        if self.pts_time > MASK_33 {
            return Err(Error::InvalidValue {
                field: "compact_splice_insert.pts_time",
                reason: "exceeds 33-bit range",
            });
        }
        if self.duration > MASK_33 {
            return Err(Error::InvalidValue {
                field: "compact_splice_insert.duration",
                reason: "exceeds 33-bit range",
            });
        }
        let have = buf.len();
        let e = |e| bit_err(e, have, WHAT);
        let mut w = BitWriter::new(&mut buf[..need]);
        w.write_bool(self.encrypted_packet).map_err(e)?;
        w.write_bits(u64::from(self.encryption_algorithm), 6)
            .map_err(e)?;
        w.write_bits(u64::from(self.cw_index), 8).map_err(e)?;
        w.write_bits(self.pts_time, 33).map_err(e)?;
        w.write_bits(u64::from(self.splice_event_id), 32)
            .map_err(e)?;
        w.write_bits(self.duration, 33).map_err(e)?;
        w.write_bits(u64::from(self.unique_program_id), 16)
            .map_err(e)?;
        w.write_bits(u64::from(self.avail_num), 8).map_err(e)?;
        w.write_bits(u64::from(self.avails_expected), 8)
            .map_err(e)?;
        w.write_bool(self.das.is_some()).map_err(e)?;
        w.write_bits(u64::from(self.reserved), SI_RESERVED_BITS)
            .map_err(e)?;
        if let (Some(das), Some(desc_len)) = (&self.das, desc_len) {
            w.write_bits(u64::from(desc_len), 8).map_err(e)?;
            w.write_bits(u64::from(das.break_num), 8).map_err(e)?;
            w.write_bits(u64::from(das.breaks_expected), 8).map_err(e)?;
            // AMBIGUITY: nibble order per `order` (Table 8 vs Table 1).
            let byte = order.join(das.equivalent_segmentation_type.bits(), das.reserved);
            w.write_bits(u64::from(byte >> 4), 4).map_err(e)?;
            w.write_bits(u64::from(byte & 0x0F), DAS_RESERVED_BITS)
                .map_err(e)?;
            for &b in &das.upid {
                w.write_bits(u64::from(b), 8).map_err(e)?;
            }
        }
        if self.encrypted_packet {
            w.write_bits(u64::from(self.e_crc_32.unwrap_or(0)), 32)
                .map_err(e)?;
        }
        debug_assert_eq!(w.bits_written(), need * 8);
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent oracle: pack `(value, width)` fields MSB-first through a
    /// string of `0`/`1` characters (deliberately not `BitWriter`), so a layout
    /// bug in the implementation cannot be mirrored by the expected bytes.
    fn pack(fields: &[(u64, u32)]) -> Vec<u8> {
        let mut s = alloc::string::String::new();
        for &(v, w) in fields {
            for i in (0..w).rev() {
                s.push(if (v >> i) & 1 == 1 { '1' } else { '0' });
            }
        }
        assert_eq!(s.len() % 8, 0, "layout must be byte aligned");
        s.as_bytes()
            .chunks(8)
            .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b == b'1')))
            .collect()
    }

    fn sample_time_signal() -> CompactTimeSignal {
        CompactTimeSignal {
            encrypted_packet: false,
            encryption_algorithm: 0,
            cw_index: 0,
            pts_time: 0x1_2345_6789 & MASK_33,
            segmentation_event_id: 0xDEAD_BEEF,
            segmentation_duration: 0x00_0123_4567,
            segmentation_type_id: 0x34,
            segmentation_upid: b"urn:com.broadcaster:112210F47DE98115".to_vec(),
            segments_num: 1,
            segments_expected: 2,
            e_crc_32: None,
        }
    }

    #[test]
    fn time_signal_round_trip_and_dispatch() {
        let ts = sample_time_signal();
        let compact = CompactScte35::TimeSignal(ts.clone());
        let bytes = compact.try_to_bytes().unwrap();
        assert_eq!(bytes[0], MESSAGE_TYPE_TIME_SIGNAL);
        let back = CompactScte35::parse(&bytes).unwrap();
        assert_eq!(compact, back);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }

    /// Table 6 is packed with no padding: `cw_index` straddles bytes 0/1 and
    /// `pts_time` straddles bytes 1/2 (the pre-fix layout inserted reserved bits
    /// the table does not list and put the fields on different offsets).
    #[test]
    fn time_signal_matches_the_spec_table_packing() {
        let mut ts = sample_time_signal();
        ts.encrypted_packet = true;
        ts.encryption_algorithm = 0x15;
        ts.cw_index = 0xA5;
        ts.segmentation_upid = b"AB".to_vec();
        ts.e_crc_32 = Some(0x1122_3344);
        let bytes = ts.try_to_bytes().unwrap();
        // enc(1)=1, alg(6)=010101, cw[7]=1 -> 1010_1011
        assert_eq!(bytes[0], 0xAB);
        // cw[6:0]=0100101, pts[32]=1 -> 0100_1011
        assert_eq!(bytes[1], 0x4B);
        assert_eq!(&bytes[2..6], &[0x23, 0x45, 0x67, 0x89]);
        assert_eq!(&bytes[6..10], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(&bytes[10..15], &[0x00, 0x01, 0x23, 0x45, 0x67]);
        assert_eq!(bytes[15], 0x34);
        assert_eq!(bytes[16], 2);
        assert_eq!(&bytes[17..19], b"AB");
        assert_eq!(&bytes[19..21], &[1, 2]);
        assert_eq!(&bytes[21..25], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(bytes.len(), 25);
        let back = CompactTimeSignal::parse(&bytes).unwrap();
        assert_eq!(ts, back);
    }

    #[test]
    fn time_signal_pts_over_33_bits_is_rejected_not_masked() {
        let mut ts = sample_time_signal();
        ts.pts_time = 1u64 << 33;
        assert!(ts.try_to_bytes().is_err());
    }

    #[test]
    fn time_signal_duration_over_40_bits_is_rejected_not_masked() {
        let mut ts = sample_time_signal();
        ts.segmentation_duration = 1u64 << 40;
        assert!(ts.try_to_bytes().is_err());
    }

    #[test]
    fn time_signal_truncated_is_an_error() {
        let bytes = sample_time_signal().try_to_bytes().unwrap();
        for cut in 0..bytes.len() {
            assert!(
                CompactTimeSignal::parse(&bytes[..cut]).is_err(),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn time_signal_field_mutation_bites() {
        let a = sample_time_signal();
        let mut b = a.clone();
        b.segmentation_type_id = 0x36;
        assert_ne!(a.try_to_bytes().unwrap(), b.try_to_bytes().unwrap());
        let mut c = a.clone();
        c.pts_time = a.pts_time ^ 1;
        assert_ne!(a.try_to_bytes().unwrap(), c.try_to_bytes().unwrap());
    }

    fn sample_splice_insert(das: Option<CompactDas>) -> CompactSpliceInsert {
        CompactSpliceInsert {
            encrypted_packet: false,
            encryption_algorithm: 0,
            cw_index: 0,
            pts_time: 0x0_1234_5678,
            splice_event_id: 0x0102_0304,
            duration: 0x0_0009_0000,
            unique_program_id: 0xABCD,
            avail_num: 1,
            avails_expected: 3,
            reserved: SI_RESERVED_DEFAULT,
            das,
            e_crc_32: None,
        }
    }

    #[test]
    fn splice_insert_pts_over_33_bits_is_rejected_not_masked() {
        let mut si = sample_splice_insert(None);
        si.pts_time = 1u64 << 33;
        assert!(si.try_to_bytes().is_err());
    }

    #[test]
    fn splice_insert_duration_over_33_bits_is_rejected_not_masked() {
        let mut si = sample_splice_insert(None);
        si.duration = 1u64 << 33;
        assert!(si.try_to_bytes().is_err());
    }

    /// Table 8 against the independent packer: 152 fixed bits, the
    /// `DAS_descriptor_flag` followed by 6 reserved bits (not 7).
    #[test]
    fn splice_insert_matches_the_spec_table_packing() {
        let das = CompactDas {
            break_num: 2,
            breaks_expected: 5,
            equivalent_segmentation_type:
                EquivalentSegmentationType::DistributorPlacementOpportunity,
            reserved: 0x0A,
            upid: b"urn:tv.acme:B637643".to_vec(),
        };
        let mut si = sample_splice_insert(Some(das));
        si.encrypted_packet = true;
        si.encryption_algorithm = 0x2A;
        si.cw_index = 0x5C;
        si.reserved = 0x15;
        si.e_crc_32 = Some(0xCAFE_F00D);
        let mut fields = alloc::vec![
            (1, 1),
            (0x2A, 6),
            (0x5C, 8),
            (0x0_1234_5678, 33),
            (0x0102_0304, 32),
            (0x9_0000, 33),
            (0xABCD, 16),
            (1, 8),
            (3, 8),
            (1, 1),
            (0x15, 6),
            (3 + 19, 8),
            (2, 8),
            (5, 8),
            (
                u64::from(EquivalentSegmentationType::DistributorPlacementOpportunity.bits()),
                4
            ),
            (0x0A, 4),
        ];
        for &b in b"urn:tv.acme:B637643" {
            fields.push((u64::from(b), 8));
        }
        fields.push((0xCAFE_F00D, 32));
        assert_eq!(si.try_to_bytes().unwrap(), pack(&fields));
        let back = CompactSpliceInsert::parse(&si.try_to_bytes().unwrap()).unwrap();
        assert_eq!(si, back);
    }

    #[test]
    fn splice_insert_round_trip_no_das() {
        let si = sample_splice_insert(None);
        let compact = CompactScte35::SpliceInsert(si.clone());
        let bytes = compact.try_to_bytes().unwrap();
        assert_eq!(bytes[0], MESSAGE_TYPE_SPLICE_INSERT);
        assert_eq!(bytes.len(), 1 + 19);
        let back = CompactScte35::parse(&bytes).unwrap();
        assert_eq!(compact, back);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }

    /// The reserved rows are typed fields: whatever a sender put there
    /// survives parse -> serialize.
    #[test]
    fn reserved_bits_survive_a_round_trip() {
        let das = CompactDas {
            break_num: 0,
            breaks_expected: 0,
            equivalent_segmentation_type: EquivalentSegmentationType::NoEquivalent,
            reserved: 0x00,
            upid: Vec::new(),
        };
        let mut si = sample_splice_insert(Some(das));
        si.reserved = 0x00;
        let bytes = si.try_to_bytes().unwrap();
        let back = CompactSpliceInsert::parse(&bytes).unwrap();
        assert_eq!(back.reserved, 0);
        assert_eq!(back.das.as_ref().unwrap().reserved, 0);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn splice_insert_field_mutation_bites() {
        let a = sample_splice_insert(None);
        let mut b = a.clone();
        b.unique_program_id = 0x0001;
        assert_ne!(a.try_to_bytes().unwrap(), b.try_to_bytes().unwrap());
        let c = sample_splice_insert(Some(CompactDas {
            break_num: 0,
            breaks_expected: 0,
            equivalent_segmentation_type: EquivalentSegmentationType::NoEquivalent,
            reserved: DAS_RESERVED_DEFAULT,
            upid: Vec::new(),
        }));
        assert_ne!(a.try_to_bytes().unwrap(), c.try_to_bytes().unwrap());
    }

    #[test]
    fn rejects_unknown_message_type() {
        let bytes = [0x7F, 0, 0, 0, 0];
        assert!(matches!(
            CompactScte35::parse(&bytes).unwrap_err(),
            Error::InvalidValue { .. }
        ));
    }

    #[test]
    fn time_signal_upid_overflow_guard_fires() {
        let mut ts = sample_time_signal();
        ts.segmentation_upid = vec![0u8; 256];
        let err = ts
            .serialize_into(&mut vec![0u8; ts.serialized_len()])
            .unwrap_err();
        assert!(matches!(err, Error::FieldOverflow(_)), "got {err:?}");
    }
}
