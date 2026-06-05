//! Transport Stream Description Table — ISO/IEC 13818-1 §2.4.5.
//!
//! TSDT is carried on PID 0x0002 with table_id 0x03. It provides a
//! means of describing the transport stream using descriptors.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for Transport Stream Description Table.
pub const TABLE_ID: u8 = 0x03;
/// Well-known PID on which TSDT is carried.
pub const PID: u16 = 0x0002;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const CRC_LEN: usize = 4;

/// Transport Stream Description Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Tsdt<'a> {
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
    /// Descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

impl<'a> Parse<'a> for Tsdt<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Tsdt",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Tsdt",
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

        // §2.4.4.12: descriptors run directly from byte 8 to the CRC; there is
        // no descriptor_loop_length field. The section_length bounds the loop.
        let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let desc_end = total - CRC_LEN;

        Ok(Tsdt {
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            descriptors: DescriptorLoop::new(&bytes[desc_start..desc_end]),
        })
    }
}

impl Serialize for Tsdt<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN + EXTENSION_HEADER_LEN + self.descriptors.len() + CRC_LEN
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

        let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[desc_start..desc_start + self.descriptors.len()]
            .copy_from_slice(self.descriptors.raw());

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

impl<'a> Table<'a> for Tsdt<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Tsdt<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "TRANSPORT_STREAM_DESCRIPTION";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_tsdt(table_id_extension: u16, version: u8, descriptors: &[u8]) -> Vec<u8> {
        let section_length: u16 = (EXTENSION_HEADER_LEN + descriptors.len() + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&table_id_extension.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0x00);
        v.push(0x00);
        v.extend_from_slice(descriptors);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_tsdt(0x1234, 5, &[]);
        bytes[0] = 0x02;
        assert!(matches!(
            Tsdt::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x02, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = Tsdt::parse(&[0x03, 0xB0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_empty_descriptor_loop() {
        let bytes = build_tsdt(0x1234, 5, &[]);
        let tsdt = Tsdt::parse(&bytes).unwrap();
        assert_eq!(tsdt.table_id_extension, 0x1234);
        assert_eq!(tsdt.version_number, 5);
        assert!(tsdt.current_next_indicator);
        assert_eq!(tsdt.section_number, 0);
        assert_eq!(tsdt.last_section_number, 0);
        assert!(tsdt.descriptors.is_empty());
    }

    #[test]
    fn parse_with_descriptors() {
        let descriptors = [0x01, 0x03, 0xAA, 0xBB, 0xCC];
        let bytes = build_tsdt(0xABCD, 7, &descriptors);
        let tsdt = Tsdt::parse(&bytes).unwrap();
        assert_eq!(tsdt.table_id_extension, 0xABCD);
        assert_eq!(tsdt.version_number, 7);
        assert_eq!(tsdt.descriptors.raw(), &descriptors[..]);
    }

    #[test]
    fn serialize_round_trip() {
        let descriptors = [0x4D, 0x02, 0x01, 0x02];
        let bytes = build_tsdt(0xCAFE, 3, &descriptors);
        let tsdt = Tsdt::parse(&bytes).unwrap();
        let mut buf = vec![0u8; tsdt.serialized_len()];
        tsdt.serialize_into(&mut buf).unwrap();
        let re = Tsdt::parse(&buf).unwrap();
        assert_eq!(tsdt, re);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let bytes = build_tsdt(0x0001, 0, &[]);
        let tsdt = Tsdt::parse(&bytes).unwrap();
        let mut buf = vec![0u8; tsdt.serialized_len()];
        tsdt.serialize_into(&mut buf).unwrap();
        let re = Tsdt::parse(&buf).unwrap();
        assert_eq!(tsdt, re);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Tsdt<'_> as Table>::TABLE_ID, 0x03);
        assert_eq!(<Tsdt<'_> as Table>::PID, 0x0002);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn tsdt_serializes_typed_loop() {
        // TSDT now borrows its descriptor loop (3.0): serialize-only, the loop
        // emits the typed descriptor sequence (here a short_event, tag 0x4D).
        let descriptors = [0x4D, 0x02, b'e', b'n']; // valid short_event header bytes
        let bytes = build_tsdt(0xDEAD, 9, &descriptors);
        let tsdt = Tsdt::parse(&bytes).unwrap();
        let v = serde_json::to_value(&tsdt).unwrap();
        assert!(
            v["descriptors"].is_array(),
            "descriptors must serialize as a typed sequence, got {v}"
        );
    }
}
