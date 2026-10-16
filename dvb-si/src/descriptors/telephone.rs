//! Telephone Descriptor — ETSI EN 300 468 §6.2.42 (tag 0x57).
//!
//! Table 100 (PDF p. 107). Carries a telephone number (for IRD-initiated
//! return-channel calls) split into five length-prefixed character fields.
//!
//! Fixed 3-byte header:
//!   byte0: reserved(2) | foreign_availability(1) | connection_type(5)
//!   byte1: reserved(1) | country_prefix_length(2)
//!          | international_area_code_length(3) | operator_code_length(2)
//!   byte2: reserved(1) | national_area_code_length(3) | core_number_length(4)
//! followed by the five char loops, each `<field>_length` ISO 8859-1 bytes.
//!
//! Each char field is exposed as [`DvbText`] so the
//! public API is decoded text rather than raw bytes; the four/three/two-bit
//! length fields are derived from the raw byte length on serialize and ERROR
//! on over-range rather than truncating (crate idiom).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for telephone_descriptor.
pub const TAG: u8 = 0x57;
const HEADER_LEN: usize = 2;
/// Three fixed flag/length bytes precede the char loops.
const FIXED_LEN: usize = 3;

const FOREIGN_AVAIL_MASK: u8 = 0x20;
const CONNECTION_TYPE_MASK: u8 = 0x1F;
const BYTE0_RESERVED_MASK: u8 = 0xC0;
const BYTE0_RESERVED_SHIFT: u8 = 6;
const BYTE1_RESERVED_MASK: u8 = 0x80;
const BYTE1_RESERVED_SHIFT: u8 = 7;
const BYTE2_RESERVED_MASK: u8 = 0x80;
const BYTE2_RESERVED_SHIFT: u8 = 7;
/// Spec default of the 2-bit `reserved_future_use` in byte 0: all ones
/// (EN 300 468 V1.3.1-V1.20.1 Table 100, TS 102 006 V1.2.1-V1.4.1, EN 301 192
/// V1.3.1-V1.8.1; audit `dvb-si/docs/reserved-bits-edition-audit.md` §6.1,
/// class A. The V1.17.1 erratum that prints a 5-bit reserved is corrected in
/// V1.18.1).
pub const RESERVED_FUTURE_USE_BYTE0_DEFAULT: u8 = 0b11;
/// Spec default of the 1-bit `reserved_future_use` in byte 1: `1`.
pub const RESERVED_FUTURE_USE_BYTE1_DEFAULT: u8 = 1;
/// Spec default of the 1-bit `reserved_future_use` in byte 2: `1`.
pub const RESERVED_FUTURE_USE_BYTE2_DEFAULT: u8 = 1;

/// Telephone Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct TelephoneDescriptor<'a> {
    /// `reserved_future_use`(2) at the top of the first body byte (2 bits,
    /// right-aligned). Captured on parse, re-emitted verbatim;
    /// [`TelephoneDescriptor::new`] defaults to
    /// [`RESERVED_FUTURE_USE_BYTE0_DEFAULT`].
    pub reserved_future_use_before_foreign_availability: u8,
    /// `reserved_future_use`(1) ahead of `country_prefix_length` (1 bit).
    /// Default [`RESERVED_FUTURE_USE_BYTE1_DEFAULT`].
    pub reserved_future_use_before_country_prefix_length: u8,
    /// `reserved_future_use`(1) ahead of `national_area_code_length` (1 bit).
    /// Default [`RESERVED_FUTURE_USE_BYTE2_DEFAULT`].
    pub reserved_future_use_before_national_area_code_length: u8,
    /// When true, the number may be dialled from outside the prefix's country.
    pub foreign_availability: bool,
    /// 5-bit connection_type (meaning out of scope of EN 300 468).
    pub connection_type: u8,
    /// country_prefix_char bytes (≤ 3, ISO 8859-1).
    pub country_prefix: DvbText<'a>,
    /// international_area_code_char bytes (≤ 7).
    pub international_area_code: DvbText<'a>,
    /// operator_code_char bytes (≤ 3).
    pub operator_code: DvbText<'a>,
    /// national_area_code_char bytes (≤ 7).
    pub national_area_code: DvbText<'a>,
    /// core_number_char bytes (≤ 15).
    pub core_number: DvbText<'a>,
    /// Bytes after `core_number` inside the declared descriptor length. EN 300
    /// 468 Table 100 defines none (a non-conformant sender appended them);
    /// parse is tolerant and keeps them here so the descriptor re-serializes
    /// byte-identically. [`TelephoneDescriptor::new`] leaves it empty.
    pub trailing: Vec<u8>,
}

