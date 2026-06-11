//! DSM-CC section parser — ISO/IEC 13818-6 + ETSI EN 301 192 §9.
//!
//! This is intentionally minimal: header framing, table_id dispatch,
//! length check, and carrying the payload as `&[u8]`. Full DSM-CC payload
//! parsing is deliberately out of scope (YAGNI).
//!
//! SSI-dependent trailer: when `section_syntax_indicator == 1` the trailing
//! 4 bytes are a CRC-32 (computed over the whole section); when SSI == 0
//! they are an ISO/IEC 13818-6 checksum preserved verbatim in
//! [`DsmccSection::checksum`]. For SSI=1 the serializer recomputes CRC-32;
//! for SSI=0 the serializer re-emits the preserved checksum bytes.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// First table_id in the DSM-CC section range (inclusive).
pub const TABLE_ID_FIRST: u8 = 0x3A;
/// Last table_id in the DSM-CC section range (inclusive).
pub const TABLE_ID_LAST: u8 = 0x3F;
/// DSM-CC has no well-known PID.
pub const PID: u16 = 0x0000;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;

/// A DSM-CC section — minimal wrapper that validates header framing
/// and carries the raw payload.
#[derive(Debug, Clone, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DsmccSection<'a> {
    /// The table_id byte (0x3A..=0x3F).
    pub table_id: u8,
    /// `section_syntax_indicator` bit. When `true` the trailer is a computed
    /// CRC-32; when `false` it is an ISO/IEC 13818-6 checksum preserved
    /// verbatim in [`Self::checksum`].
    pub section_syntax_indicator: bool,
    /// `private_indicator` bit (byte 1, bit 6).
    pub private_indicator: bool,
    /// 16-bit table_id_extension.
    pub extension_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number.
    pub section_number: u8,
    /// last_section_number.
    pub last_section_number: u8,
    /// Raw payload bytes (everything between the extension header and the trailer).
    pub payload: &'a [u8],
    /// Verbatim trailer bytes when `section_syntax_indicator == false` (an
    /// ISO/IEC 13818-6 checksum). Ignored when SSI is `true`, where the
    /// trailer is a computed CRC-32.
    pub checksum: [u8; 4],
}

impl PartialEq for DsmccSection<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.table_id == other.table_id
            && self.section_syntax_indicator == other.section_syntax_indicator
            && self.private_indicator == other.private_indicator
            && self.extension_id == other.extension_id
            && self.version_number == other.version_number
            && self.current_next_indicator == other.current_next_indicator
            && self.section_number == other.section_number
            && self.last_section_number == other.last_section_number
            && self.payload == other.payload
            && (self.section_syntax_indicator || self.checksum == other.checksum)
    }
}

impl<'a> Parse<'a> for DsmccSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "DsmccSection",
            });
        }

        let table_id = bytes[0];
        if !(TABLE_ID_FIRST..=TABLE_ID_LAST).contains(&table_id) {
            return Err(Error::UnexpectedTableId {
                table_id,
                what: "DsmccSection",
                expected: &[TABLE_ID_FIRST, TABLE_ID_LAST],
            });
        }

        let section_syntax_indicator = (bytes[1] & 0x80) != 0;
        let private_indicator = (bytes[1] & 0x40) != 0;
        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;

        let extension_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let payload_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let payload_end = total - CRC_LEN;
        let payload = &bytes[payload_start..payload_end];

        let trailer_start = total - CRC_LEN;
        let checksum = [
            bytes[trailer_start],
            bytes[trailer_start + 1],
            bytes[trailer_start + 2],
            bytes[trailer_start + 3],
        ];

        Ok(DsmccSection {
            table_id,
            section_syntax_indicator,
            private_indicator,
            extension_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            payload,
            checksum,
        })
    }
}

