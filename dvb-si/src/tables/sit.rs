//! Selection Information Table — ETSI EN 300 468 §5.2.10.
//!
//! SIT is carried on PID 0x001F with table_id 0x7F. It is used in partial
//! transport streams (e.g., DVB-S2 multi-stream) to carry selection-related
//! descriptors.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Selection Information Table.
pub const TABLE_ID: u8 = 0x7F;
/// Well-known PID on which SIT is carried.
pub const PID: u16 = 0x001F;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const DESC_LOOP_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;

/// Selection Information Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sit {
    /// 16-bit table_id_extension.
    pub table_id_extension: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Descriptor loop bytes (owned copy).
    pub descriptors: Vec<u8>,
}

impl<'a> Parse<'a> for Sit {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Sit",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Sit",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = MIN_HEADER_LEN + section_length as usize;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length as usize,
                available: bytes.len() - MIN_HEADER_LEN,
            });
        }

        let table_id_extension = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let dl_pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let desc_loop_length =
            (((bytes[dl_pos] & 0x0F) as usize) << 8) | bytes[dl_pos + 1] as usize;
        let desc_start = dl_pos + DESC_LOOP_LEN_FIELD;
        let desc_end = desc_start + desc_loop_length;
        if desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: desc_loop_length,
                available: total - CRC_LEN - desc_start,
            });
        }

        Ok(Sit {
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            descriptors: bytes[desc_start..desc_end].to_vec(),
        })
    }
}

impl Serialize for Sit {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + DESC_LOOP_LEN_FIELD
            + self.descriptors.len()
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let section_length: u16 = (len - MIN_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.table_id_extension.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        let dl_pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let dl = self.descriptors.len() as u16;
        buf[dl_pos] = 0xF0 | ((dl >> 8) as u8 & 0x0F);
        buf[dl_pos + 1] = (dl & 0xFF) as u8;

        let desc_start = dl_pos + DESC_LOOP_LEN_FIELD;
        buf[desc_start..desc_start + self.descriptors.len()]
            .copy_from_slice(&self.descriptors);

        let crc_pos = len - CRC_LEN;
        buf[crc_pos..len].copy_from_slice(&[0, 0, 0, 0]);

        Ok(len)
    }
}

impl<'a> Table<'a> for Sit {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sit(table_id_extension: u16, version: u8, descriptors: &[u8]) -> Vec<u8> {
        let section_length: u16 =
            (EXTENSION_HEADER_LEN + DESC_LOOP_LEN_FIELD + descriptors.len() + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&table_id_extension.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0x00);
        v.push(0x00);
        let dl = descriptors.len() as u16;
        v.push(0xF0 | ((dl >> 8) as u8 & 0x0F));
        v.push((dl & 0xFF) as u8);
        v.extend_from_slice(descriptors);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_sit(0x1234, 5, &[]);
        bytes[0] = 0x7E;
        assert!(matches!(
            Sit::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x7E, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Sit::parse(&[0x7F, 0xB0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_empty_descriptor_loop() {
        let bytes = build_sit(0x1234, 5, &[]);
        let sit = Sit::parse(&bytes).unwrap();
        assert_eq!(sit.table_id_extension, 0x1234);
        assert_eq!(sit.version_number, 5);
        assert!(sit.current_next_indicator);
        assert_eq!(sit.section_number, 0);
        assert_eq!(sit.last_section_number, 0);
        assert!(sit.descriptors.is_empty());
    }

    #[test]
    fn parse_with_descriptors() {
        let descriptors = [0x4D, 0x02, 0x01, 0x02];
        let bytes = build_sit(0xABCD, 7, &descriptors);
        let sit = Sit::parse(&bytes).unwrap();
        assert_eq!(sit.table_id_extension, 0xABCD);
        assert_eq!(sit.version_number, 7);
        assert_eq!(sit.descriptors, &descriptors[..]);
    }

    #[test]
    fn serialize_round_trip() {
        let descriptors = [0x4D, 0x02, 0x01, 0x02];
        let bytes = build_sit(0xCAFE, 3, &descriptors);
        let sit = Sit::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sit.serialized_len()];
        sit.serialize_into(&mut buf).unwrap();
        let re = Sit::parse(&buf).unwrap();
        assert_eq!(sit, re);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let bytes = build_sit(0x0001, 0, &[]);
        let sit = Sit::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sit.serialized_len()];
        sit.serialize_into(&mut buf).unwrap();
        let re = Sit::parse(&buf).unwrap();
        assert_eq!(sit, re);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Sit as Table>::TABLE_ID, 0x7F);
        assert_eq!(<Sit as Table>::PID, 0x001F);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn sit_round_trips_via_json() {
        let descriptors = [0x4D, 0x02, 0x01, 0x02];
        let bytes = build_sit(0xDEAD, 9, &descriptors);
        let sit = Sit::parse(&bytes).unwrap();
        let j = serde_json::to_string(&sit).unwrap();
        let back: Sit = serde_json::from_str(&j).unwrap();
        assert_eq!(sit, back);
    }
}
