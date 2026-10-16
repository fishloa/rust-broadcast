//! `Green_access_unit_section_message()` — Rec. ITU-T H.222.0 (08/2023) |
//! ISO/IEC 13818-1:2023 §2.18.1–2.18.3, Tables 2-162/2-163 (table_id 0x09);
//! syntax in `docs/tables/iso_13818_1/2-153-2-158-2-162-sections-14496-metadata-green.md`.
//!
//! ```text
//! table_ID(8)=0x09 section_syntax_indicator(1)='0' private_indicator(1) reserved(2)
//! private_section_length(12) '0010'(4) Display_in_PTS[32..30](3) marker_bit(1)
//! Display_in_PTS[29..15](15) marker_bit(1) Display_in_PTS[14..0](15) marker_bit(1)
//! Green_AU() CRC_32(32)
//! ```
//!
//! `Green_AU()` (Table 2-163) is bounded by counts carried in the Green
//! extension descriptor (out of band), so it cannot be parsed from the section
//! alone and is carried as one opaque byte run, by design. The `'0010'` prefix
//! and the three marker bits are literals: a section without them is rejected.

use crate::error::{Error, Result};
use broadcast_common::clock33::{self, pack_marked, unpack_marked};
use broadcast_common::{Parse, Serialize};

/// `table_id` of the green access unit section.
pub const TABLE_ID: u8 = 0x09;
const MIN_HEADER_LEN: usize = 3;
const PTS_FIELD_LEN: usize = 5;
use super::CRC_LEN;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + PTS_FIELD_LEN + CRC_LEN;

/// The literal `'0010'` ahead of `Display_in_PTS[32..30]`.
const PTS_PREFIX: u8 = 0b0010;
/// A 33-bit PTS.
const PTS_BITS: u32 = 33;

/// `Green_access_unit_section_message()` — Table 2-162.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct GreenAccessUnitSection<'a> {
    /// `Display_in_PTS`: the 33-bit PTS of the associated video access unit.
    pub display_in_pts: u64,
    /// `Green_AU()` (opaque: its loops are bounded by the Green extension
    /// descriptor, which is out of band).
    pub green_au: &'a [u8],
    /// Header bit fields: `section_syntax_indicator` (shall be `'0'`),
    /// `private_indicator`, `reserved(2)`. Default is
    /// [`SectionBits::psi`](super::SectionBits::psi)`.short_form()`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bits: super::SectionBits,
}

impl<'a> GreenAccessUnitSection<'a> {
    /// Construct with the spec header fill (`section_syntax_indicator` 0).
    #[must_use]
    pub fn new(display_in_pts: u64, green_au: &'a [u8]) -> Self {
        Self {
            display_in_pts,
            green_au,
            bits: super::SectionBits::psi().short_form(),
        }
    }
}

impl<'a> Parse<'a> for GreenAccessUnitSection<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "GreenAccessUnitSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "GreenAccessUnitSection",
                expected: &[TABLE_ID],
            });
        }
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            super::section_length_of(bytes),
            MIN_SECTION_LEN,
        )?;
        let mut p = [0u8; PTS_FIELD_LEN];
        p.copy_from_slice(&bytes[MIN_HEADER_LEN..MIN_HEADER_LEN + PTS_FIELD_LEN]);
        let marked = unpack_marked(&p);
        if marked.prefix != PTS_PREFIX || marked.markers != clock33::MARKERS_ALL_SET {
            return Err(Error::ReservedBitsViolation {
                field: "Display_in_PTS",
                reason: "'0010' prefix or marker_bit missing",
            });
        }
        Ok(Self {
            display_in_pts: marked.timestamp,
            green_au: &bytes[MIN_HEADER_LEN + PTS_FIELD_LEN..total - CRC_LEN],
            bits: super::SectionBits::parse(bytes[1], None),
        })
    }
}

impl Serialize for GreenAccessUnitSection<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MIN_SECTION_LEN + self.green_au.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pts = broadcast_common::len::fit_bits(self.display_in_pts, PTS_BITS, "Display_in_PTS")?;
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TABLE_ID;
        buf[1] = self.bits.b1()?;
        super::write_section_length(buf, len - MIN_HEADER_LEN)?;
        let p = MIN_HEADER_LEN;
        buf[p..p + PTS_FIELD_LEN].copy_from_slice(&pack_marked(pts, PTS_PREFIX));
        let start = MIN_HEADER_LEN + PTS_FIELD_LEN;
        buf[start..start + self.green_au.len()].copy_from_slice(self.green_au);
        super::write_crc(buf, len - CRC_LEN);
        Ok(len)
    }
}

impl<'a> crate::traits::TableDef<'a> for GreenAccessUnitSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "GREEN_ACCESS_UNIT";
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;

    #[test]
    fn spec_vector_table_2_162() {
        // PTS 0x1_2345_6789 (33 bits): [32..30]=0b100, [29..15]=0x08D1, [14..0]=0x2B3C? computed below
        let pts: u64 = 0x1_2345_6789 & (clock33::WRAP_33BIT - 1);
        let s = GreenAccessUnitSection::new(pts, &[0x41, 0xFF, 0x10]);
        let mut b = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut b).unwrap();
        // header: table_id 09, ssi=0 priv=0 reserved=11 -> 0x30
        assert_eq!(&b[..2], &[0x09, 0x30]);
        // prefix 0010, PTS[32..30], marker
        assert_eq!(b[3] >> 4, 0b0010);
        assert_eq!(b[3] & 1, 1);
        assert_eq!(b[5] & 1, 1);
        assert_eq!(b[7] & 1, 1);
        assert_eq!(&b[8..11], &[0x41, 0xFF, 0x10]);
        assert_eq!(broadcast_common::crc32_mpeg2::compute(&b), 0);
        let p = GreenAccessUnitSection::parse(&b).unwrap();
        assert_eq!(p.display_in_pts, pts);
        assert_eq!(p, s);
        // independent encoding of PTS = 90_000 (1 s): 00 01 5F 90 per PES convention
        let s = GreenAccessUnitSection::new(90_000, &[]);
        let mut b = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut b).unwrap();
        assert_eq!(&b[3..8], &[0x21, 0x00, 0x05, 0xBF, 0x21]);
    }

    #[test]
    fn rejects_bad_markers_pts_overflow_and_ssi_kept() {
        let s = GreenAccessUnitSection::new(1, &[]);
        let mut b = vec![0u8; s.serialized_len()];
        s.serialize_into(&mut b).unwrap();
        let mut bad = b.clone();
        bad[5] &= 0xFE;
        assert!(matches!(
            GreenAccessUnitSection::parse(&bad),
            Err(Error::ReservedBitsViolation { .. })
        ));
        let mut bad = b.clone();
        bad[3] = 0x31;
        assert!(GreenAccessUnitSection::parse(&bad).is_err());
        assert!(
            GreenAccessUnitSection::new(clock33::WRAP_33BIT, &[])
                .serialize_into(&mut [0u8; 32])
                .is_err()
        );
        // a received ssi=1 is preserved on re-serialize
        let mut with_ssi = s.clone();
        with_ssi.bits.section_syntax_indicator = true;
        let mut b2 = vec![0u8; with_ssi.serialized_len()];
        with_ssi.serialize_into(&mut b2).unwrap();
        assert_eq!(b2[1] & 0x80, 0x80);
        assert_eq!(GreenAccessUnitSection::parse(&b2).unwrap(), with_ssi);
    }
}
