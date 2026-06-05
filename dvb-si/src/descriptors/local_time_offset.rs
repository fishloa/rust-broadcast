//! Local Time Offset Descriptor — ETSI EN 300 468 §6.2.20 (tag 0x58).
//!
//! Carried inside the TOT (Time Offset Table) on PID 0x0014. Signals per-
//! country offsets from UTC plus any upcoming DST transition.

use crate::error::{Error, Result};
use crate::text::LangCode;
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for local_time_offset_descriptor.
pub const TAG: u8 = 0x58;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 13;
const POLARITY_MASK: u8 = 0x01;
const REGION_ID_MASK: u8 = 0xFC;
const RESERVED_BIT_MASK: u8 = 0x02;

/// One per-country offset entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalTimeOffsetEntry {
    /// ISO 3166 alpha country code.
    pub country_code: LangCode,
    /// 6-bit country_region_id for sub-national regions.
    pub country_region_id: u8,
    /// Polarity: false = offset is positive (local = UTC + offset),
    /// true = offset is negative (local = UTC − offset).
    pub local_time_offset_negative: bool,
    /// 16-bit BCD HHMM local time offset.
    pub local_time_offset_bcd: u16,
    /// 40-bit MJD+UTC raw bytes of the DST/offset transition moment.
    pub time_of_change_raw: [u8; 5],
    /// 16-bit BCD HHMM next offset (applied after `time_of_change`).
    pub next_time_offset_bcd: u16,
}

/// Local Time Offset Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalTimeOffsetDescriptor {
    /// Entries in wire order.
    pub entries: Vec<LocalTimeOffsetEntry>,
}

impl<'a> Parse<'a> for LocalTimeOffsetDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "LocalTimeOffsetDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for local_time_offset_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 13",
            });
        }
        let body_start = HEADER_LEN;
        let body_end = body_start + length;
        if bytes.len() < body_end {
            return Err(Error::BufferTooShort {
                need: body_end,
                have: bytes.len(),
                what: "LocalTimeOffsetDescriptor body",
            });
        }
        let mut entries = Vec::with_capacity(length / ENTRY_LEN);
        let mut offset = body_start;
        while offset < body_end {
            let country_code = LangCode([bytes[offset], bytes[offset + 1], bytes[offset + 2]]);
            let flags = bytes[offset + 3];
            // The reserved bit is ignored on parse (EN 300 468 §5.1: decoders
            // shall ignore reserved bits).
            let country_region_id = (flags & REGION_ID_MASK) >> 2;
            let local_time_offset_negative = flags & POLARITY_MASK != 0;
            let local_time_offset_bcd = u16::from_be_bytes([bytes[offset + 4], bytes[offset + 5]]);
            let mut time_of_change_raw = [0u8; 5];
            time_of_change_raw.copy_from_slice(&bytes[offset + 6..offset + 11]);
            let next_time_offset_bcd = u16::from_be_bytes([bytes[offset + 11], bytes[offset + 12]]);
            entries.push(LocalTimeOffsetEntry {
                country_code,
                country_region_id,
                local_time_offset_negative,
                local_time_offset_bcd,
                time_of_change_raw,
                next_time_offset_bcd,
            });
            offset += ENTRY_LEN;
        }
        Ok(Self { entries })
    }
}

impl Serialize for LocalTimeOffsetDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        let mut offset = HEADER_LEN;
        for entry in &self.entries {
            buf[offset..offset + 3].copy_from_slice(&entry.country_code.0);
            let flags = ((entry.country_region_id << 2) & REGION_ID_MASK)
                | RESERVED_BIT_MASK
                | if entry.local_time_offset_negative {
                    POLARITY_MASK
                } else {
                    0
                };
            buf[offset + 3] = flags;
            buf[offset + 4..offset + 6].copy_from_slice(&entry.local_time_offset_bcd.to_be_bytes());
            buf[offset + 6..offset + 11].copy_from_slice(&entry.time_of_change_raw);
            buf[offset + 11..offset + 13]
                .copy_from_slice(&entry.next_time_offset_bcd.to_be_bytes());
            offset += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for LocalTimeOffsetDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for LocalTimeOffsetDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "LOCAL_TIME_OFFSET";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].country_code, LangCode([0x46, 0x52, 0x41]));
        assert_eq!(d.entries[0].country_region_id, 0);
        assert!(!d.entries[0].local_time_offset_negative);
        assert_eq!(d.entries[0].local_time_offset_bcd, 0x0100);
        assert_eq!(
            d.entries[0].time_of_change_raw,
            [0xAB, 0xCD, 0xEF, 0x12, 0x34]
        );
        assert_eq!(d.entries[0].next_time_offset_bcd, 0x0200);
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [
            TAG, 26, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
            0x47, 0x42, 0x52, 0x06, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x01, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].country_code, LangCode([0x46, 0x52, 0x41]));
        assert_eq!(d.entries[1].country_code, LangCode([0x47, 0x42, 0x52]));
    }

    #[test]
    fn parse_extracts_polarity_negative() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x03, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert!(d.entries[0].local_time_offset_negative);
    }

    #[test]
    fn parse_extracts_country_region_id() {
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x1A, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries[0].country_region_id, 6);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = LocalTimeOffsetDescriptor::parse(&[
            0x59, 13, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ])
        .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x59, .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_13() {
        let bytes = [
            TAG, 14, 0x46, 0x52, 0x41, 0x02, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
            0xFF,
        ];
        let err = LocalTimeOffsetDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_ignores_reserved_bit_not_set() {
        // Reserved bit clear must be ignored, not rejected (EN 300 468 §5.1).
        let bytes = [
            TAG, 13, 0x46, 0x52, 0x41, 0x00, 0x01, 0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x02, 0x00,
        ];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert!(!d.entries[0].local_time_offset_negative);
    }

    #[test]
    fn serialize_round_trip() {
        let d = LocalTimeOffsetDescriptor {
            entries: vec![LocalTimeOffsetEntry {
                country_code: LangCode([0x46, 0x52, 0x41]),
                country_region_id: 0,
                local_time_offset_negative: false,
                local_time_offset_bcd: 0x0100,
                time_of_change_raw: [0xAB, 0xCD, 0xEF, 0x12, 0x34],
                next_time_offset_bcd: 0x0200,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LocalTimeOffsetDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = LocalTimeOffsetDescriptor::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }
}
