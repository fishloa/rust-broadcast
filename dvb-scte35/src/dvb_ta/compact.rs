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
//! # Bit-width sourcing
//!
//! The PDF render of Tables 6 and 8 is vertically mis-registered; this module
//! implements the **reconstructed ("likely") widths** the transcription cross-
//! checks against §5.3.5.11 and the `DVB_DAS_descriptor()` (Table 1):
//! `unique_program_id` is 16-bit, `avail_num`/`avails_expected` 8-bit,
//! `DAS_descriptor_flag` 1-bit, `equivalent_segmentation_type` 4-bit, and
//! `E_CRC_32` a 32-bit CRC. See `docs/dvb_ta/compact-scte35.md`.

use alloc::vec::Vec;

use crate::dvb_ta::das_descriptor::EquivalentSegmentationType;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `message_type` for [`CompactTimeSignal`] (§8.3.3, Table 5).
pub const MESSAGE_TYPE_TIME_SIGNAL: u8 = 0x00;

/// `message_type` for [`CompactSpliceInsert`] (§8.3.3, Table 5).
pub const MESSAGE_TYPE_SPLICE_INSERT: u8 = 0x01;

/// Mask for the 33-bit `pts_time` / `duration` fields.
const MASK_33: u64 = (1 << 33) - 1;

/// Read a big-endian 33-bit field starting at `bits[0]`'s low bit being the top
/// of `pts_time`. Helper for the compact bit layouts: the 33-bit field occupies
/// 1 low bit of `first` plus the next four bytes.
fn read_33(first_low_bit: u8, next4: &[u8]) -> u64 {
    (u64::from(first_low_bit & 0x01) << 32)
        | (u64::from(next4[0]) << 24)
        | (u64::from(next4[1]) << 16)
        | (u64::from(next4[2]) << 8)
        | u64::from(next4[3])
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

/// Mask for the 40-bit `segmentation_duration`.
const MASK_40: u64 = (1 << 40) - 1;

impl<'a> Parse<'a> for CompactTimeSignal {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Fixed prefix to segmentation_upid_length (N):
        //   byte0:  encrypted(1) + encryption_algorithm(6) + top bit of nothing
        //           (the 1+6 = 7 bits, low bit unused/reserved here)
        //   byte1:  cw_index(8)
        //   byte2:  reserved(7) + top pts_time bit (we store pts top bit in low)
        //   byte3..6: pts_time low 32 bits
        //   byte7..10: segmentation_event_id(32)
        //   byte11..15: segmentation_duration(40)
        //   byte16: segmentation_type_id(8)
        //   byte17: segmentation_upid_length N(8)
        //   byte18..18+N: segmentation_upid
        //   then segments_num(8), segments_expected(8)
        //   then optional E_CRC_32(32)
        const FIXED_PREFIX: usize = 18;
        if bytes.len() < FIXED_PREFIX {
            return Err(Error::BufferTooShort {
                need: FIXED_PREFIX,
                have: bytes.len(),
                what: "compact_time_signal fixed prefix",
            });
        }
        let encrypted_packet = bytes[0] & 0x80 != 0;
        let encryption_algorithm = (bytes[0] >> 1) & 0x3F;
        let cw_index = bytes[1];
        let pts_time = read_33(bytes[2], &bytes[3..7]);
        let segmentation_event_id = u32::from_be_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]);
        let segmentation_duration = (u64::from(bytes[11]) << 32)
            | (u64::from(bytes[12]) << 24)
            | (u64::from(bytes[13]) << 16)
            | (u64::from(bytes[14]) << 8)
            | u64::from(bytes[15]);
        let segmentation_type_id = bytes[16];
        let upid_len = bytes[17] as usize;
        let upid_end = FIXED_PREFIX + upid_len;
        // need upid + segments_num + segments_expected
        if bytes.len() < upid_end + 2 {
            return Err(Error::BufferTooShort {
                need: upid_end + 2,
                have: bytes.len(),
                what: "compact_time_signal upid + segments",
            });
        }
        let segmentation_upid = bytes[FIXED_PREFIX..upid_end].to_vec();
        let segments_num = bytes[upid_end];
        let segments_expected = bytes[upid_end + 1];
        let mut pos = upid_end + 2;
        let e_crc_32 = if encrypted_packet {
            if bytes.len() < pos + 4 {
                return Err(Error::BufferTooShort {
                    need: pos + 4,
                    have: bytes.len(),
                    what: "compact_time_signal E_CRC_32",
                });
            }
            let v =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
            pos += 4;
            let _ = pos;
            Some(v)
        } else {
            None
        };
        Ok(Self {
            encrypted_packet,
            encryption_algorithm,
            cw_index,
            pts_time: pts_time & MASK_33,
            segmentation_event_id,
            segmentation_duration: segmentation_duration & MASK_40,
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
        18 + self.segmentation_upid.len() + 2 + if self.encrypted_packet { 4 } else { 0 }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if self.segmentation_upid.len() > u8::MAX as usize {
            return Err(Error::InvalidValue {
                field: "compact_time_signal.segmentation_upid",
                reason: "length exceeds 8-bit segmentation_upid_length",
            });
        }
        // byte0: encrypted(1) + encryption_algorithm(6) + reserved low bit (1).
        buf[0] = (u8::from(self.encrypted_packet) << 7)
            | ((self.encryption_algorithm & 0x3F) << 1)
            | 0x01;
        buf[1] = self.cw_index;
        let pts = self.pts_time & MASK_33;
        // byte2: 7 reserved bits = 1, top pts bit.
        buf[2] = 0xFE | ((pts >> 32) as u8 & 0x01);
        buf[3] = (pts >> 24) as u8;
        buf[4] = (pts >> 16) as u8;
        buf[5] = (pts >> 8) as u8;
        buf[6] = pts as u8;
        buf[7..11].copy_from_slice(&self.segmentation_event_id.to_be_bytes());
        let dur = self.segmentation_duration & MASK_40;
        buf[11] = (dur >> 32) as u8;
        buf[12] = (dur >> 24) as u8;
        buf[13] = (dur >> 16) as u8;
        buf[14] = (dur >> 8) as u8;
        buf[15] = dur as u8;
        buf[16] = self.segmentation_type_id;
        buf[17] = self.segmentation_upid.len() as u8;
        let upid_end = 18 + self.segmentation_upid.len();
        buf[18..upid_end].copy_from_slice(&self.segmentation_upid);
        buf[upid_end] = self.segments_num;
        buf[upid_end + 1] = self.segments_expected;
        let mut pos = upid_end + 2;
        if self.encrypted_packet {
            let crc = self.e_crc_32.unwrap_or(0);
            buf[pos..pos + 4].copy_from_slice(&crc.to_be_bytes());
            pos += 4;
        }
        debug_assert_eq!(pos, need);
        Ok(need)
    }
}

