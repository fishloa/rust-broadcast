//! Time Offset Table — ETSI EN 300 468 §5.2.6.
//!
//! Carried on PID 0x0014 with table_id 0x73. Structure:
//!   5-byte UTC time + descriptor loop + 4-byte CRC32.
//!
//! The TOT is the spec's framing exception: `section_syntax_indicator` SHALL
//! be `0b0` (§5.2.6: "This 1-bit field shall be set to 0b0") yet the section
//! still ends with a CRC_32. Do not route TOT bytes through the generic
//! [`crate::section::Section`] short-form path — it would fold the CRC into
//! the payload. Parse with [`Tot::parse`] directly.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Time Offset Table.
pub const TABLE_ID: u8 = 0x73;
/// Well-known PID on which TOT is carried (same as TDT).
pub const PID: u16 = 0x0014;

const HEADER_LEN: usize = 3;
const UTC_TIME_LEN: usize = 5;
const DESC_LOOP_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;

/// Time Offset Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Tot<'a> {
    /// Raw 5-byte UTC time (16-bit MJD + 24-bit BCD HHMMSS).
    pub utc_time_raw: [u8; 5],
    /// Raw descriptor bytes (typically local_time_offset_descriptor tag 0x58).
    /// Descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

impl<'a> Parse<'a> for Tot<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Tot",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Tot",
                expected: &[TABLE_ID],
            });
        }
        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = HEADER_LEN + section_length as usize;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - HEADER_LEN,
            });
        }
        let utc_time_raw = [bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]];
        let dl_pos = HEADER_LEN + UTC_TIME_LEN;
        let dl = (((bytes[dl_pos] & 0x0F) as usize) << 8) | bytes[dl_pos + 1] as usize;
        let d_start = dl_pos + DESC_LOOP_LEN_FIELD;
        let d_end = d_start + dl;
        if d_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: dl,
                available: total - CRC_LEN - d_start,
            });
        }
        Ok(Tot {
            utc_time_raw,
            descriptors: DescriptorLoop::new(&bytes[d_start..d_end]),
        })
    }
}

impl Serialize for Tot<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + self.descriptors.len() + CRC_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        // §5.2.6: section_syntax_indicator SHALL be 0 for the TOT (despite the
        // trailing CRC_32). 0x70 = SSI(0) | reserved_future_use(1) | reserved(11).
        buf[1] = 0x70 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..8].copy_from_slice(&self.utc_time_raw);
        let dl = self.descriptors.len() as u16;
        buf[8] = 0xF0 | ((dl >> 8) as u8 & 0x0F);
        buf[9] = (dl & 0xFF) as u8;
        let d_end = 10 + self.descriptors.len();
        buf[10..d_end].copy_from_slice(self.descriptors.raw());
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Tot<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Tot<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "TIME_OFFSET";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_tot(desc: &[u8]) -> Vec<u8> {
        let section_length = (UTC_TIME_LEN + DESC_LOOP_LEN_FIELD + desc.len() + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        // SSI=0 per §5.2.6 (the TOT exception: SSI=0 but CRC present).
        v.push(0x70 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&[0xE4, 0x09, 0x12, 0x34, 0x56]);
        let dl = desc.len() as u16;
        v.push(0xF0 | ((dl >> 8) as u8 & 0x0F));
        v.push((dl & 0xFF) as u8);
        v.extend_from_slice(desc);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_with_no_descriptors() {
        let bytes = build_tot(&[]);
        let tot = Tot::parse(&bytes).unwrap();
        assert_eq!(tot.utc_time_raw, [0xE4, 0x09, 0x12, 0x34, 0x56]);
        assert_eq!(tot.descriptors.raw(), &[] as &[u8]);
    }

    #[test]
    fn parse_with_local_time_offset_descriptor() {
        let lto = [
            0x58u8, 13, b'G', b'B', b'R', 0x02, 0x00, 0x00, 0xE4, 0x09, 0x12, 0x34, 0x56, 0x01,
            0x00,
        ];
        let bytes = build_tot(&lto);
        let tot = Tot::parse(&bytes).unwrap();
        assert_eq!(tot.descriptors.raw(), &lto[..]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_tot(&[]);
        bytes[0] = 0x70;
        assert!(matches!(
            Tot::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let lto = [0x58u8, 0];
        let bytes = build_tot(&lto);
        let tot = Tot::parse(&bytes).unwrap();
        let mut buf = vec![0u8; tot.serialized_len()];
        tot.serialize_into(&mut buf).unwrap();
        let re = Tot::parse(&buf).unwrap();
        assert_eq!(tot, re);
    }
}
