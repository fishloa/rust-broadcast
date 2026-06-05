//! Satellite Access Table (SAT) — ETSI EN 300 468 §5.2.11.
//!
//! Long-form private section on PID 0x001B with table_id 0x4D. The SAT is a
//! *family*: a common `satellite_access_section()` header carries a 6-bit
//! `satellite_table_id` discriminant ([`SatTableId`]) that selects one of five
//! body structures (position v2, cell fragment, time association, beamhopping
//! time plan, position v3).
//!
//! The body is bit-packed orbital / beamhopping data (33-bit NCR split fields,
//! two's-complement lat/long, conditional ephemeris loops). It is exposed here
//! as a raw byte slice ([`Sat::body`]); the full per-variant field layout is
//! documented in `docs/en_300_468.md` (Tables 11a–11i). This mirrors the crate
//! convention of keeping complex variable-length loops raw (cf. the descriptor
//! loops in `bat.rs`).

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};
use num_enum::TryFromPrimitive;

/// table_id for the Satellite Access Table.
pub const TABLE_ID: u8 = 0x4D;
/// Well-known PID on which the SAT is carried (EN 300 468 Table 1, §5.1.3).
pub const PID: u16 = 0x001B;

/// Bytes of fixed header before the body (table_id..reserved_zero_future_use).
const HEADER_LEN: usize = 9;
/// `section_length` counts from byte 3 (just after the field) to end of section.
const SECTION_LENGTH_PREFIX: usize = 3;
/// CRC_32 trailer length.
const CRC_LEN: usize = 4;

/// `satellite_table_id` discriminant — selects the SAT body structure (§5.2.11.1, Table 11b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
pub enum SatTableId {
    /// `satellite_position_v2_info` — TLE/SGP4 orbital elements (§5.2.11.2).
    PositionV2 = 0,
    /// `cell_fragment_info` — earth-surface cell coverage areas (§5.2.11.3).
    CellFragment = 1,
    /// `time_association_info` — NCR↔UTC time association (§5.2.11.4).
    TimeAssociation = 2,
    /// `beamhopping_time_plan_info` — beam illumination schedule (§5.2.11.5).
    BeamhoppingTimePlan = 3,
    /// `satellite_position_v3_info` — ephemeris state vectors (§5.2.11.6).
    PositionV3 = 4,
}

/// Satellite Access Table section (EN 300 468 §5.2.11.1, Table 11a).
///
/// The typed fields cover the common section header; [`Sat::body`] is the raw
/// body whose structure depends on [`Sat::satellite_table_id`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Sat<'a> {
    /// 6-bit discriminant selecting the body structure (see [`SatTableId`]).
    pub satellite_table_id: u8,
    /// 10-bit sub_table discriminator (e.g. the 10 MSBs of `satellite_id`).
    pub table_count: u16,
    /// 5-bit sub_table version number.
    pub version_number: u8,
    /// When `true`, this sub_table is currently applicable.
    pub current_next_indicator: bool,
    /// Section number within the sub_table.
    pub section_number: u8,
    /// Highest section number of the sub_table.
    pub last_section_number: u8,
    /// Raw body bytes — interpret per [`Sat::satellite_table_id`]; layout in `docs/tables/sat.md`.
    pub body: &'a [u8],
}

impl Sat<'_> {
    /// Typed view of [`Sat::satellite_table_id`], or `None` if reserved (5–63).
    #[must_use]
    pub fn kind(&self) -> Option<SatTableId> {
        SatTableId::try_from(self.satellite_table_id).ok()
    }
}

impl<'a> Parse<'a> for Sat<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "Sat",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Sat",
                expected: &[TABLE_ID],
            });
        }
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = SECTION_LENGTH_PREFIX + section_length;
        if bytes.len() < total || total < HEADER_LEN + CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len().saturating_sub(SECTION_LENGTH_PREFIX),
            });
        }
        let satellite_table_id = bytes[3] >> 2;
        let table_count = (((bytes[3] & 0x03) as u16) << 8) | bytes[4] as u16;
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = bytes[5] & 0x01 != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        // bytes[8] = reserved_zero_future_use
        let body = &bytes[HEADER_LEN..total - CRC_LEN];
        Ok(Sat {
            satellite_table_id,
            table_count,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            body,
        })
    }
}

impl Serialize for Sat<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body.len() + CRC_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - SECTION_LENGTH_PREFIX) as u16;
        buf[0] = TABLE_ID;
        // section_syntax_indicator=1, private_indicator=1, reserved=11, section_length hi nibble.
        buf[1] = 0xF0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3] = (self.satellite_table_id << 2) | ((self.table_count >> 8) as u8 & 0x03);
        buf[4] = (self.table_count & 0xFF) as u8;
        // reserved=11, version_number(5), current_next_indicator(1).
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8] = 0x00; // reserved_zero_future_use
        let body_end = HEADER_LEN + self.body.len();
        buf[HEADER_LEN..body_end].copy_from_slice(self.body);
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..body_end]);
        buf[body_end..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for Sat<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Sat<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "SATELLITE_ACCESS";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a SAT section with the given discriminant + body.
    fn build_sat(satellite_table_id: u8, table_count: u16, body: &[u8]) -> Vec<u8> {
        let section_length = (HEADER_LEN - SECTION_LENGTH_PREFIX + body.len() + CRC_LEN) as u16;
        let mut v = vec![
            TABLE_ID,
            0xF0 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            (satellite_table_id << 2) | ((table_count >> 8) as u8 & 0x03),
            (table_count & 0xFF) as u8,
            0xC0 | (0x05 << 1) | 0x01, // version 5, current_next = 1
            0x00,                      // section_number
            0x00,                      // last_section_number
            0x00,                      // reserved_zero_future_use
        ];
        v.extend_from_slice(body);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_position_v3_discriminant() {
        let body = [0xAA, 0xBB, 0xCC, 0xDD];
        let bytes = build_sat(4, 0x1A3, &body);
        let sat = Sat::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 4);
        assert_eq!(sat.kind(), Some(SatTableId::PositionV3));
        assert_eq!(sat.table_count, 0x1A3);
        assert_eq!(sat.version_number, 5);
        assert!(sat.current_next_indicator);
        assert_eq!(sat.body, &body);
    }

    #[test]
    fn reserved_discriminant_has_no_kind() {
        let bytes = build_sat(7, 0, &[]);
        let sat = Sat::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 7);
        assert_eq!(sat.kind(), None);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_sat(0, 0, &[1, 2, 3]);
        bytes[0] = 0x40;
        assert!(matches!(
            Sat::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x40, .. }
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(matches!(
            Sat::parse(&[0x4D, 0xF0]).unwrap_err(),
            Error::BufferTooShort { what: "Sat", .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let body = [0x01, 0x02, 0x03, 0x04, 0x05];
        let bytes = build_sat(1, 0x2FF, &body);
        let sat = Sat::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        let re = Sat::parse(&buf).unwrap();
        assert_eq!(sat, re);
        assert_eq!(re.kind(), Some(SatTableId::CellFragment));
        assert_eq!(re.table_count, 0x2FF);
    }
}
