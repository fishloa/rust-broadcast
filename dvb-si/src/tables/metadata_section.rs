//! `Metadata_section()` — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023
//! §2.12.6, Tables 2-158/2-159 (table_id 0x06); syntax in
//! `docs/tables/iso_13818_1/2-153-2-158-2-162-sections-14496-metadata-green.md`.
//!
//! ```text
//! table_id(8)=0x06 section_syntax_indicator(1) private_indicator(1)
//! random_access_indicator(1) decoder_config_flag(1) metadata_section_length(12)
//! metadata_service_id(8) reserved(8) section_fragment_indication(2)
//! version_number(5) current_next_indicator(1) section_number(8)
//! last_section_number(8) { metadata_byte(8) }* CRC_32(32)
//! ```
//!
//! The `metadata_byte`s are the contiguous bytes of a metadata Access Unit
//! whose format is out of scope here: carried as one opaque byte run.

use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// `table_id` of `Metadata_section()`.
pub const TABLE_ID: u8 = 0x06;
const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
use super::CRC_LEN;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;
/// Default of the 8-bit `reserved` (all ones, H.222.0 clause 2.1).
pub const RESERVED_DEFAULT: u8 = 0xFF;

const B1_SSI: u8 = 0x80;
const B1_PRIVATE: u8 = 0x40;
const B1_RANDOM_ACCESS: u8 = 0x20;
const B1_DECODER_CONFIG: u8 = 0x10;
const FRAGMENT_SHIFT: u32 = 6;
const FRAGMENT_MASK: u8 = 0x03;

/// `section_fragment_indication` (Table 2-159).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SectionFragmentIndication {
    /// `'11'` — a single section carrying a complete metadata Access Unit.
    Complete,
    /// `'10'` — the first section of a series for one metadata Access Unit.
    First,
    /// `'01'` — the last section of such a series.
    Last,
    /// `'00'` — neither the first nor the last section of such a series.
    Middle,
}

impl SectionFragmentIndication {
    /// Decode the 2-bit field (all four values are defined).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v & FRAGMENT_MASK {
            0b11 => Self::Complete,
            0b10 => Self::First,
            0b01 => Self::Last,
            _ => Self::Middle,
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Complete => 0b11,
            Self::First => 0b10,
            Self::Last => 0b01,
            Self::Middle => 0b00,
        }
    }

    /// Spec label (Table 2-159).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Complete => "complete metadata Access Unit",
            Self::First => "first section",
            Self::Last => "last section",
            Self::Middle => "neither first nor last section",
        }
    }
}
broadcast_common::impl_spec_display!(SectionFragmentIndication);

/// `Metadata_section()` — Table 2-158.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct MetadataSection<'a> {
    /// `section_syntax_indicator` (shall be 1; a received value is kept).
    pub section_syntax_indicator: bool,
    /// `private_indicator` (not specified by H.222.0).
    pub private_indicator: bool,
    /// `random_access_indicator`.
    pub random_access_indicator: bool,
    /// `decoder_config_flag`.
    pub decoder_config_flag: bool,
    /// `metadata_service_id`.
    pub metadata_service_id: u8,
    /// `reserved(8)`; re-emitted verbatim, default `0xFF`.
    pub reserved: u8,
    /// `section_fragment_indication` (Table 2-159).
    pub section_fragment_indication: SectionFragmentIndication,
    /// `version_number` (5 bits).
    pub version_number: u8,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
    /// `section_number`.
    pub section_number: u8,
    /// `last_section_number`.
    pub last_section_number: u8,
    /// `metadata_byte`s (opaque: the metadata Access Unit format is not part of
    /// this section syntax).
    pub metadata_bytes: &'a [u8],
}

impl<'a> MetadataSection<'a> {
    /// Construct with the spec fills (`section_syntax_indicator` 1, `reserved` all ones).
    #[must_use]
    pub fn new(
        metadata_service_id: u8,
        section_fragment_indication: SectionFragmentIndication,
        version_number: u8,
        section_number: u8,
        last_section_number: u8,
        metadata_bytes: &'a [u8],
    ) -> Self {
        Self {
            section_syntax_indicator: true,
            private_indicator: false,
            random_access_indicator: false,
            decoder_config_flag: false,
            metadata_service_id,
            reserved: RESERVED_DEFAULT,
            section_fragment_indication,
            version_number,
            current_next_indicator: true,
            section_number,
            last_section_number,
            metadata_bytes,
        }
    }
}

