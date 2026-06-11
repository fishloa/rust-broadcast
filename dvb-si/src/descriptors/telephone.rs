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
//! Each *_char field is kept as a borrowed `&[u8]`; the four/three/two-bit
//! length fields are derived from slice length on serialize and ERROR on
//! over-range rather than truncating (crate idiom).

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for telephone_descriptor.
pub const TAG: u8 = 0x57;
const HEADER_LEN: usize = 2;
/// Three fixed flag/length bytes precede the char loops.
const FIXED_LEN: usize = 3;

const FOREIGN_AVAIL_MASK: u8 = 0x20; // byte0 bit 5
const CONNECTION_TYPE_MASK: u8 = 0x1F; // byte0 bits 0..4
const BYTE0_RESERVED: u8 = 0xC0; // byte0 bits 7..6 reserved -> 1s
const BYTE1_RESERVED: u8 = 0x80; // byte1 bit 7 reserved -> 1
const BYTE2_RESERVED: u8 = 0x80; // byte2 bit 7 reserved -> 1

// Maxima implied by each length field's bit-width.
const MAX_COUNTRY_PREFIX: usize = 0x03; // 2 bits
const MAX_INTL_AREA: usize = 0x07; // 3 bits
const MAX_OPERATOR: usize = 0x03; // 2 bits
const MAX_NATIONAL_AREA: usize = 0x07; // 3 bits
const MAX_CORE_NUMBER: usize = 0x0F; // 4 bits

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
    pub country_prefix: &'a [u8],
    /// international_area_code_char bytes (≤ 7).
    pub international_area_code: &'a [u8],
    /// operator_code_char bytes (≤ 3).
    pub operator_code: &'a [u8],
    /// national_area_code_char bytes (≤ 7).
    pub national_area_code: &'a [u8],
    /// core_number_char bytes (≤ 15).
    pub core_number: &'a [u8],
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
        // reserved bits ignored on parse (EN 300 468 §5.1).
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
            country_prefix,
            international_area_code,
            operator_code,
            national_area_code,
            core_number,
        })
    }
}

impl Serialize for TelephoneDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_LEN
            + self.country_prefix.len()
            + self.international_area_code.len()
            + self.operator_code.len()
            + self.national_area_code.len()
            + self.core_number.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // Per-field length fields: error on over-range rather than truncate.
        if self.country_prefix.len() > MAX_COUNTRY_PREFIX
            || self.international_area_code.len() > MAX_INTL_AREA
            || self.operator_code.len() > MAX_OPERATOR
            || self.national_area_code.len() > MAX_NATIONAL_AREA
            || self.core_number.len() > MAX_CORE_NUMBER
        {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "telephone char-field exceeds its length-field capacity",
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        // byte0: reserved(2)=1s | foreign_availability(1) | connection_type(5)
        buf[2] = BYTE0_RESERVED
            | if self.foreign_availability {
                FOREIGN_AVAIL_MASK
            } else {
                0
            }
            | (self.connection_type & CONNECTION_TYPE_MASK);
        // byte1: reserved(1)=1 | cp_len(2) | iac_len(3) | op_len(2)
        buf[3] = BYTE1_RESERVED
            | ((self.country_prefix.len() as u8 & 0x03) << 5)
            | ((self.international_area_code.len() as u8 & 0x07) << 2)
            | (self.operator_code.len() as u8 & 0x03);
        // byte2: reserved(1)=1 | nac_len(3) | core_len(4)
        buf[4] = BYTE2_RESERVED
            | ((self.national_area_code.len() as u8 & 0x07) << 4)
            | (self.core_number.len() as u8 & 0x0F);
        let mut pos = HEADER_LEN + FIXED_LEN;
        for field in [
            self.country_prefix,
            self.international_area_code,
            self.operator_code,
            self.national_area_code,
            self.core_number,
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
            country_prefix: b"44",
            international_area_code: b"171",
            operator_code: b"01",
            national_area_code: b"207",
            core_number: b"123456",
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
        assert_eq!(p.country_prefix, b"44");
        assert_eq!(p.international_area_code, b"171");
        assert_eq!(p.operator_code, b"01");
        assert_eq!(p.national_area_code, b"207");
        assert_eq!(p.core_number, b"123456");
    }

    #[test]
    fn parse_minimal_no_chars() {
        // length=3, all char-field lengths 0
        let bytes = [TAG, 3, 0x20, 0x00, 0x00];
        let d = TelephoneDescriptor::parse(&bytes).unwrap();
        assert!(d.foreign_availability);
        assert!(d.country_prefix.is_empty());
        assert!(d.core_number.is_empty());
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
        // declares length 3 but only 2 body bytes present
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
        // country_prefix_length=3 (byte1 bits 6..5 = 11) but body only has the
        // 3 fixed bytes — the char loop would overrun.
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
            country_prefix: b"",
            international_area_code: b"",
            operator_code: b"",
            national_area_code: b"",
            core_number: b"",
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // byte0 reserved bits 7..6 set; byte1 & byte2 reserved bit 7 set
        assert_eq!(buf[2] & BYTE0_RESERVED, BYTE0_RESERVED);
        assert_eq!(buf[3] & BYTE1_RESERVED, BYTE1_RESERVED);
        assert_eq!(buf[4] & BYTE2_RESERVED, BYTE2_RESERVED);
    }

    #[test]
    fn serialize_rejects_over_range_field() {
        // operator_code length 4 exceeds its 2-bit field (max 3).
        let d = TelephoneDescriptor {
            foreign_availability: false,
            connection_type: 0,
            country_prefix: b"",
            international_area_code: b"",
            operator_code: b"1234",
            national_area_code: b"",
            core_number: b"",
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
        // Borrowed-byte fields cannot deserialize from a JSON array (serde_json
        // requires a borrowed-str for &[u8]); assert the Serialize half is
        // stable, matching the other borrowed descriptors (e.g.
        // content_identifier) in this crate.
        let d = sample();
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("connection_type"));
        assert!(json.contains("foreign_availability"));
        assert_eq!(json, serde_json::to_string(&sample()).unwrap());
    }
}
