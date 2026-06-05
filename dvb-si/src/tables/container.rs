//! Container Table — ETSI TS 102 323 v1.4.1 §7.3.1.4 (table_id 0x75).
//!
//! The container section carries TV-Anytime container data (an MHP object
//! carousel file fragment, after optional compression). Syntax per "Table 20 —
//! Container section" (`dvb-si/docs/ts_102_323_tva.md`, §7.3.1.4, PDF p. 36):
//!
//! ```text
//! container_section() {
//!   table_id                 8   (0x75)
//!   section_syntax_indicator 1
//!   private_indicator        1
//!   reserved                 2
//!   private_section_length  12
//!   container_id            16   (table_id_extension)
//!   reserved                 2
//!   version_number           5
//!   current_next_indicator   1
//!   section_number           8
//!   last_section_number      8
//!   container_data()        N*8
//!   CRC_32                  32
//! }
//! ```
//!
//! This is a private section (the spec's byte-1 bit 6 is `private_indicator`),
//! so it is handled with the same idiom as [`crate::tables::cit`]. The
//! `container_data` payload is kept as a borrowed raw slice; its internal
//! structure (compression_wrapper / object-carousel fragments) is out of scope.
//!
//! Carriage is signalled via descriptors (no well-known PID), following the
//! [`crate::tables::dsmcc`] precedent of `PID = 0x0000`.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the Container Table.
pub const TABLE_ID: u8 = 0x75;

/// The Container Table has no well-known PID — its carriage is signalled via
/// descriptors. `0x0000` is a placeholder following the DSM-CC precedent.
pub const PID: u16 = 0x0000;

/// Bytes 0-2: table_id (1) + flags + private_section_length (2).
const HEADER_LEN: usize = 3;

/// Bytes 3-7: container_id(2) + reserved/version/cni(1) + section_number(1)
/// + last_section_number(1).
const EXTENSION_HEADER_LEN: usize = 5;

/// Bytes occupied by the trailing CRC-32 field.
const CRC_LEN: usize = 4;

/// Minimum total encoded length: header + extension + CRC.
const MIN_LEN: usize = HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;