impl<'a> Parse<'a> for MetadataSection<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "MetadataSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "MetadataSection",
                expected: &[TABLE_ID],
            });
        }
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            super::section_length_of(bytes),
            MIN_SECTION_LEN,
        )?;
        let b1 = bytes[1];
        Ok(Self {
            section_syntax_indicator: b1 & B1_SSI != 0,
            private_indicator: b1 & B1_PRIVATE != 0,
            random_access_indicator: b1 & B1_RANDOM_ACCESS != 0,
            decoder_config_flag: b1 & B1_DECODER_CONFIG != 0,
            metadata_service_id: bytes[3],
            reserved: bytes[4],
            section_fragment_indication: SectionFragmentIndication::from_u8(
                bytes[5] >> FRAGMENT_SHIFT,
            ),
            version_number: super::version_number_of(bytes[5]),
            current_next_indicator: super::current_next_of(bytes[5]),
            section_number: bytes[6],
            last_section_number: bytes[7],
            metadata_bytes: &bytes[MIN_HEADER_LEN + EXTENSION_HEADER_LEN..total - CRC_LEN],
        })
    }
}

impl Serialize for MetadataSection<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MIN_SECTION_LEN + self.metadata_bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TABLE_ID;
        buf[1] = (u8::from(self.section_syntax_indicator) * B1_SSI)
            | (u8::from(self.private_indicator) * B1_PRIVATE)
            | (u8::from(self.random_access_indicator) * B1_RANDOM_ACCESS)
            | (u8::from(self.decoder_config_flag) * B1_DECODER_CONFIG);
        super::write_section_length(buf, len - MIN_HEADER_LEN)?;
        buf[3] = self.metadata_service_id;
        buf[4] = self.reserved;
        // version_number(5) | current_next_indicator(1), with the 2-bit
        // fragment indication in the reserved(2) position of the shared helper.
        buf[5] = super::b5_of(
            self.section_fragment_indication.to_u8(),
            self.version_number,
            self.current_next_indicator,
        )?;
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        let start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
        buf[start..start + self.metadata_bytes.len()].copy_from_slice(self.metadata_bytes);
        super::write_crc(buf, len - CRC_LEN);
        Ok(len)
    }
}

impl<'a> crate::traits::TableDef<'a> for MetadataSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "METADATA";
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;

    fn rt(s: &MetadataSection<'_>) -> Vec<u8> {
        let mut out = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut out).unwrap();
        let p = MetadataSection::parse(&out).unwrap();
        assert_eq!(&p, s);
        out
    }

    #[test]
    fn spec_vector_table_2_158() {
        // table_id 06, ssi=1 private=0 ra=1 dc=0 len=12-bit, service 0x21, reserved FF,
        // fragment '10' version 5 cni 1, section 0 of 2, 3 metadata bytes, CRC.
        let s = MetadataSection {
            random_access_indicator: true,
            ..MetadataSection::new(0x21, SectionFragmentIndication::First, 5, 0, 2, &[1, 2, 3])
        };
        let b = rt(&s);
        // 1 0 1 0 | length hi 0 -> 0xA0; length = 5 + 3 + 4 = 12
        assert_eq!(&b[..8], &[0x06, 0xA0, 0x0C, 0x21, 0xFF, 0b10_00101_1, 0, 2]);
        assert_eq!(&b[8..11], &[1, 2, 3]);
        assert_eq!(broadcast_common::crc32_mpeg2::compute(&b), 0);
    }

    #[test]
    fn all_flags_reserved_and_fragments_preserved() {
        for f in [
            SectionFragmentIndication::Complete,
            SectionFragmentIndication::First,
            SectionFragmentIndication::Last,
            SectionFragmentIndication::Middle,
        ] {
            let mut s = MetadataSection::new(1, f, 31, 4, 9, &[]);
            s.private_indicator = true;
            s.decoder_config_flag = true;
            s.reserved = 0x00;
            s.current_next_indicator = false;
            let b = rt(&s);
            assert_eq!(MetadataSection::parse(&b).unwrap().reserved, 0);
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert!(MetadataSection::parse(&[0x06, 0xB0, 0x00]).is_err());
        let mut b = vec![0u8; 12];
        b[0] = 0x05;
        assert!(matches!(
            MetadataSection::parse(&b),
            Err(Error::UnexpectedTableId { .. })
        ));
        let s = MetadataSection::new(1, SectionFragmentIndication::Complete, 32, 0, 0, &[]);
        assert!(s.serialize_into(&mut [0u8; 16]).is_err());
        let big = vec![0u8; 4090];
        let s = MetadataSection::new(1, SectionFragmentIndication::Complete, 0, 0, 0, &big);
        assert!(s.serialize_into(&mut vec![0u8; 5000]).is_err());
    }
}
