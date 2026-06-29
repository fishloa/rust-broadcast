//! DVB-J Application Descriptor — ETSI TS 102 727 §10.9.1, Table 84
//! (AIT tag 0x03).
//!
//! Carried in the AIT per-application descriptor loop. The body is a run of
//! length-prefixed parameter strings consumed until `descriptor_length` is exhausted.

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for dvb_j_application_descriptor (AIT namespace).
pub const TAG: u8 = 0x03;
const HEADER_LEN: usize = 2;

/// DVB-J Application Descriptor (AIT tag 0x03).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbJApplicationDescriptor<'a> {
    /// Length-prefixed parameter strings in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub parameters: Vec<&'a [u8]>,
}

impl<'a> Parse<'a> for DvbJApplicationDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DvbJApplicationDescriptor",
            "unexpected tag for dvb_j_application_descriptor",
        )?;
        let mut parameters = Vec::new();
        let mut pos = 0;
        while pos < body.len() {
            let param_len = body[pos] as usize;
            pos += 1;
            if pos + param_len > body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "dvb_j_application_descriptor parameter overruns body",
                });
            }
            parameters.push(&body[pos..pos + param_len]);
            pos += param_len;
        }
        Ok(Self { parameters })
    }
}

impl Serialize for DvbJApplicationDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.parameters.iter().map(|p| 1 + p.len()).sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // Check each parameter fits in one length byte.
        for p in &self.parameters {
            if p.len() > u8::MAX as usize {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "dvb_j_application_descriptor parameter exceeds 255 bytes",
                });
            }
        }
        let body_len = self.serialized_len() - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_j_application_descriptor body exceeds 255 bytes",
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
        let mut pos = HEADER_LEN;
        for p in &self.parameters {
            buf[pos] = p.len() as u8;
            pos += 1;
            buf[pos..pos + p.len()].copy_from_slice(p);
            pos += p.len();
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DvbJApplicationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DVB_J_APPLICATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        let bytes = [TAG, 0];
        let d = DvbJApplicationDescriptor::parse(&bytes).unwrap();
        assert!(d.parameters.is_empty());
    }

    #[test]
    fn parse_two_params() {
        let bytes = [
            TAG, 6, // descriptor_length = 6
            2, b'a', b'b', // param_len=2, "ab"
            2, b'c', b'd', // param_len=2, "cd"
        ];
        let d = DvbJApplicationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.parameters.len(), 2);
        assert_eq!(d.parameters[0], b"ab");
        assert_eq!(d.parameters[1], b"cd");
    }

    #[test]
    fn parse_truncated_param_value() {
        let bytes = [
            TAG, 3, // descriptor_length = 3
            4, b'a',
            b'b', // param_len=4, but only 2 bytes remain after length
                  // (body is only 3 bytes: [4, a, b])
        ];
        assert!(DvbJApplicationDescriptor::parse(&bytes).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let d = DvbJApplicationDescriptor {
            parameters: vec![&b"hello"[..], &b"world"[..]],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = DvbJApplicationDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_byte_identical() {
        let bytes = [
            TAG, 4, // descriptor_length = 4
            1, b'a', // param1
            1, b'b', // param2
        ];
        let d = DvbJApplicationDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