/// `compact_splice_insert()` — §8.3.3, Table 8.
///
/// Uses the reconstructed ("likely") bit widths (see module docs): a 16-bit
/// `unique_program_id`, 8-bit `avail_num`/`avails_expected`, a 1-bit
/// `DAS_descriptor_flag`, and — when the flag is set — an inline DAS-descriptor
/// body (`break_num`, `breaks_expected`, 4-bit `equivalent_segmentation_type`,
/// and the variable `upid`). The implied SCTE 35 / DAS field values are Tables 9
/// and 10.
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
    /// `upid()` — variable-length URI bytes.
    pub upid: Vec<u8>,
}

impl<'a> Parse<'a> for CompactSpliceInsert {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Layout (likely widths):
        //   byte0:  encrypted(1) + encryption_algorithm(6) + reserved(1)
        //   byte1:  cw_index(8)
        //   byte2:  reserved(7) + top pts bit
        //   byte3..6: pts_time low 32
        //   byte7..10: splice_event_id(32)
        //   byte11: reserved(7) + top duration bit
        //   byte12..15: duration low 32
        //   byte16..17: unique_program_id(16)
        //   byte18: avail_num(8)
        //   byte19: avails_expected(8)
        //   byte20: DAS_descriptor_flag(1) + reserved(7)
        //   if DAS_descriptor_flag:
        //     descriptor_length N(8)   [length of the DAS body that follows]
        //     break_num(8), breaks_expected(8)
        //     reserved(4) + equivalent_segmentation_type(4)
        //     upid (descriptor_length - 3 bytes)
        //   if encrypted: E_CRC_32(32)
        const FIXED: usize = 21;
        if bytes.len() < FIXED {
            return Err(Error::BufferTooShort {
                need: FIXED,
                have: bytes.len(),
                what: "compact_splice_insert fixed fields",
            });
        }
        let encrypted_packet = bytes[0] & 0x80 != 0;
        let encryption_algorithm = (bytes[0] >> 1) & 0x3F;
        let cw_index = bytes[1];
        let pts_time = read_33(bytes[2], &bytes[3..7]) & MASK_33;
        let splice_event_id = u32::from_be_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]);
        let duration = read_33(bytes[11], &bytes[12..16]) & MASK_33;
        let unique_program_id = u16::from_be_bytes([bytes[16], bytes[17]]);
        let avail_num = bytes[18];
        let avails_expected = bytes[19];
        let das_flag = bytes[20] & 0x80 != 0;
        let mut pos = FIXED;
        let das = if das_flag {
            // descriptor_length (1) + break_num (1) + breaks_expected (1)
            // + reserved/eqseg (1) = 4 bytes minimum.
            if bytes.len() < pos + 4 {
                return Err(Error::BufferTooShort {
                    need: pos + 4,
                    have: bytes.len(),
                    what: "compact_splice_insert DAS body",
                });
            }
            let desc_len = bytes[pos] as usize; // length of break_num..upid end
            pos += 1;
            // desc_len counts break_num(1)+breaks_expected(1)+nibble(1)+upid.
            if desc_len < 3 {
                return Err(Error::InvalidValue {
                    field: "compact_splice_insert.descriptor_length",
                    reason: "DAS body length must be at least 3 (break_num, breaks_expected, type)",
                });
            }
            if bytes.len() < pos + desc_len {
                return Err(Error::LengthOverflow {
                    declared: desc_len,
                    available: bytes.len().saturating_sub(pos),
                    what: "compact_splice_insert DAS body",
                });
            }
            let break_num = bytes[pos];
            let breaks_expected = bytes[pos + 1];
            let equivalent_segmentation_type =
                EquivalentSegmentationType::from_bits(bytes[pos + 2] & 0x0F);
            let upid = bytes[pos + 3..pos + desc_len].to_vec();
            pos += desc_len;
            Some(CompactDas {
                break_num,
                breaks_expected,
                equivalent_segmentation_type,
                upid,
            })
        } else {
            None
        };
        let e_crc_32 = if encrypted_packet {
            if bytes.len() < pos + 4 {
                return Err(Error::BufferTooShort {
                    need: pos + 4,
                    have: bytes.len(),
                    what: "compact_splice_insert E_CRC_32",
                });
            }
            Some(u32::from_be_bytes([
                bytes[pos],
                bytes[pos + 1],
                bytes[pos + 2],
                bytes[pos + 3],
            ]))
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
            das,
            e_crc_32,
        })
    }
}

