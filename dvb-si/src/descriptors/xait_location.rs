//! XAIT Location Descriptor — ETSI TS 102 727 §10.17.6, Table 96 (tag 0x7D).
//!
//! Points at the service carrying the XAIT (deprecated MHP discovery
//! mechanism). Per the MHP PDF (etsi_ts_102_727_v01.01.01, p. 185, Table 96)
//! the body is a fixed 5 bytes:
//!
//! ```text
//! xait_original_network_id(16) + xait_service_id(16)
//!   + xait_version_number(5) + xait_update_policy(3)
//! ```
//!
//! xait_update_policy values (Table 97, p. 185): 0 = reload immediately on
//! version change, 1 = ignore version changes until reset, 2..=7 reserved.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for xait_location_descriptor.
pub const TAG: u8 = 0x7D;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 5;

/// Largest representable 5-bit xait_version_number.
const VERSION_MAX: u8 = 0x1F;
/// Largest representable 3-bit xait_update_policy.
const UPDATE_POLICY_MAX: u8 = 0x07;

/// XAIT Location Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct XaitLocationDescriptor {
    /// 16-bit original_network_id of the service carrying the XAIT.
    pub xait_original_network_id: u16,
    /// 16-bit service_id of the service carrying the XAIT.
    pub xait_service_id: u16,
    /// 5-bit version number of the referenced XAIT.
    pub xait_version_number: u8,
    /// 3-bit update policy (Table 97).
    pub xait_update_policy: u8,
}

impl<'a> Parse<'a> for XaitLocationDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "XaitLocationDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for xait_location_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "XaitLocationDescriptor body",
            });
        }
        if length < BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "xait_location_descriptor body shorter than 5 bytes",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let xait_original_network_id = u16::from_be_bytes([body[0], body[1]]);
        let xait_service_id = u16::from_be_bytes([body[2], body[3]]);
        let xait_version_number = (body[4] >> 3) & VERSION_MAX;
        let xait_update_policy = body[4] & UPDATE_POLICY_MAX;
        Ok(Self {
            xait_original_network_id,
            xait_service_id,
            xait_version_number,
            xait_update_policy,
        })
    }
}

impl Serialize for XaitLocationDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.xait_version_number > VERSION_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "xait_version_number exceeds 5 bits",
            });
        }
        if self.xait_update_policy > UPDATE_POLICY_MAX {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "xait_update_policy exceeds 3 bits",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN as u8;
        buf[2..4].copy_from_slice(&self.xait_original_network_id.to_be_bytes());
        buf[4..6].copy_from_slice(&self.xait_service_id.to_be_bytes());
        buf[6] = ((self.xait_version_number & VERSION_MAX) << 3)
            | (self.xait_update_policy & UPDATE_POLICY_MAX);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for XaitLocationDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for XaitLocationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "XAIT_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        // onid=0x2024, sid=0x1234, version=5, policy=1.
        let bytes = [TAG, 5, 0x20, 0x24, 0x12, 0x34, (5 << 3) | 1];
        let d = XaitLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.xait_original_network_id, 0x2024);
        assert_eq!(d.xait_service_id, 0x1234);
        assert_eq!(d.xait_version_number, 5);
        assert_eq!(d.xait_update_policy, 1);
    }

    #[test]
    fn parse_ignores_trailing_bytes() {
        let bytes = [TAG, 6, 0x00, 0x01, 0x00, 0x02, 0x00, 0xFF];
        let d = XaitLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.xait_original_network_id, 0x0001);
        assert_eq!(d.xait_service_id, 0x0002);
        assert_eq!(d.xait_version_number, 0);
        assert_eq!(d.xait_update_policy, 0);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x7C, 5, 0, 0, 0, 0, 0];
        assert!(matches!(
            XaitLocationDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7C, .. }
        ));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        let bytes = [TAG, 4, 0, 0, 0, 0];
        assert!(matches!(
            XaitLocationDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 5, 0, 0, 0];
        assert!(matches!(
            XaitLocationDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = XaitLocationDescriptor {
            xait_original_network_id: 0x233A,
            xait_service_id: 0x4470,
            xait_version_number: 0x1F,
            xait_update_policy: 0,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(XaitLocationDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_version_over_range() {
        let d = XaitLocationDescriptor {
            xait_original_network_id: 0,
            xait_service_id: 0,
            xait_version_number: 0x20,
            xait_update_policy: 0,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_rejects_update_policy_over_range() {
        let d = XaitLocationDescriptor {
            xait_original_network_id: 0,
            xait_service_id: 0,
            xait_version_number: 0,
            xait_update_policy: 0x08,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = XaitLocationDescriptor {
            xait_original_network_id: 0x0001,
            xait_service_id: 0x0539,
            xait_version_number: 3,
            xait_update_policy: 1,
        };
        let j = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
    }
}
