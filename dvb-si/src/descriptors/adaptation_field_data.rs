//! Adaptation Field Data Descriptor — ETSI EN 300 468 §6.2.1 (tag 0x70, Table 13, PDF p. 54).
//!
//! Carried inside the PMT ES_info loop. Fixed 1-byte body: a bit-flag field
//! `adaptation_field_data_identifier` signalling which data fields are carried
//! in the adaptation field private_data (Table 14).

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for adaptation_field_data_descriptor.
pub const TAG: u8 = 0x70;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed body length: one identifier flag byte.
pub const BODY_LEN: usize = 1;

/// Table 14 bit positions (0-based from LSB): `b₁` = bit 0, `b₂` = bit 1, …
const ANNOUNCEMENT_SWITCHING_DATA: u8 = 1 << 0;
const AU_INFORMATION: u8 = 1 << 1;
const PVR_ASSIST_INFORMATION: u8 = 1 << 2;

/// Decoded adaptation field data flags — ETSI EN 300 468 Table 14.
///
/// Bit numbering per the spec: `b₁` (LSB, transmitted last per §5.1.6)
/// through `b₈` (MSB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptationFieldDataFlags {
    /// Announcement switching data (`b₁` = bit 0).
    pub announcement_switching_data: bool,
    /// AU information (`b₂` = bit 1).
    pub au_information: bool,
    /// PVR assist information (`b₃` = bit 2).
    pub pvr_assist_information: bool,
}

/// Adaptation Field Data Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AdaptationFieldDataDescriptor {
    /// 8-bit adaptation_field_data_identifier flag field (Table 14).
    pub adaptation_field_data_identifier: u8,
}

impl AdaptationFieldDataDescriptor {
    /// Decodes the `adaptation_field_data_identifier` flag byte into named
    /// booleans per ETSI EN 300 468 Table 14.
    #[must_use]
    pub fn flags(&self) -> AdaptationFieldDataFlags {
        let b = self.adaptation_field_data_identifier;
        AdaptationFieldDataFlags {
            announcement_switching_data: (b & ANNOUNCEMENT_SWITCHING_DATA) != 0,
            au_information: (b & AU_INFORMATION) != 0,
            pvr_assist_information: (b & PVR_ASSIST_INFORMATION) != 0,
        }
    }
}

impl<'a> Parse<'a> for AdaptationFieldDataDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AdaptationFieldDataDescriptor",
            "unexpected tag for adaptation_field_data_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "adaptation_field_data_descriptor length must be exactly 1",
            });
        }
        Ok(Self {
            adaptation_field_data_identifier: body[0],
        })
    }
}

impl Serialize for AdaptationFieldDataDescriptor {
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
        buf[HEADER_LEN] = self.adaptation_field_data_identifier;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for AdaptationFieldDataDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ADAPTATION_FIELD_DATA";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_identifier() {
        let bytes = [TAG, 1, 0x07];
        let d = AdaptationFieldDataDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.adaptation_field_data_identifier, 0x07);
    }

    #[test]
    fn flags_decode_all_set() {
        // bits 0,1,2 set → 0b0000_0111 = 0x07
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0x07,
        };
        let f = d.flags();
        assert!(f.announcement_switching_data);
        assert!(f.au_information);
        assert!(f.pvr_assist_information);
    }

    #[test]
    fn flags_decode_none_set() {
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0x00,
        };
        let f = d.flags();
        assert!(!f.announcement_switching_data);
        assert!(!f.au_information);
        assert!(!f.pvr_assist_information);
    }

    #[test]
    fn flags_decode_au_only() {
        // bit 1 only → 0b0000_0010 = 0x02
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0x02,
        };
        let f = d.flags();
        assert!(!f.announcement_switching_data);
        assert!(f.au_information);
        assert!(!f.pvr_assist_information);
    }

    #[test]
    fn flags_decode_pvr_only() {
        // bit 2 only → 0b0000_0100 = 0x04
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0x04,
        };
        let f = d.flags();
        assert!(!f.announcement_switching_data);
        assert!(!f.au_information);
        assert!(f.pvr_assist_information);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            AdaptationFieldDataDescriptor::parse(&[0x71, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x71, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(matches!(
            AdaptationFieldDataDescriptor::parse(&[TAG, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_body() {
        assert!(matches!(
            AdaptationFieldDataDescriptor::parse(&[TAG, 1]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0x05,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0x05]);
        assert_eq!(AdaptationFieldDataDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0,
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
        let d = AdaptationFieldDataDescriptor {
            adaptation_field_data_identifier: 0x05,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