impl Serialize for DsmccSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN + EXTENSION_HEADER_LEN + self.payload.len() + CRC_LEN
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
        buf[0] = self.table_id;
        buf[1] = if self.section_syntax_indicator {
            super::SECTION_B1_FLAGS_PSI
        } else {
            (u8::from(self.private_indicator) << 6) | super::SECTION_B1_RESERVED_HI
        } | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.extension_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        let payload_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        let payload_end = payload_start + self.payload.len();
        buf[payload_start..payload_end].copy_from_slice(self.payload);

        let trailer_start = payload_end;
        if self.section_syntax_indicator {
            let crc = dvb_common::crc32_mpeg2::compute(&buf[..trailer_start]);
            buf[trailer_start..len].copy_from_slice(&crc.to_be_bytes());
        } else {
            buf[trailer_start..len].copy_from_slice(&self.checksum);
        }
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for DsmccSection<'a> {
    /// Full DSM-CC range including `0x3E` (MPE datagram_section). The typed
    /// [`crate::tables::mpe::MpeDatagramSection`] view of `0x3E` is reachable
    /// type-keyed only (via `AnyTableSection::parse_as` or
    /// `MpeDatagramSection::parse`); the default dispatcher routes `0x3E` here.
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID_FIRST, TABLE_ID_LAST)];
    const NAME: &'static str = "DSM_CC_SECTION";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_dsmcc(table_id: u8, extension_id: u16, version: u8, payload: &[u8]) -> Vec<u8> {
        let section_length: u16 = (EXTENSION_HEADER_LEN + payload.len() + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(table_id);
        v.push(super::super::SECTION_B1_FLAGS_PSI | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&extension_id.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0);
        v.push(0);
        v.extend_from_slice(payload);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_dsmcc(0x3B, 0x0001, 0, &[]);
        bytes[0] = 0x00;
        let err = DsmccSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn parse_rejects_table_id_below_range() {
        let bytes = build_dsmcc(0x39, 0x0001, 0, &[]);
        let err = DsmccSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x39, .. }
        ));
    }

    #[test]
    fn parse_rejects_table_id_above_range() {
        let bytes = build_dsmcc(0x40, 0x0001, 0, &[]);
        let err = DsmccSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x40, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = DsmccSection::parse(&[0x3B, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_empty_payload() {
        let bytes = build_dsmcc(0x3B, 0x1234, 5, &[]);
        let sec = DsmccSection::parse(&bytes).expect("parse");
        assert_eq!(sec.table_id, 0x3B);
        assert_eq!(sec.extension_id, 0x1234);
        assert_eq!(sec.version_number, 5);
        assert!(sec.current_next_indicator);
        assert_eq!(sec.section_number, 0);
        assert_eq!(sec.last_section_number, 0);
        assert_eq!(sec.payload.len(), 0);
    }

    #[test]
    fn parse_0x3c_table_id_accepted() {
        let bytes = build_dsmcc(0x3C, 0x0001, 0, &[0xAA, 0xBB]);
        let sec = DsmccSection::parse(&bytes).unwrap();
        assert_eq!(sec.table_id, 0x3C);
        assert_eq!(sec.payload, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_payload_preserved() {
        let payload = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let bytes = build_dsmcc(0x3B, 0x0001, 0, &payload);
        let sec = DsmccSection::parse(&bytes).unwrap();
        assert_eq!(sec.payload, &payload[..]);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let sec = DsmccSection {
            table_id: 0x3B,
            section_syntax_indicator: true,
            private_indicator: false,
            extension_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            payload: &[],
            checksum: [0; 4],
        };
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        let reparsed = DsmccSection::parse(&buf).unwrap();
        assert_eq!(sec, reparsed);
    }

    #[test]
    fn serialize_round_trip_with_payload() {
        let payload: [u8; 5] = [0xDE, 0xAD, 0xBE, 0xEF, 0x00];
        let sec = DsmccSection {
            table_id: 0x3C,
            section_syntax_indicator: true,
            private_indicator: false,
            extension_id: 0xABCD,
            version_number: 3,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 2,
            payload: &payload,
            checksum: [0; 4],
        };
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        let reparsed = DsmccSection::parse(&buf).unwrap();
        assert_eq!(sec, reparsed);
    }

    #[test]
    fn serialize_round_trip_ssi_clear_preserves_checksum() {
        let payload: [u8; 3] = [0x01, 0x02, 0x03];
        let checksum: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];
        let sec = DsmccSection {
            table_id: 0x3B,
            section_syntax_indicator: false,
            private_indicator: true,
            extension_id: 0x5678,
            version_number: 1,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            payload: &payload,
            checksum,
        };
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[buf.len() - 4..], &checksum);
        let reparsed = DsmccSection::parse(&buf).unwrap();
        assert_eq!(sec, reparsed);
        assert_eq!(reparsed.checksum, checksum);
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID_FIRST;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            DsmccSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }
}
