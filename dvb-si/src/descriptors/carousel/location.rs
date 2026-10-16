//! Location Descriptor — ETSI EN 301 192 V1.7.1 §10.2.7, Table 55
//! (data-carousel tag 0x06). PDF p.67, transcribed in
//! `docs/descriptors/en_301_192/55-*.md`, verified against the rendered page.
//!
//! Locates the PID carrying blocks, modules or groups of the carousel by the
//! `component_tag` of its `stream_identifier_descriptor`.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for location_descriptor (data-carousel namespace).
pub const TAG: u8 = 0x06;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 1;

/// Location Descriptor (carousel tag 0x06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocationDescriptor {
    /// `location_tag` — the `component_tag` of the stream_identifier_descriptor
    /// of the stream that carries the data.
    pub location_tag: u8,
}

impl<'a> Parse<'a> for LocationDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "LocationDescriptor",
            "unexpected tag for location_descriptor",
        )?;
        match body {
            [location_tag] => Ok(Self {
                location_tag: *location_tag,
            }),
            _ => Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "location_descriptor body must be 1 byte",
            }),
        }
    }
}

impl Serialize for LocationDescriptor {
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
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[2] = self.location_tag;
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for LocationDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip() {
        let bytes = [TAG, 1, 0x42];
        let d = LocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.location_tag, 0x42);
        let mut out = [0u8; 3];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn wrong_length_and_short_buffer_rejected() {
        assert!(LocationDescriptor::parse(&[TAG, 0]).is_err());
        assert!(LocationDescriptor::parse(&[TAG, 2, 1, 2]).is_err());
        let d = LocationDescriptor { location_tag: 1 };
        assert!(d.serialize_into(&mut [0u8; 2]).is_err());
    }
}