impl Serialize for CompactSpliceInsert {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = 21;
        if let Some(das) = &self.das {
            n += 1 + 3 + das.upid.len();
        }
        if self.encrypted_packet {
            n += 4;
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if let Some(das) = &self.das {
            if das.upid.len() + 3 > u8::MAX as usize {
                return Err(Error::InvalidValue {
                    field: "compact_splice_insert.das.upid",
                    reason: "DAS body length exceeds 8-bit descriptor_length",
                });
            }
        }
        buf[0] = (u8::from(self.encrypted_packet) << 7)
            | ((self.encryption_algorithm & 0x3F) << 1)
            | 0x01;
        buf[1] = self.cw_index;
        let pts = self.pts_time & MASK_33;
        buf[2] = 0xFE | ((pts >> 32) as u8 & 0x01);
        buf[3] = (pts >> 24) as u8;
        buf[4] = (pts >> 16) as u8;
        buf[5] = (pts >> 8) as u8;
        buf[6] = pts as u8;
        buf[7..11].copy_from_slice(&self.splice_event_id.to_be_bytes());
        let dur = self.duration & MASK_33;
        buf[11] = 0xFE | ((dur >> 32) as u8 & 0x01);
        buf[12] = (dur >> 24) as u8;
        buf[13] = (dur >> 16) as u8;
        buf[14] = (dur >> 8) as u8;
        buf[15] = dur as u8;
        buf[16..18].copy_from_slice(&self.unique_program_id.to_be_bytes());
        buf[18] = self.avail_num;
        buf[19] = self.avails_expected;
        // DAS_descriptor_flag (1) + 7 reserved bits = 1.
        buf[20] = (u8::from(self.das.is_some()) << 7) | 0x7F;
        let mut pos = 21;
        if let Some(das) = &self.das {
            let desc_len = 3 + das.upid.len();
            buf[pos] = desc_len as u8;
            buf[pos + 1] = das.break_num;
            buf[pos + 2] = das.breaks_expected;
            buf[pos + 3] = 0xF0 | das.equivalent_segmentation_type.bits();
            buf[pos + 4..pos + 4 + das.upid.len()].copy_from_slice(&das.upid);
            pos += 1 + desc_len;
        }
        if self.encrypted_packet {
            let crc = self.e_crc_32.unwrap_or(0);
            buf[pos..pos + 4].copy_from_slice(&crc.to_be_bytes());
            pos += 4;
        }
        debug_assert_eq!(pos, need);
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let bytes = compact.to_bytes();
        assert_eq!(bytes[0], MESSAGE_TYPE_TIME_SIGNAL);
        let back = CompactScte35::parse(&bytes).unwrap();
        assert_eq!(compact, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn time_signal_hand_computed_prefix() {
        let mut ts = sample_time_signal();
        ts.segmentation_upid = b"AB".to_vec(); // N = 2
        let bytes = ts.to_bytes();
        // byte0: encrypted=0, algo=0, reserved low bit=1 -> 0x01
        assert_eq!(bytes[0], 0x01);
        // pts_time = 0x123456789 & 2^33-1 = 0x123456789 (33 bits). top bit = 1.
        assert_eq!(bytes[2], 0xFE | 0x01);
        assert_eq!(&bytes[3..7], &[0x23, 0x45, 0x67, 0x89]);
        assert_eq!(&bytes[7..11], &[0xDE, 0xAD, 0xBE, 0xEF]);
        // duration 0x0001234567 over 5 bytes
        assert_eq!(&bytes[11..16], &[0x00, 0x01, 0x23, 0x45, 0x67]);
        assert_eq!(bytes[16], 0x34);
        assert_eq!(bytes[17], 2); // N
        assert_eq!(&bytes[18..20], b"AB");
        assert_eq!(bytes[20], 1); // segments_num
        assert_eq!(bytes[21], 2); // segments_expected
        assert_eq!(bytes.len(), 22);
    }

    #[test]
    fn time_signal_encrypted_carries_e_crc() {
        let mut ts = sample_time_signal();
        ts.encrypted_packet = true;
        ts.e_crc_32 = Some(0x1122_3344);
        let bytes = ts.to_bytes();
        assert_eq!(bytes[0] & 0x80, 0x80);
        let back = CompactTimeSignal::parse(&bytes).unwrap();
        assert_eq!(ts, back);
        assert_eq!(back.e_crc_32, Some(0x1122_3344));
    }

    #[test]
    fn time_signal_field_mutation_bites() {
        let a = sample_time_signal();
        let mut b = a.clone();
        b.segmentation_type_id = 0x36;
        assert_ne!(a.to_bytes(), b.to_bytes());
        let mut c = a.clone();
        c.pts_time = a.pts_time ^ 1;
        assert_ne!(a.to_bytes(), c.to_bytes());
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
            das,
            e_crc_32: None,
        }
    }

    #[test]
    fn splice_insert_round_trip_no_das() {
        let si = sample_splice_insert(None);
        let compact = CompactScte35::SpliceInsert(si.clone());
        let bytes = compact.to_bytes();
        assert_eq!(bytes[0], MESSAGE_TYPE_SPLICE_INSERT);
        // DAS flag clear.
        assert_eq!(bytes[21] & 0x80, 0x00);
        let back = CompactScte35::parse(&bytes).unwrap();
        assert_eq!(compact, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn splice_insert_round_trip_with_das() {
        let das = CompactDas {
            break_num: 2,
            breaks_expected: 5,
            equivalent_segmentation_type:
                EquivalentSegmentationType::DistributorPlacementOpportunity,
            upid: b"urn:tv.acme:B637643".to_vec(),
        };
        let si = sample_splice_insert(Some(das));
        let bytes = si.to_bytes();
        // DAS flag set in the body (offset 20 within the splice-insert body).
        assert_eq!(bytes[20] & 0x80, 0x80);
        // descriptor_length = 3 + upid.len()
        assert_eq!(bytes[21] as usize, 3 + b"urn:tv.acme:B637643".len());
        // reserved|eqseg nibble = 0xF1 (DPO = 0x1)
        assert_eq!(bytes[24], 0xF1);
        let back = CompactSpliceInsert::parse(&bytes).unwrap();
        assert_eq!(si, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn splice_insert_field_mutation_bites() {
        let a = sample_splice_insert(None);
        let mut b = a.clone();
        b.unique_program_id = 0x0001;
        assert_ne!(a.to_bytes(), b.to_bytes());
        // Adding a DAS body changes the wire.
        let c = sample_splice_insert(Some(CompactDas {
            break_num: 0,
            breaks_expected: 0,
            equivalent_segmentation_type: EquivalentSegmentationType::NoEquivalent,
            upid: Vec::new(),
        }));
        assert_ne!(a.to_bytes(), c.to_bytes());
    }

    #[test]
    fn rejects_unknown_message_type() {
        let bytes = [0x7F, 0, 0, 0, 0];
        assert!(matches!(
            CompactScte35::parse(&bytes).unwrap_err(),
            Error::InvalidValue { .. }
        ));
    }
}
