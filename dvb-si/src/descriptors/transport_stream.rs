//! Transport Stream Descriptor — ETSI EN 300 468 §6.2.46 (tag 0x67, Table 105, PDF p. 109).
//!
//! Carried inside the NIT transport_stream_loop. The body is a raw byte
//! sequence "for indication of DVB compliance" — a single
//! `for (i=0;i<N;i++) { byte }` loop with no internal structure (Table 105).
//! The de-facto convention is the three ASCII bytes `b"DVB"`, but the spec
//! defines no semantics so we preserve the raw bytes verbatim.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for transport_stream_descriptor.
pub const TAG: u8 = 0x67;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;

/// Transport Stream Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TransportStreamDescriptor<'a> {
    /// Raw `byte` loop — DVB-compliance marker (conventionally `b"DVB"`).
    pub bytes: &'a [u8],
}

impl<'a> Parse<'a> for TransportStreamDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "TransportStreamDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for transport_stream_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "TransportStreamDescriptor body",
            });
        }
        Ok(Self {
            bytes: &bytes[HEADER_LEN..end],
        })
    }
}

impl Serialize for TransportStreamDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.bytes.len() > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "transport_stream_descriptor body exceeds 255 bytes",
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
        buf[1] = self.bytes.len() as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.bytes);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for TransportStreamDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        self.bytes.len() as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TransportStreamDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TRANSPORT_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_dvb_marker() {
        let bytes = [TAG, 3, b'D', b'V', b'B'];
        let d = TransportStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.bytes, b"DVB");
    }

    #[test]
    fn empty_body_is_valid() {
        let bytes = [TAG, 0];
        let d = TransportStreamDescriptor::parse(&bytes).unwrap();
        assert!(d.bytes.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TransportStreamDescriptor::parse(&[0x68, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x68, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            TransportStreamDescriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 5, 1, 2, 3];
        assert!(matches!(
            TransportStreamDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = TransportStreamDescriptor { bytes: b"DVB" };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TransportStreamDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = TransportStreamDescriptor { bytes: b"DVB" };
        let mut buf = vec![0u8; 1];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        let big = vec![0u8; 256];
        let d = TransportStreamDescriptor { bytes: &big };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    // No JSON serde round-trip test: the borrowed `&[u8]` field cannot
    // round-trip through serde_json (it serializes a slice as a sequence,
    // not a borrowed byte array). This matches sibling borrowed-bytes
    // descriptors (linkage.rs, default_authority.rs, content_identifier.rs),
    // none of which carry a serde_round_trip test. The serde derive itself
    // is still exercised by compilation under `--all-features`.
}