impl<'a> TelephoneDescriptor<'a> {
    /// Build a descriptor with the spec-default reserved bits (all ones).
    #[must_use]
    pub const fn new(
        foreign_availability: bool,
        connection_type: u8,
        country_prefix: DvbText<'a>,
        international_area_code: DvbText<'a>,
        operator_code: DvbText<'a>,
        national_area_code: DvbText<'a>,
        core_number: DvbText<'a>,
    ) -> Self {
        Self {
            reserved_future_use_before_foreign_availability: RESERVED_FUTURE_USE_BYTE0_DEFAULT,
            reserved_future_use_before_country_prefix_length: RESERVED_FUTURE_USE_BYTE1_DEFAULT,
            reserved_future_use_before_national_area_code_length: RESERVED_FUTURE_USE_BYTE2_DEFAULT,
            foreign_availability,
            connection_type,
            country_prefix,
            international_area_code,
            operator_code,
            national_area_code,
            core_number,
            trailing: Vec::new(),
        }
    }
}

impl<'a> Parse<'a> for TelephoneDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TelephoneDescriptor",
            "unexpected tag for telephone_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "telephone_descriptor length too short for fixed fields",
            });
        }
        let foreign_availability = body[0] & FOREIGN_AVAIL_MASK != 0;
        let connection_type = body[0] & CONNECTION_TYPE_MASK;
        let country_prefix_length = ((body[1] >> 5) & 0x03) as usize;
        let international_area_code_length = ((body[1] >> 2) & 0x07) as usize;
        let operator_code_length = (body[1] & 0x03) as usize;
        let national_area_code_length = ((body[2] >> 4) & 0x07) as usize;
        let core_number_length = (body[2] & 0x0F) as usize;
        let total_chars = country_prefix_length
            + international_area_code_length
            + operator_code_length
            + national_area_code_length
            + core_number_length;
        if FIXED_LEN + total_chars > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "sum of telephone char-field lengths exceeds descriptor body",
            });
        }
        let mut pos = FIXED_LEN;
        let mut take = |n: usize| {
            let s = &body[pos..pos + n];
            pos += n;
            s
        };
        let country_prefix = take(country_prefix_length);
        let international_area_code = take(international_area_code_length);
        let operator_code = take(operator_code_length);
        let national_area_code = take(national_area_code_length);
        let core_number = take(core_number_length);
        Ok(Self {
            reserved_future_use_before_foreign_availability: (body[0] & BYTE0_RESERVED_MASK)
                >> BYTE0_RESERVED_SHIFT,
            reserved_future_use_before_country_prefix_length: (body[1] & BYTE1_RESERVED_MASK)
                >> BYTE1_RESERVED_SHIFT,
            reserved_future_use_before_national_area_code_length: (body[2] & BYTE2_RESERVED_MASK)
                >> BYTE2_RESERVED_SHIFT,
            foreign_availability,
            connection_type,
            country_prefix: DvbText::new(country_prefix),
            international_area_code: DvbText::new(international_area_code),
            operator_code: DvbText::new(operator_code),
            national_area_code: DvbText::new(national_area_code),
            core_number: DvbText::new(core_number),
            trailing: body[FIXED_LEN + total_chars..].to_vec(),
        })
    }
}

