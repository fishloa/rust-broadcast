//! Simple Application Boundary Descriptor — ETSI TS 102 809 §5.3.8, Table 35
//! (AIT tag 0x17).
//!
//! Carried in the AIT per-application descriptor loop. A list of boundary
//! extensions, each a length-prefixed byte string.

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for simple_application_boundary_descriptor (AIT namespace).
pub const TAG: u8 = 0x17;
const HEADER_LEN: usize = 2;
const COUNT_LEN: usize = 1;

/// Simple Application Boundary Descriptor (AIT tag 0x17).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SimpleApplicationBoundaryDescriptor<'a> {
    /// Boundary extension byte strings in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub boundary_extensions: Vec<&'a [u8]>,
}

impl<'a> Parse<'a> for SimpleApplicationBoundaryDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SimpleApplicationBoundaryDescriptor",
            "unexpected tag for simple_application_boundary_descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "simple_application_boundary_descriptor body is empty",
            });
        }
        let boundary_extension_count = body[0] as usize;
        let mut extensions = Vec::with_capacity(boundary_extension_count);
        let mut pos = COUNT_LEN;
        for _ in 0..boundary_extension_count {
            if pos >= body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "boundary_extension_length missing",
                });
            }
            let ext_len = body[pos] as usize;
            pos += 1;
            let ext_end = pos + ext_len;
            if ext_end > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "boundary_extension bytes run past descriptor end",
                });
            }
            extensions.push(&body[pos..ext_end]);
            pos = ext_end;
        }
        Ok(Self {
            boundary_extensions: extensions,
        })
    }
}

impl Serialize for SimpleApplicationBoundaryDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + COUNT_LEN
            + self
                .boundary_extensions
                .iter()
                .map(|e| 1 + e.len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.boundary_extensions {
            if e.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "boundary_extension exceeds 255 bytes",
                });
            }
        }
        let len = self.serialized_len();
        let body_len = len - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "simple_application_boundary_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        buf[2] = self.boundary_extensions.len() as u8;
        let mut pos = HEADER_LEN + COUNT_LEN;
        for e in &self.boundary_extensions {
            buf[pos] = e.len() as u8;
            buf[pos + 1..pos + 1 + e.len()].copy_from_slice(e);
            pos += 1 + e.len();
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for SimpleApplicationBoundaryDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SIMPLE_APPLICATION_BOUNDARY";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Body: count(1) + len(1) + "foo"(3) + len(1) + "ba"(2) = 8.
    fn build_two_extensions() -> [u8; 10] {
        [
            TAG, 8, 2, // count
            3, b'f', b'o', b'o', 2, b'b', b'a',
        ]
    }

    #[test]
    fn parse_two_extensions() {
        let bytes = build_two_extensions();
        let d = SimpleApplicationBoundaryDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.boundary_extensions.len(), 2);
        assert_eq!(d.boundary_extensions[0], b"foo");
        assert_eq!(d.boundary_extensions[1], b"ba");
    }

    #[test]
    fn parse_no_extensions() {
        let bytes = [TAG, 1, 0];
        let d = SimpleApplicationBoundaryDescriptor::parse(&bytes).unwrap();
        assert!(d.boundary_extensions.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = SimpleApplicationBoundaryDescriptor {
            boundary_extensions: alloc::vec![b"abc" as &[u8], b"de" as &[u8]],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = SimpleApplicationBoundaryDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = build_two_extensions();
        let d = SimpleApplicationBoundaryDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
