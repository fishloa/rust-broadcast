//! Service Move Descriptor — ETSI EN 300 468 §6.2.37 (tag 0x60).
//!
//! Table 92 (PDF p. 102). Carried in the SDT when a service is being moved to a
//! new location; gives the new (original_network_id, transport_stream_id,
//! service_id) triple. Fixed 6-byte payload.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for service_move_descriptor.
pub const TAG: u8 = 0x60;
const HEADER_LEN: usize = 2;
/// Fixed payload length: three 16-bit identifiers (EN 300 468 Table 92).
const BODY_LEN: u8 = 6;

/// Service Move Descriptor (tag 0x60).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceMoveDescriptor {
    /// original_network_id of the service's new location.
    pub new_original_network_id: u16,
    /// transport_stream_id of the service's new location.
    pub new_transport_stream_id: u16,
    /// service_id of the service at its new location.
    pub new_service_id: u16,
}

impl<'a> Parse<'a> for ServiceMoveDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ServiceMoveDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for service_move_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let total = HEADER_LEN + length;
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "ServiceMoveDescriptor body",
            });
        }
        if length != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_move_descriptor length must equal 6",
            });
        }
        let b = HEADER_LEN;
        Ok(Self {
            new_original_network_id: u16::from_be_bytes([bytes[b], bytes[b + 1]]),
            new_transport_stream_id: u16::from_be_bytes([bytes[b + 2], bytes[b + 3]]),
            new_service_id: u16::from_be_bytes([bytes[b + 4], bytes[b + 5]]),
        })
    }
}

impl Serialize for ServiceMoveDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
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
        buf[1] = BODY_LEN;
        let b = HEADER_LEN;
        buf[b..b + 2].copy_from_slice(&self.new_original_network_id.to_be_bytes());
        buf[b + 2..b + 4].copy_from_slice(&self.new_transport_stream_id.to_be_bytes());
        buf[b + 4..b + 6].copy_from_slice(&self.new_service_id.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ServiceMoveDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_triple() {
        let bytes = [TAG, 6, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03];
        let d = ServiceMoveDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.new_original_network_id, 0x0001);
        assert_eq!(d.new_transport_stream_id, 0x0002);
        assert_eq!(d.new_service_id, 0x0003);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err =
            ServiceMoveDescriptor::parse(&[0x61, 6, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x61, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = ServiceMoveDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        // length=6 but only 4 payload bytes present.
        let err = ServiceMoveDescriptor::parse(&[TAG, 6, 0, 1, 0, 2]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err =
            ServiceMoveDescriptor::parse(&[TAG, 5, 0, 1, 0, 2, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ServiceMoveDescriptor {
            new_original_network_id: 0x1234,
            new_transport_stream_id: 0x5678,
            new_service_id: 0x9ABC,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ServiceMoveDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ServiceMoveDescriptor {
            new_original_network_id: 1,
            new_transport_stream_id: 2,
            new_service_id: 3,
        };
        let mut tiny = [0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = ServiceMoveDescriptor {
            new_original_network_id: 0,
            new_transport_stream_id: 0,
            new_service_id: 0,
        };
        assert_eq!(d.descriptor_length(), 6);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = ServiceMoveDescriptor {
            new_original_network_id: 0x0001,
            new_transport_stream_id: 0x0002,
            new_service_id: 0x0003,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: ServiceMoveDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
