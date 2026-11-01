//! Country Availability Descriptor — ETSI EN 300 468 §6.2.10 (tag 0x49).
//!
//! Table 30 (PDF p. 70). Carried in SDT/BAT descriptor loops. A flag byte
//! (`country_availability_flag` MSB + 7 reserved bits) followed by a loop of
//! 3-byte ISO 3166 country codes. When the flag is set, the service is
//! intended for reception in the listed countries; when clear, in all
//! countries EXCEPT those listed.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

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
/// Lower 7 bits of the flag byte are `reserved_future_use`.
const RESERVED_MASK: u8 = 0x7F;
/// Spec default of `reserved_future_use`(7): all ones (EN 300 468 V1.3.1-V1.20.1,
/// audit `dvb-si/docs/reserved-bits-edition-audit.md` §6.1, class A: stable in
/// every edition).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 0x7F;

/// Country Availability Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CountryAvailabilityDescriptor {
    /// `reserved_future_use`(7) after the flag (7 bits, right-aligned).
    /// Captured on parse and re-emitted verbatim; [`Self::new`] defaults to
    /// [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use: u8,
    /// true = available in the listed countries; false = available everywhere
    /// EXCEPT the listed countries.
    pub country_availability_flag: bool,
    /// ISO 3166 alpha-3 country codes in wire order.
    pub country_codes: Vec<LangCode>,
}

impl CountryAvailabilityDescriptor {
    /// Build a descriptor with the spec-default reserved bits (all ones).
    #[must_use]
    pub const fn new(country_availability_flag: bool, country_codes: Vec<LangCode>) -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            country_availability_flag,
            country_codes,
        }
    }
}

impl<'a> Parse<'a> for CountryAvailabilityDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CountryAvailabilityDescriptor",
            "unexpected tag for country_availability_descriptor",
        )?;
        if body.len() < MIN_BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "country_availability_descriptor missing flag byte",
            });
        }
        if !(body.len() - FLAG_LEN).is_multiple_of(COUNTRY_CODE_LEN) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "country_code loop length must be a multiple of 3",
            });
        }
        let flags = body[0];
        let country_availability_flag = flags & AVAILABILITY_FLAG_MASK != 0;
        let loop_body = &body[FLAG_LEN..];
        let mut country_codes = Vec::with_capacity(loop_body.len() / COUNTRY_CODE_LEN);
        for chunk in loop_body.chunks_exact(COUNTRY_CODE_LEN) {
            country_codes.push(LangCode([chunk[0], chunk[1], chunk[2]]));
        }
        Ok(Self {
            reserved_future_use: flags & RESERVED_MASK,
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
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "country_availability_descriptor body exceeds 255 bytes",
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, body_len)?;
        // reserved_future_use preserved from parse (default all ones).
        let flag_bit = if self.country_availability_flag {
            AVAILABILITY_FLAG_MASK
        } else {
            0
        };
        buf[HEADER_LEN] = flag_bit
            | broadcast_common::len::fit_mask(
                self.reserved_future_use,
                RESERVED_MASK,
                "reserved_future_use",
            )?;
        let mut pos = HEADER_LEN + FLAG_LEN;
        for code in &self.country_codes {
            buf[pos..pos + COUNTRY_CODE_LEN].copy_from_slice(&code.0);
            pos += COUNTRY_CODE_LEN;
        }
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for CountryAvailabilityDescriptor {
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
        let d =
            CountryAvailabilityDescriptor::new(true, vec![LangCode(*b"GBR"), LangCode(*b"IRL")]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(CountryAvailabilityDescriptor::parse(&buf).unwrap(), d);
    }

    /// Every reserved_future_use(7) value (0..=0x7F) re-serializes verbatim;
    /// parsed values are preserved, only hand-built ones carry the default.
    #[test]
    fn reserved_preserved_for_every_value() {
        for r in 0u8..=0x7F {
            for flag in [0u8, 0x80] {
                let bytes = [TAG, 4, flag | r, b'G', b'B', b'R'];
                let d = CountryAvailabilityDescriptor::parse(&bytes).unwrap();
                assert_eq!(d.reserved_future_use, r);
                let mut buf = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut buf).unwrap();
                assert_eq!(buf, bytes);
            }
        }
    }

    #[test]
    fn default_is_spec_value() {
        let d = CountryAvailabilityDescriptor::new(true, vec![]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0xFF]);
    }

    #[test]
    fn serialize_emits_reserved_ones() {
        let d = CountryAvailabilityDescriptor::new(false, vec![]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // flag clear, reserved bits all 1 -> 0x7F
        assert_eq!(buf[HEADER_LEN], 0x7F);
    }
}
