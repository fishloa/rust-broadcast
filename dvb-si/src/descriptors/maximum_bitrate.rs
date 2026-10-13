//! Maximum Bitrate Descriptor — ISO/IEC 13818-1 §2.6.26 (tag 0x0E).
//!
//! Indicates the maximum bitrate of the associated elementary stream
//! in units of 50 bytes/second (§2.6.27).

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for maximum_bitrate_descriptor.
pub const TAG: u8 = 0x0E;
/// `maximum_bitrate` is a 22-bit uimsbf (units of 50 bytes/s).
const MAXIMUM_BITRATE_BITS: u32 = 22;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 3;
/// `reserved(2)` ahead of `maximum_bitrate` (Table 2-67), emitted as `11`: H.222.0
/// reserved bits are 1 (r03-W7; confirmed by the TSDuck-compiled oracle PMT).
const RESERVED_2: u8 = 0xC0;

/// Maximum Bitrate Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MaximumBitrateDescriptor {
    /// Maximum bitrate in units of 50 bytes/second.
    pub maximum_bitrate: u32,
}

impl<'a> Parse<'a> for MaximumBitrateDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MaximumBitrateDescriptor",
            "unexpected tag for maximum_bitrate_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "maximum_bitrate_descriptor length must equal 3",
            });
        }
        let maximum_bitrate =
            ((u32::from(body[0]) & 0x3F) << 16) | (u32::from(body[1]) << 8) | u32::from(body[2]);
        Ok(Self { maximum_bitrate })
    }
}

impl Serialize for MaximumBitrateDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let mb = broadcast_common::len::fit_bits(
            u64::from(self.maximum_bitrate),
            MAXIMUM_BITRATE_BITS,
            "maximum_bitrate",
        )?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        let mb = mb as u32;
        buf[HEADER_LEN] = RESERVED_2 | ((mb >> 16) & 0x3F) as u8;
        buf[HEADER_LEN + 1] = ((mb >> 8) & 0xFF) as u8;
        buf[HEADER_LEN + 2] = (mb & 0xFF) as u8;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for MaximumBitrateDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MAXIMUM_BITRATE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            3,
            0b00_111111,
            0xAB,
            0xCD, // reserved=0, maximum_bitrate=0x3FABCD
        ];
        let d = MaximumBitrateDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.maximum_bitrate, 0x3FABCD);
    }

    #[test]
    fn serialize_round_trip() {
        let d = MaximumBitrateDescriptor {
            maximum_bitrate: 0x123456,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MaximumBitrateDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn max_value() {
        let d = MaximumBitrateDescriptor {
            maximum_bitrate: 0x3FFFFF, // 22-bit max
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MaximumBitrateDescriptor::parse(&buf).unwrap();
        assert_eq!(reparsed.maximum_bitrate, 0x3FFFFF);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MaximumBitrateDescriptor::parse(&[0x0F, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0F, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = MaximumBitrateDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MaximumBitrateDescriptor { maximum_bitrate: 0 };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_22bit_bitrate() {
        // Pre-fix `(mb >> 16) & 0x3F` silently dropped bits 22..32 (r03-W3).
        let d = MaximumBitrateDescriptor {
            maximum_bitrate: 1 << 22,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    /// r03-W7 — H.222.0 Table 2-67 (ISO/IEC 13818-1:2021). Evidence for reserved=1 is the TSDuck-compiled oracle
    /// PMT `tsduck-mpeg-simple-pmt.bin` (TSDuck only; no real broadcast capture carries it). (1) conformant bytes (reserved=1) round-trip byte-exactly;
    /// (2) the same descriptor with reserved=0 (non-conformant sender) parses to
    /// equal typed fields and re-serializes to the normalized reserved=1 form.
    #[test]
    fn reserved_bits_one_on_serialize_and_zero_input_normalized() {
        let conformant: &[u8] = &[TAG, 3, 0xFF, 0xAB, 0xCD];
        let zeroed: &[u8] = &[TAG, 3, 0x3F, 0xAB, 0xCD];
        let d = MaximumBitrateDescriptor::parse(conformant).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, conformant);

        let dz = MaximumBitrateDescriptor::parse(zeroed).unwrap();
        assert_eq!(dz, d, "typed fields must not depend on reserved bits");
        let mut outz = vec![0u8; dz.serialized_len()];
        dz.serialize_into(&mut outz).unwrap();
        assert_eq!(outz, conformant);
    }
}
