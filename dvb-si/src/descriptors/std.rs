//! STD Descriptor — ISO/IEC 13818-1 §2.6.32 (tag 0x11).
//!
//! Indicates whether the leak method of buffer management is valid.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for STD_descriptor.
pub const TAG: u8 = 0x11;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 1;

/// STD Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct StdDescriptor {
    /// Leak valid flag — 1 means the leak method is applicable.
    pub leak_valid_flag: bool,
}

impl<'a> Parse<'a> for StdDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "StdDescriptor",
            "unexpected tag for STD_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "STD_descriptor length must equal 1",
            });
        }
        let leak_valid_flag = (body[0] & 0x01) != 0;
        Ok(Self { leak_valid_flag })
    }
}

impl Serialize for StdDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
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
        buf[HEADER_LEN] = self.leak_valid_flag as u8;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for StdDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "STD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leak_valid() {
        let bytes = [TAG, 1, 0b0000_0001];
        let d = StdDescriptor::parse(&bytes).unwrap();
        assert!(d.leak_valid_flag);
    }

    #[test]
    fn parse_leak_not_valid() {
        let bytes = [TAG, 1, 0b0000_0000];
        let d = StdDescriptor::parse(&bytes).unwrap();
        assert!(!d.leak_valid_flag);
    }

    #[test]
    fn serialize_round_trip() {
        let d = StdDescriptor {
            leak_valid_flag: true,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = StdDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = StdDescriptor::parse(&[0x12, 1, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x12, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = StdDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = StdDescriptor {
            leak_valid_flag: false,
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