/// Container Table (ETSI TS 102 323 v1.4.1 §7.3.1.4).
///
/// `container_data` borrows the payload region (everything between the extension
/// header and the CRC-32 trailer) without parsing its internal structure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Container<'a> {
    /// `private_indicator` bit from byte 1 (this is a private section).
    pub private_indicator: bool,
    /// 16-bit `container_id` (carried in the table_id_extension slot, bytes 3-4).
    pub container_id: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator` bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Raw `container_data` bytes (everything between the extension header and
    /// the CRC-32 trailer). Internal structure is not parsed.
    pub container_data: &'a [u8],
}

impl<'a> Parse<'a> for Container<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_LEN,
                have: bytes.len(),
                what: "Container",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Container",
                expected: &[TABLE_ID],
            });
        }

        // byte 1 = section_syntax_indicator(1) | private_indicator(1)
        //          | reserved(2) | private_section_length[11:8](4)
        // byte 2 = private_section_length[7:0]
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }

        let private_indicator = (bytes[1] & 0x40) != 0;

        // Extension header (bytes 3..8).
        let container_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        // byte 5: reserved(2) | version_number(5) | current_next_indicator(1)
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // container_data: from end of extension header up to (not including) CRC.
        let data_start = HEADER_LEN + EXTENSION_HEADER_LEN;
        let data_end = total - CRC_LEN;
        let container_data = &bytes[data_start..data_end];

        Ok(Container {
            private_indicator,
            container_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            container_data,
        })
    }
}

impl Serialize for Container<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + EXTENSION_HEADER_LEN + self.container_data.len() + CRC_LEN
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

        // Byte 0: table_id.
        buf[0] = TABLE_ID;
        // Byte 1: section_syntax_indicator(1)=1 | private_indicator(1)
        //         | reserved(2)=11 | private_section_length[11:8](4).
        buf[1] = 0x80
            | (u8::from(self.private_indicator) << 6)
            | 0x30
            | ((section_length >> 8) as u8 & 0x0F);
        // Byte 2: private_section_length[7:0].
        buf[2] = (section_length & 0xFF) as u8;

        // Extension header.
        buf[3..5].copy_from_slice(&self.container_id.to_be_bytes());
        buf[5] = 0xC0 // reserved(2) = 11
            | ((self.version_number & 0x1F) << 1)
            | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // container_data.
        let data_start = HEADER_LEN + EXTENSION_HEADER_LEN;
        let data_end = data_start + self.container_data.len();
        buf[data_start..data_end].copy_from_slice(self.container_data);

        // CRC-32 over everything up to (but not including) the CRC slot.
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..data_end]);
        buf[data_end..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

impl<'a> Table<'a> for Container<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Container<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "CONTAINER";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a syntactically valid Container section. `container_data` is pasted
    /// verbatim; the CRC slot is filled by the serializer.
    fn build_container(
        container_id: u16,
        version: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        container_data: &[u8],
    ) -> Vec<u8> {
        let c = Container {
            private_indicator: true,
            container_id,
            version_number: version,
            current_next_indicator: current_next,
            section_number,
            last_section_number,
            container_data,
        };
        let mut buf = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_happy_path() {
        let data = [0x00u8, 0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = build_container(0x1234, 7, true, 1, 3, &data);
        let c = Container::parse(&bytes).unwrap();
        assert!(c.private_indicator);
        assert_eq!(c.container_id, 0x1234);
        assert_eq!(c.version_number, 7);
        assert!(c.current_next_indicator);
        assert_eq!(c.section_number, 1);
        assert_eq!(c.last_section_number, 3);
        assert_eq!(c.container_data, &data[..]);
    }

    #[test]
    fn parse_empty_container_data() {
        let bytes = build_container(0x0000, 0, false, 0, 0, &[]);
        let c = Container::parse(&bytes).unwrap();
        assert_eq!(c.container_id, 0x0000);
        assert_eq!(c.version_number, 0);
        assert!(!c.current_next_indicator);
        assert!(c.container_data.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_container(0x0001, 0, true, 0, 0, &[]);
        bytes[0] = 0x70; // not 0x75
        assert!(matches!(
            Container::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x70, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            Container::parse(&[0x75, 0x80]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_section_length_overflow() {
        let mut bytes = build_container(0x0001, 0, true, 0, 0, &[]);
        let fake_sl: u16 = (bytes.len() as u16) + 100 - HEADER_LEN as u16;
        bytes[1] = (bytes[1] & 0xF0) | ((fake_sl >> 8) as u8 & 0x0F);
        bytes[2] = (fake_sl & 0xFF) as u8;
        assert!(matches!(
            Container::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let data = [0x01u8, 0x02, 0x03];
        let original = Container {
            private_indicator: false,
            container_id: 0xABCD,
            version_number: 15,
            current_next_indicator: false,
            section_number: 2,
            last_section_number: 4,
            container_data: &data,
        };
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        assert_eq!(Container::parse(&buf).unwrap(), original);
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let c = Container {
            private_indicator: false,
            container_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            container_data: &[],
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            c.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<Container as Table>::TABLE_ID, 0x75);
        assert_eq!(<Container as Table>::PID, 0x0000);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_json_round_trip() {
        // `container_data` is a borrowed `&[u8]` (the rnt/cit idiom), so a
        // deserialize round-trip is not possible — serde encodes a byte slice
        // as a JSON sequence which cannot be re-borrowed. Mirror the borrowed
        // tables by asserting serialization yields valid, field-bearing JSON.
        let data = [0xCAu8, 0xFE];
        let bytes = build_container(0xBEEF, 9, true, 0, 0, &data);
        let c = Container::parse(&bytes).unwrap();
        let v: serde_json::Value = serde_json::to_value(&c).unwrap();
        assert_eq!(v["container_id"], 0xBEEF);
        assert_eq!(v["version_number"], 9);
        assert_eq!(v["current_next_indicator"], true);
        assert_eq!(v["container_data"], serde_json::json!([0xCA, 0xFE]));
    }
}
