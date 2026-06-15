//! Simple Application Location Descriptor — ETSI TS 102 809 §5.3.7.0, Table 33
//! (AIT tag 0x15).
//!
//! Carried in the AIT per-application descriptor loop. The body is the
//! initial path string (e.g. `/index.html`).

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use crate::text::DvbText;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for simple_application_location_descriptor (AIT namespace).
pub const TAG: u8 = 0x15;
const HEADER_LEN: usize = 2;

/// Simple Application Location Descriptor (AIT tag 0x15).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SimpleApplicationLocationDescriptor<'a> {
    /// Initial path bytes (DVB Annex-A encoded; typically ASCII/UTF-8).
    pub initial_path_bytes: DvbText<'a>,
}

impl<'a> Parse<'a> for SimpleApplicationLocationDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SimpleApplicationLocationDescriptor",
            "unexpected tag for simple_application_location_descriptor",
        )?;
        Ok(Self {
            initial_path_bytes: DvbText::new(body),
        })
    }
}

impl Serialize for SimpleApplicationLocationDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.initial_path_bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.initial_path_bytes.len();
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "simple_application_location_descriptor body exceeds 255 bytes",
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
        buf[1] = body_len as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.initial_path_bytes.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SimpleApplicationLocationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SIMPLE_APPLICATION_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path() {
        let path = b"/index.html";
        let mut bytes = vec![TAG, path.len() as u8];
        bytes.extend_from_slice(path);
        let d = SimpleApplicationLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.initial_path_bytes.raw(), b"/index.html");
    }

    #[test]
    fn parse_empty_path() {
        let bytes = [TAG, 0];
        let d = SimpleApplicationLocationDescriptor::parse(&bytes).unwrap();
        assert!(d.initial_path_bytes.raw().is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = SimpleApplicationLocationDescriptor {
            initial_path_bytes: DvbText::new(b"/app/main.html"),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = SimpleApplicationLocationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = [TAG, 5, b'/', b'a', b'p', b'p', b'/'];
        let d = SimpleApplicationLocationDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
