//! Linkage Descriptor — ETSI EN 300 468 §6.2.19 (tag 0x4A).
//!
//! Carried inside NIT (network linkage), BAT (bouquet linkage) and
//! SDT (service replacement / premiere hand-over).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for linkage_descriptor.
pub const TAG: u8 = 0x4A;
const HEADER_LEN: usize = 2;
const FIXED_FIELDS_LEN: usize = 7;

/// Linkage Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LinkageDescriptor<'a> {
    /// transport_stream_id of the linked-to TS.
    pub transport_stream_id: u16,
    /// original_network_id of the linked-to TS.
    pub original_network_id: u16,
    /// service_id of the linked-to service (0 if linkage is at the network or
    /// bouquet level).
    pub service_id: u16,
    /// linkage_type byte (ETSI Table 70): 0x01 information, 0x02 EPG,
    /// 0x03 CA_replacement, 0x04 TS_containing_complete_SI, 0x05 service_replacement,
    /// 0x06 data_broadcast, 0x07 RCS_map, 0x08 mobile_hand-over, etc.
    pub linkage_type: u8,
    /// Raw private_data_byte tail — interpretation depends on linkage_type.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for LinkageDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "LinkageDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for linkage_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "LinkageDescriptor body",
            });
        }
        if length < FIXED_FIELDS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "linkage_descriptor body shorter than minimum 7 bytes",
            });
        }
        let body_start = HEADER_LEN;
        let transport_stream_id = u16::from_be_bytes([bytes[body_start], bytes[body_start + 1]]);
        let original_network_id =
            u16::from_be_bytes([bytes[body_start + 2], bytes[body_start + 3]]);
        let service_id = u16::from_be_bytes([bytes[body_start + 4], bytes[body_start + 5]]);
        let linkage_type = bytes[body_start + 6];
        let private_data = &bytes[body_start + FIXED_FIELDS_LEN..end];
        Ok(Self {
            transport_stream_id,
            original_network_id,
            service_id,
            linkage_type,
            private_data,
        })
    }
}

impl Serialize for LinkageDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_FIELDS_LEN + self.private_data.len()
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
        buf[1] = (FIXED_FIELDS_LEN + self.private_data.len()) as u8;
        let body_start = HEADER_LEN;
        buf[body_start..body_start + 2].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[body_start + 2..body_start + 4]
            .copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[body_start + 4..body_start + 6].copy_from_slice(&self.service_id.to_be_bytes());
        buf[body_start + 6] = self.linkage_type;
        if !self.private_data.is_empty() {
            let private_start = body_start + FIXED_FIELDS_LEN;
            buf[private_start..private_start + self.private_data.len()]
                .copy_from_slice(self.private_data);
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for LinkageDescriptor<'a> {
    const TAG: u8 = TAG;

    fn descriptor_length(&self) -> u8 {
        (FIXED_FIELDS_LEN + self.private_data.len()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_tsid_onid_sid() {
        let bytes = [
            TAG, 0x09, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05, 0xAA, 0xBB,
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.transport_stream_id, 0x0001);
        assert_eq!(d.original_network_id, 0x0002);
        assert_eq!(d.service_id, 0x0003);
    }

    #[test]
    fn parse_extracts_linkage_type() {
        let bytes = [TAG, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x06];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.linkage_type, 0x06);
    }

    #[test]
    fn parse_preserves_raw_private_data() {
        let bytes = [
            TAG, 0x0A, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05, 0xAA, 0xBB, 0xCC,
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.private_data, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn parse_accepts_empty_private_data() {
        let bytes = [TAG, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = LinkageDescriptor::parse(&[0x4B, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x4B, .. }));
    }

    #[test]
    fn parse_rejects_body_shorter_than_seven() {
        let bytes = [TAG, 0x05, 0x00, 0x01, 0x00, 0x02, 0x00];
        let err = LinkageDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_truncated_buffer() {
        let err = LinkageDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip_no_private_data() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x1234,
            original_network_id: 0x5678,
            service_id: 0xABCD,
            linkage_type: 0x02,
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_with_private_data() {
        let private = [0xDE, 0xAD, 0xBE, 0xEF];
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x05,
            private_data: &private,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }
}
