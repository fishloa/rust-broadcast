//! Ancillary Data Descriptor — ETSI EN 300 468 §6.2.3 (tag 0x6B, Table 15, PDF p. 55).
//!
//! Carried inside the PMT ES_info loop. Fixed 1-byte body: a bit-flag field
//! `ancillary_data_identifier` whose bits select which ancillary-data formats
//! are present (Table 16: DVD video AD, extended AD, announcement switching,
//! DAB AD, ScF-CRC, MPEG-4 AD, RDS-via-UECP). We carry the raw flag byte; the
//! bit meanings are defined by ETSI TS 101 154 and not interpreted here.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for ancillary_data_descriptor.
pub const TAG: u8 = 0x6B;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed body length: one identifier flag byte.
pub const BODY_LEN: usize = 1;

/// Ancillary Data Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncillaryDataDescriptor {
    /// 8-bit ancillary_data_identifier flag field (Table 16).
    pub ancillary_data_identifier: u8,
}

impl<'a> Parse<'a> for AncillaryDataDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AncillaryDataDescriptor",
            "unexpected tag for ancillary_data_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ancillary_data_descriptor length must be exactly 1",
            });
        }
        Ok(Self {
            ancillary_data_identifier: body[0],
        })
    }
}

impl Serialize for AncillaryDataDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN as u8;
        buf[HEADER_LEN] = self.ancillary_data_identifier;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for AncillaryDataDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ANCILLARY_DATA";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_identifier() {
        let bytes = [TAG, 1, 0x55];
        let d = AncillaryDataDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ancillary_data_identifier, 0x55);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            AncillaryDataDescriptor::parse(&[0x6C, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6C, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(matches!(
            AncillaryDataDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_body() {
        assert!(matches!(
            AncillaryDataDescriptor::parse(&[TAG, 1]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0xA3,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0xA3]);
        assert_eq!(AncillaryDataDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0,
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0xA3,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