impl Serialize for TelephoneDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_LEN
            + self.country_prefix.raw().len()
            + self.international_area_code.raw().len()
            + self.operator_code.raw().len()
            + self.national_area_code.raw().len()
            + self.core_number.raw().len()
            + self.trailing.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let country_prefix_len = broadcast_common::len::fit_bits(
            self.country_prefix.raw().len() as u64,
            2,
            "country_prefix",
        )? as u8;
        let intl_area_len = broadcast_common::len::fit_bits(
            self.international_area_code.raw().len() as u64,
            3,
            "international_area_code",
        )? as u8;
        let operator_code_len = broadcast_common::len::fit_bits(
            self.operator_code.raw().len() as u64,
            2,
            "operator_code",
        )? as u8;
        let national_area_len = broadcast_common::len::fit_bits(
            self.national_area_code.raw().len() as u64,
            3,
            "national_area_code",
        )? as u8;
        let core_number_len =
            broadcast_common::len::fit_bits(self.core_number.raw().len() as u64, 4, "core_number")?
                as u8;
        buf[2] = (broadcast_common::len::fit_mask(
            self.reserved_future_use_before_foreign_availability,
            BYTE0_RESERVED_MASK >> BYTE0_RESERVED_SHIFT,
            "reserved_future_use_before_foreign_availability",
        )? << BYTE0_RESERVED_SHIFT)
            | if self.foreign_availability {
                FOREIGN_AVAIL_MASK
            } else {
                0
            }
            | broadcast_common::len::fit_mask(
                self.connection_type,
                CONNECTION_TYPE_MASK,
                "connection_type",
            )?;
        buf[3] = (broadcast_common::len::fit_mask(
            self.reserved_future_use_before_country_prefix_length,
            BYTE1_RESERVED_MASK >> BYTE1_RESERVED_SHIFT,
            "reserved_future_use_before_country_prefix_length",
        )? << BYTE1_RESERVED_SHIFT)
            | (country_prefix_len << 5)
            | (intl_area_len << 2)
            | operator_code_len;
        buf[4] = (broadcast_common::len::fit_mask(
            self.reserved_future_use_before_national_area_code_length,
            BYTE2_RESERVED_MASK >> BYTE2_RESERVED_SHIFT,
            "reserved_future_use_before_national_area_code_length",
        )? << BYTE2_RESERVED_SHIFT)
            | (national_area_len << 4)
            | core_number_len;
        let mut pos = HEADER_LEN + FIXED_LEN;
        for field in [
            self.country_prefix.raw(),
            self.international_area_code.raw(),
            self.operator_code.raw(),
            self.national_area_code.raw(),
            self.core_number.raw(),
        ] {
            buf[pos..pos + field.len()].copy_from_slice(field);
            pos += field.len();
        }
        buf[pos..pos + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for TelephoneDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TELEPHONE";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TelephoneDescriptor<'static> {
        TelephoneDescriptor::new(
            true,
            0x05,
            DvbText::new(b"44"),
            DvbText::new(b"171"),
            DvbText::new(b"01"),
            DvbText::new(b"207"),
            DvbText::new(b"123456"),
        )
    }

    #[test]
    fn parse_extracts_all_fields() {
        let d = sample();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let p = TelephoneDescriptor::parse(&buf).unwrap();
        assert!(p.foreign_availability);
        assert_eq!(p.connection_type, 0x05);
        assert_eq!(p.country_prefix.raw(), b"44");
        assert_eq!(p.international_area_code.raw(), b"171");
        assert_eq!(p.operator_code.raw(), b"01");
        assert_eq!(p.national_area_code.raw(), b"207");
        assert_eq!(p.core_number.raw(), b"123456");
    }

    #[test]
    fn parse_minimal_no_chars() {
        let bytes = [TAG, 3, 0x20, 0x00, 0x00];
        let d = TelephoneDescriptor::parse(&bytes).unwrap();
        assert!(d.foreign_availability);
        assert!(d.country_prefix.raw().is_empty());
        assert!(d.core_number.raw().is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TelephoneDescriptor::parse(&[0x58, 3, 0, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x58, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let bytes = [TAG, 3, 0x20, 0x00];
        assert!(matches!(
            TelephoneDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_below_fixed() {
        let bytes = [TAG, 2, 0x20, 0x00];
        assert!(matches!(
            TelephoneDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_char_lengths_overrun() {
        let bytes = [TAG, 3, 0x20, 0x60, 0x00];
        assert!(matches!(
            TelephoneDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = sample();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TelephoneDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_emits_reserved_ones() {
        let d = TelephoneDescriptor::new(
            false,
            0,
            DvbText::new(b""),
            DvbText::new(b""),
            DvbText::new(b""),
            DvbText::new(b""),
            DvbText::new(b""),
        );
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[2] & BYTE0_RESERVED_MASK, BYTE0_RESERVED_MASK);
        assert_eq!(buf[3] & BYTE1_RESERVED_MASK, BYTE1_RESERVED_MASK);
        assert_eq!(buf[4] & BYTE2_RESERVED_MASK, BYTE2_RESERVED_MASK);
    }

    /// Every combination of the three reserved regions (2+1+1 bits) re-serializes
    /// byte-identically; hand-built values carry the default.
    #[test]
    fn reserved_preserved_for_every_combination() {
        for r0 in 0u8..=3 {
            for r1 in 0u8..=1 {
                for r2 in 0u8..=1 {
                    let bytes = [
                        TAG,
                        10,
                        (r0 << 6) | 0x05,
                        (r1 << 7) | 0x14,
                        (r2 << 7) | 0x11,
                        b'1',
                        b'7',
                        b'1',
                        b'2',
                        b'3',
                        b'9',
                        b'8',
                    ];
                    let d = TelephoneDescriptor::parse(&bytes).unwrap();
                    assert_eq!(d.reserved_future_use_before_foreign_availability, r0);
                    assert_eq!(d.reserved_future_use_before_country_prefix_length, r1);
                    assert_eq!(d.reserved_future_use_before_national_area_code_length, r2);
                    let mut buf = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut buf).unwrap();
                    assert_eq!(buf, bytes);
                }
            }
        }
    }

    #[test]
    fn default_is_spec_value() {
        let d = sample();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[2] >> 6, 0b11);
        assert_eq!(buf[3] >> 7, 1);
        assert_eq!(buf[4] >> 7, 1);
    }

    #[test]
    fn serialize_rejects_over_range_field() {
        let d = TelephoneDescriptor::new(
            false,
            0,
            DvbText::new(b""),
            DvbText::new(b""),
            DvbText::new(b"1234"),
            DvbText::new(b""),
            DvbText::new(b""),
        );
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_stable() {
        let d = sample();
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("connection_type"));
        assert!(json.contains("foreign_availability"));
        assert_eq!(json, serde_json::to_string(&sample()).unwrap());
    }
}
