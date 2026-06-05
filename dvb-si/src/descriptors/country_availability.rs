//! Country Availability Descriptor — ETSI EN 300 468 §6.2.10 (tag 0x49).
//!
//! Table 30 (PDF p. 70). Carried in SDT/BAT descriptor loops. A flag byte
//! (`country_availability_flag` MSB + 7 reserved bits) followed by a loop of
//! 3-byte ISO 3166 country codes. When the flag is set, the service is
//! intended for reception in the listed countries; when clear, in all
//! countries EXCEPT those listed.

use crate::error::{Error, Result};
use crate::text::LangCode;
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for country_availability_descriptor.
pub const TAG: u8 = 0x49;
const HEADER_LEN: usize = 2;
const FLAG_LEN: usize = 1;
const COUNTRY_CODE_LEN: usize = 3;
const MIN_BODY_LEN: usize = FLAG_LEN;
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;
/// country_availability_flag occupies bit 7 (MSB) of the flag byte.
const AVAILABILITY_FLAG_MASK: u8 = 0x80;
/// Lower 7 bits are reserved_future_use — emitted as 1s on serialize.
const RESERVED_MASK: u8 = 0x7F;

/// Country Availability Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CountryAvailabilityDescriptor {
    /// true = available in the listed countries; false = available everywhere
    /// EXCEPT the listed countries.
    pub country_availability_flag: bool,
    /// ISO 3166 alpha-3 country codes in wire order.
    pub country_codes: Vec<LangCode>,
}

impl<'a> Parse<'a> for CountryAvailabilityDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "CountryAvailabilityDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for country_availability_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length < MIN_BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "country_availability_descriptor missing flag byte",
            });
        }
        if (length - FLAG_LEN) % COUNTRY_CODE_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "country_code loop length must be a multiple of 3",
            });
        }
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "CountryAvailabilityDescriptor body",
            });
        }
        let flags = bytes[HEADER_LEN];
        // reserved_future_use bits ignored on parse (EN 300 468 §5.1).
        let country_availability_flag = flags & AVAILABILITY_FLAG_MASK != 0;
        let loop_body = &bytes[HEADER_LEN + FLAG_LEN..end];
        let mut country_codes = Vec::with_capacity(loop_body.len() / COUNTRY_CODE_LEN);
        for chunk in loop_body.chunks_exact(COUNTRY_CODE_LEN) {
            country_codes.push(LangCode([chunk[0], chunk[1], chunk[2]]));
        }
        Ok(Self {
            country_availability_flag,
            country_codes,
        })
    }
}

impl Serialize for CountryAvailabilityDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FLAG_LEN + COUNTRY_CODE_LEN * self.country_codes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = FLAG_LEN + COUNTRY_CODE_LEN * self.country_codes.len();
        // 8-bit descriptor_length field: error rather than silently truncate.
        if body_len > MAX_BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: MAX_BODY_LEN,
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        // reserved_future_use bits emitted as 1s (EN 300 468 §5.1).
        let flag_bit = if self.country_availability_flag {
            AVAILABILITY_FLAG_MASK
        } else {
            0
        };
        buf[HEADER_LEN] = flag_bit | RESERVED_MASK;
        let mut pos = HEADER_LEN + FLAG_LEN;
        for code in &self.country_codes {
            buf[pos..pos + COUNTRY_CODE_LEN].copy_from_slice(&code.0);
            pos += COUNTRY_CODE_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for CountryAvailabilityDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (FLAG_LEN + COUNTRY_CODE_LEN * self.country_codes.len()) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for CountryAvailabilityDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "COUNTRY_AVAILABILITY";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_available_with_countries() {
        // flag=1 (0x80) + reserved 1s ignored, two country codes
        let bytes = [TAG, 7, 0xFF, b'G', b'B', b'R', b'F', b'R', b'A'];
        let d = CountryAvailabilityDescriptor::parse(&bytes).unwrap();
        assert!(d.country_availability_flag);
        assert_eq!(d.country_codes, vec![LangCode(*b"GBR"), LangCode(*b"FRA")]);
    }

    #[test]
    fn parse_flag_clear() {
        let bytes = [TAG, 4, 0x7F, b'D', b'E', b'U'];
        let d = CountryAvailabilityDescriptor::parse(&bytes).unwrap();
        assert!(!d.country_availability_flag);
        assert_eq!(d.country_codes, vec![LangCode(*b"DEU")]);
    }

    #[test]
    fn parse_flag_only_no_countries() {
        let bytes = [TAG, 1, 0x80];
        let d = CountryAvailabilityDescriptor::parse(&bytes).unwrap();
        assert!(d.country_availability_flag);
        assert!(d.country_codes.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            CountryAvailabilityDescriptor::parse(&[0x4A, 1, 0x80]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x4A, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declares 7 body bytes, only 4 present
        let bytes = [TAG, 7, 0x80, b'G', b'B'];
        assert!(matches!(
            CountryAvailabilityDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_loop_not_multiple_of_3() {
        // 1 flag byte + 2 loop bytes — loop not a multiple of 3
        let bytes = [TAG, 3, 0x80, b'G', b'B'];
        assert!(matches!(
            CountryAvailabilityDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_zero_length() {
        let bytes = [TAG, 0];
        assert!(matches!(
            CountryAvailabilityDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = CountryAvailabilityDescriptor {
            country_availability_flag: true,
            country_codes: vec![LangCode(*b"GBR"), LangCode(*b"IRL")],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(CountryAvailabilityDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_emits_reserved_ones() {
        let d = CountryAvailabilityDescriptor {
            country_availability_flag: false,
            country_codes: vec![],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // flag clear, reserved bits all 1 -> 0x7F
        assert_eq!(buf[HEADER_LEN], 0x7F);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = CountryAvailabilityDescriptor {
            country_availability_flag: true,
            country_codes: vec![LangCode(*b"FRA")],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
