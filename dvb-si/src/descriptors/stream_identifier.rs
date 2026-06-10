//! Stream Identifier Descriptor — ETSI EN 300 468 §6.2.41 (tag 0x52).
//!
//! One-byte `component_tag` that anchors components named elsewhere (e.g. by
//! component_descriptor).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for stream_identifier_descriptor.
pub const TAG: u8 = 0x52;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 1;

/// Stream Identifier Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StreamIdentifierDescriptor {
    /// Component tag used to cross-reference a component_descriptor.
    pub component_tag: u8,
}

impl<'a> Parse<'a> for StreamIdentifierDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "StreamIdentifierDescriptor",
            "unexpected tag for stream_identifier_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "stream_identifier_descriptor length must equal 1",
            });
        }
        Ok(Self {
            component_tag: body[0],
        })
    }
}

impl Serialize for StreamIdentifierDescriptor {
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
        buf[2] = self.component_tag;
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for StreamIdentifierDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for StreamIdentifierDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "STREAM_IDENTIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_component_tag() {
        let d = StreamIdentifierDescriptor::parse(&[TAG, 1, 0x42]).unwrap();
        assert_eq!(d.component_tag, 0x42);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = StreamIdentifierDescriptor::parse(&[0x53, 1, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x53, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = StreamIdentifierDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = StreamIdentifierDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = StreamIdentifierDescriptor {
            component_tag: 0xFE,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(StreamIdentifierDescriptor::parse(&buf).unwrap(), d);
    }
}
