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
use dvb_common::{Parse, Serialize};

/// Descriptor tag for telephone_descriptor.
pub const TAG: u8 = 0x57;
const HEADER_LEN: usize = 2;
/// Three fixed flag/length bytes precede the char loops.
const FIXED_LEN: usize = 3;

const FOREIGN_AVAIL_MASK: u8 = 0x20;
const CONNECTION_TYPE_MASK: u8 = 0x1F;
const BYTE0_RESERVED: u8 = 0xC0;
const BYTE1_RESERVED: u8 = 0x80;
const BYTE2_RESERVED: u8 = 0x80;

const MAX_COUNTRY_PREFIX: usize = 0x03;
const MAX_INTL_AREA: usize = 0x07;
const MAX_OPERATOR: usize = 0x03;
const MAX_NATIONAL_AREA: usize = 0x07;
const MAX_CORE_NUMBER: usize = 0x0F;

/// Telephone Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TelephoneDescriptor<'a> {
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
            foreign_availability,
            connection_type,
            country_prefix: DvbText::new(country_prefix),
            international_area_code: DvbText::new(international_area_code),
            operator_code: DvbText::new(operator_code),
            national_area_code: DvbText::new(national_area_code),
            core_number: DvbText::new(core_number),
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
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.country_prefix.raw().len() > MAX_COUNTRY_PREFIX
            || self.international_area_code.raw().len() > MAX_INTL_AREA
            || self.operator_code.raw().len() > MAX_OPERATOR
            || self.national_area_code.raw().len() > MAX_NATIONAL_AREA
            || self.core_number.raw().len() > MAX_CORE_NUMBER
        {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "telephone char-field exceeds its length-field capacity",
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        buf[2] = BYTE0_RESERVED
            | if self.foreign_availability {
                FOREIGN_AVAIL_MASK
            } else {
                0
            }
            | (self.connection_type & CONNECTION_TYPE_MASK);
        buf[3] = BYTE1_RESERVED
            | ((self.country_prefix.raw().len() as u8 & 0x03) << 5)
            | ((self.international_area_code.raw().len() as u8 & 0x07) << 2)
            | (self.operator_code.raw().len() as u8 & 0x03);
        buf[4] = BYTE2_RESERVED
            | ((self.national_area_code.raw().len() as u8 & 0x07) << 4)
            | (self.core_number.raw().len() as u8 & 0x0F);
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
        TelephoneDescriptor {
            foreign_availability: true,
            connection_type: 0x05,
            country_prefix: DvbText::new(b"44"),
            international_area_code: DvbText::new(b"171"),
            operator_code: DvbText::new(b"01"),
            national_area_code: DvbText::new(b"207"),
            core_number: DvbText::new(b"123456"),
        }
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
        let d = TelephoneDescriptor {
            foreign_availability: false,
            connection_type: 0,
            country_prefix: DvbText::new(b""),
            international_area_code: DvbText::new(b""),
            operator_code: DvbText::new(b""),
            national_area_code: DvbText::new(b""),
            core_number: DvbText::new(b""),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[2] & BYTE0_RESERVED, BYTE0_RESERVED);
        assert_eq!(buf[3] & BYTE1_RESERVED, BYTE1_RESERVED);
        assert_eq!(buf[4] & BYTE2_RESERVED, BYTE2_RESERVED);
    }

    #[test]
    fn serialize_rejects_over_range_field() {
        let d = TelephoneDescriptor {
            foreign_availability: false,
            connection_type: 0,
            country_prefix: DvbText::new(b""),
            international_area_code: DvbText::new(b""),
            operator_code: DvbText::new(b"1234"),
            national_area_code: DvbText::new(b""),
            core_number: DvbText::new(b""),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
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
