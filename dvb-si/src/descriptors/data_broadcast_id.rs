//! Data Broadcast Id Descriptor — ETSI EN 300 468 §6.2.13 (tag 0x66).
//!
//! Table 32 (PDF p. 72). Identifies the data broadcast specification used by a
//! data component, plus a raw `id_selector_byte` tail whose interpretation
//! depends on the `data_broadcast_id` (see ETSI TS 101 162).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for data_broadcast_id_descriptor.
pub const TAG: u8 = 0x66;
const HEADER_LEN: usize = 2;
/// Fixed prefix length: the 16-bit data_broadcast_id (EN 300 468 Table 32).
const ID_LEN: usize = 2;

/// Data Broadcast Id Descriptor (tag 0x66).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataBroadcastIdDescriptor<'a> {
    /// 16-bit data_broadcast_id (ETSI TS 101 162 registration).
    pub data_broadcast_id: u16,
    /// Raw id_selector_byte tail — interpretation depends on data_broadcast_id.
    pub id_selector: &'a [u8],
}

impl<'a> Parse<'a> for DataBroadcastIdDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "DataBroadcastIdDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for data_broadcast_id_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "DataBroadcastIdDescriptor body",
            });
        }
        if length < ID_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_broadcast_id_descriptor body shorter than 2 bytes",
            });
        }
        let data_broadcast_id = u16::from_be_bytes([bytes[HEADER_LEN], bytes[HEADER_LEN + 1]]);
        let id_selector = &bytes[HEADER_LEN + ID_LEN..end];
        Ok(Self {
            data_broadcast_id,
            id_selector,
        })
    }
}

impl Serialize for DataBroadcastIdDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ID_LEN + self.id_selector.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let body = ID_LEN + self.id_selector.len();
        if body > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "data_broadcast_id_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body as u8;
        buf[HEADER_LEN..HEADER_LEN + ID_LEN].copy_from_slice(&self.data_broadcast_id.to_be_bytes());
        let sel_start = HEADER_LEN + ID_LEN;
        buf[sel_start..sel_start + self.id_selector.len()].copy_from_slice(self.id_selector);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for DataBroadcastIdDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (ID_LEN + self.id_selector.len()) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DataBroadcastIdDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DATA_BROADCAST_ID";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_id_and_selector() {
        let bytes = [TAG, 0x05, 0x00, 0x0B, 0xAA, 0xBB, 0xCC];
        let d = DataBroadcastIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.data_broadcast_id, 0x000B);
        assert_eq!(d.id_selector, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn parse_accepts_empty_selector() {
        let bytes = [TAG, 0x02, 0x00, 0x0A];
        let d = DataBroadcastIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.data_broadcast_id, 0x000A);
        assert!(d.id_selector.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = DataBroadcastIdDescriptor::parse(&[0x65, 0x02, 0x00, 0x0A]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x65, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = DataBroadcastIdDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        // length=1: not even the 16-bit id fits.
        let err = DataBroadcastIdDescriptor::parse(&[TAG, 0x01, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_length_overrun() {
        // length=5 but only 3 payload bytes available.
        let err = DataBroadcastIdDescriptor::parse(&[TAG, 0x05, 0x00, 0x0B, 0xAA]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x0123,
            id_selector: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = DataBroadcastIdDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x0001,
            id_selector: &[0x01],
        };
        let mut tiny = [0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // 254 selector bytes + 2 id bytes = 256 > 255.
        let sel = vec![0u8; 254];
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x0001,
            id_selector: &sel,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        let err = d.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable() {
        // Borrowed `&[u8]` cannot be deserialized from a JSON array by
        // serde_json; matching the borrowed-bytes descriptors in this crate we
        // exercise the serialize path and assert it is deterministic.
        let d = DataBroadcastIdDescriptor {
            data_broadcast_id: 0x000B,
            id_selector: &[0x01, 0x02],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, serde_json::to_string(&d.clone()).unwrap());
    }
}
