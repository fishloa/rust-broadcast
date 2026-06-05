//! NVOD Reference Descriptor — ETSI EN 300 468 §6.2.28 (tag 0x4B).
//!
//! Table 80 (PDF p. 96). Carried in the SDT for an NVOD reference service.
//! Body is a loop of (transport_stream_id, original_network_id, service_id)
//! triples, each identifying one of the time-shifted NVOD services.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for NVOD_reference_descriptor.
pub const TAG: u8 = 0x4B;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 6;
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;

/// One NVOD reference triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NvodReferenceEntry {
    /// transport_stream_id carrying the referenced service.
    pub transport_stream_id: u16,
    /// original_network_id of the referenced service.
    pub original_network_id: u16,
    /// service_id of the referenced NVOD time-shifted service.
    pub service_id: u16,
}

/// NVOD Reference Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NvodReferenceDescriptor {
    /// Reference triples in wire order.
    pub entries: Vec<NvodReferenceEntry>,
}

impl<'a> Parse<'a> for NvodReferenceDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "NvodReferenceDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for NVOD_reference_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 6",
            });
        }
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "NvodReferenceDescriptor body",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let mut entries = Vec::with_capacity(length / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            entries.push(NvodReferenceEntry {
                transport_stream_id: u16::from_be_bytes([chunk[0], chunk[1]]),
                original_network_id: u16::from_be_bytes([chunk[2], chunk[3]]),
                service_id: u16::from_be_bytes([chunk[4], chunk[5]]),
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for NvodReferenceDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ENTRY_LEN * self.entries.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = ENTRY_LEN * self.entries.len();
        // 8-bit descriptor_length field: error rather than silently truncate.
        if body_len > MAX_BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: MAX_BODY_LEN,
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 2].copy_from_slice(&e.transport_stream_id.to_be_bytes());
            buf[pos + 2..pos + 4].copy_from_slice(&e.original_network_id.to_be_bytes());
            buf[pos + 4..pos + 6].copy_from_slice(&e.service_id.to_be_bytes());
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for NvodReferenceDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (ENTRY_LEN * self.entries.len()) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for NvodReferenceDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "NVOD_REFERENCE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_triple() {
        let bytes = [TAG, 6, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03];
        let d = NvodReferenceDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].transport_stream_id, 1);
        assert_eq!(d.entries[0].original_network_id, 2);
        assert_eq!(d.entries[0].service_id, 3);
    }

    #[test]
    fn parse_multiple_triples_preserves_order() {
        let bytes = [
            TAG, 12, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00, 0x05, 0x00, 0x06,
        ];
        let d = NvodReferenceDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].transport_stream_id, 4);
        assert_eq!(d.entries[1].original_network_id, 5);
        assert_eq!(d.entries[1].service_id, 6);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            NvodReferenceDescriptor::parse(&[0x4C, 6, 0, 0, 0, 0, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x4C, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let bytes = [TAG, 12, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03];
        assert!(matches!(
            NvodReferenceDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_6() {
        let bytes = [TAG, 5, 0x00, 0x01, 0x00, 0x02, 0x00];
        assert!(matches!(
            NvodReferenceDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let d = NvodReferenceDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = NvodReferenceDescriptor {
            entries: vec![
                NvodReferenceEntry {
                    transport_stream_id: 0x1234,
                    original_network_id: 0x5678,
                    service_id: 0x9ABC,
                },
                NvodReferenceEntry {
                    transport_stream_id: 0x0001,
                    original_network_id: 0x0002,
                    service_id: 0x0003,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(NvodReferenceDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        // 43 triples = 258 body bytes, past the u8 length field.
        let d = NvodReferenceDescriptor {
            entries: vec![
                NvodReferenceEntry {
                    transport_stream_id: 1,
                    original_network_id: 2,
                    service_id: 3,
                };
                43
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = NvodReferenceDescriptor {
            entries: vec![NvodReferenceEntry {
                transport_stream_id: 1,
                original_network_id: 2,
                service_id: 3,
            }],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
