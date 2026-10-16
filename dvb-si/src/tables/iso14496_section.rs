//! `ISO_IEC_14496_section()` — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC
//! 13818-1:2023 §2.11.3.8, Table 2-153 (table_id 0x04 scene description, 0x05
//! object descriptor, 0x08 other ISO/IEC 14496 streams); syntax in
//! `docs/tables/iso_13818_1/2-153-2-158-2-162-sections-14496-metadata-green.md`.
//!
//! The header (`table_id_extension`, version, section numbers) is typed. The
//! payload is one of `SL_Packet()` / `M4MuxPacket()`s / reserved bytes
//! depending on the descriptors of the *carrying PMT* (out of band) and on
//! ISO/IEC 14496-1 (not vendored), so it is carried as one opaque byte run,
//! by design.

use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// `table_id` of the scene description stream section.
pub const TABLE_ID_SCENE_DESCRIPTION: u8 = 0x04;
/// `table_id` of the object descriptor stream section.
pub const TABLE_ID_OBJECT_DESCRIPTOR: u8 = 0x05;
/// `table_id` of the "other ISO/IEC 14496 streams" section.
pub const TABLE_ID_OTHER: u8 = 0x08;
const ALLOWED: &[u8] = &[
    TABLE_ID_SCENE_DESCRIPTION,
    TABLE_ID_OBJECT_DESCRIPTOR,
    TABLE_ID_OTHER,
];

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
use super::CRC_LEN;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;

/// `ISO_IEC_14496_section()` — Table 2-153.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct Iso14496Section<'a> {
    /// `table_id`: 0x04, 0x05 or 0x08.
    pub table_id: u8,
    /// `table_id_extension` (user defined).
    pub table_id_extension: u16,
    /// `version_number` (5 bits).
    pub version_number: u8,
    /// `current_next_indicator` (shall be 1).
    pub current_next_indicator: bool,
    /// `section_number`.
    pub section_number: u8,
    /// `last_section_number`.
    pub last_section_number: u8,
    /// `SL_Packet()` / `M4MuxPacket()`s / `reserved` bytes up to the CRC
    /// (opaque: depends on the PMT and on ISO/IEC 14496-1, not vendored).
    pub payload: &'a [u8],
    /// Header bit fields (`private_indicator`, the two `reserved(2)`; default
    /// [`SectionBits::psi`](super::SectionBits::psi)).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bits: super::SectionBits,
}

impl<'a> Iso14496Section<'a> {
    /// Construct with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        table_id: u8,
        table_id_extension: u16,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        payload: &'a [u8],
    ) -> Self {
        Self {
            table_id,
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            payload,
            bits: super::SectionBits::psi(),
        }
    }
}

impl<'a> Parse<'a> for Iso14496Section<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "Iso14496Section",
            });
        }
        if !ALLOWED.contains(&bytes[0]) {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Iso14496Section",
                expected: ALLOWED,
            });
        }
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            super::section_length_of(bytes),
            MIN_SECTION_LEN,
        )?;
        Ok(Self {
            table_id: bytes[0],
            table_id_extension: crate::wire::be_u16(bytes, 3, "iso14496")?,
            version_number: super::version_number_of(bytes[5]),
            current_next_indicator: super::current_next_of(bytes[5]),
            section_number: bytes[6],
            last_section_number: bytes[7],
            payload: &bytes[MIN_HEADER_LEN + EXTENSION_HEADER_LEN..total - CRC_LEN],
            bits: super::SectionBits::parse(bytes[1], Some(bytes[5])),
        })
    }
}

impl Serialize for Iso14496Section<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MIN_SECTION_LEN + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if !ALLOWED.contains(&self.table_id) {
            return Err(Error::UnexpectedTableId {
                table_id: self.table_id,
                what: "Iso14496Section",
                expected: ALLOWED,
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.table_id;
        buf[1] = self.bits.b1()?;
        super::write_section_length(buf, len - MIN_HEADER_LEN)?;
        buf[3..5].copy_from_slice(&self.table_id_extension.to_be_bytes());
        buf[5] = self
            .bits
            .b5(self.version_number, self.current_next_indicator)?;
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        let start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[start..start + self.payload.len()].copy_from_slice(self.payload);
        super::write_crc(buf, len - CRC_LEN);
        Ok(len)
    }
}

impl<'a> crate::traits::TableDef<'a> for Iso14496Section<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[
        (TABLE_ID_SCENE_DESCRIPTION, TABLE_ID_OBJECT_DESCRIPTOR),
        (TABLE_ID_OTHER, TABLE_ID_OTHER),
    ];
    const NAME: &'static str = "ISO_IEC_14496";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(s: &Iso14496Section<'_>) -> Vec<u8> {
        let mut out = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut out).unwrap();
        let p = Iso14496Section::parse(&out).unwrap();
        assert_eq!(&p, s);
        let mut again = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut again).unwrap();
        assert_eq!(again, out);
        out
    }

    #[test]
    fn all_three_table_ids_round_trip() {
        for tid in ALLOWED {
            let s = Iso14496Section::new(*tid, 0xBEEF, 7, true, 1, 3, &[0xDE, 0xAD, 0xBE]);
            let b = rt(&s);
            assert_eq!(b[0], *tid);
            // section_syntax_indicator=1, private_indicator=0, reserved=11 -> 0xB0
            assert_eq!(b[1] & 0xF0, 0xB0);
            assert_eq!(
                broadcast_common::crc32_mpeg2::compute(&b),
                0,
                "CRC must close"
            );
        }
    }

    #[test]
    fn reserved_and_private_bits_preserved() {
        let mut s = Iso14496Section::new(0x04, 1, 0, true, 0, 0, &[]);
        s.bits.private_indicator = true;
        s.bits.reserved_b1 = 0;
        s.bits.reserved_b5 = 0;
        let b = rt(&s);
        assert_eq!(b[1] & 0xF0, 0xC0);
        assert_eq!(b[5] & 0xC0, 0);
    }

    #[test]
    fn rejects_other_table_ids_and_truncation() {
        let mut b = vec![0u8; 12];
        b[0] = 0x06;
        assert!(matches!(
            Iso14496Section::parse(&b),
            Err(Error::UnexpectedTableId { table_id: 0x06, .. })
        ));
        assert!(Iso14496Section::parse(&[0x04, 0xB0, 0x20]).is_err());
        let s = Iso14496Section::new(0x07, 0, 0, true, 0, 0, &[]);
        assert!(s.serialize_into(&mut [0u8; 16]).is_err());
        let mut s = Iso14496Section::new(0x08, 0, 32, true, 0, 0, &[]);
        assert!(s.serialize_into(&mut [0u8; 16]).is_err()); // 6-bit-wide version
        s.version_number = 31;
        assert!(s.serialize_into(&mut [0u8; 16]).is_ok());
    }
}
