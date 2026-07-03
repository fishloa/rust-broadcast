//! Smoothing Buffer Descriptor — ISO/IEC 13818-1 §2.6.30 (tag 0x10).
//!
//! Describes the leak rate and size of a smoothing buffer.
//! `sb_leak_rate` is in units of 400 bits/s; `sb_size` in units of
//! 1 byte (§2.6.31).

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for smoothing_buffer_descriptor.
pub const TAG: u8 = 0x10;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 6;

/// Smoothing Buffer Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SmoothingBufferDescriptor {
    /// SB leak rate in units of 400 bits/s (22 bits).
    pub sb_leak_rate: u32,
    /// SB size in bytes (22 bits).
    pub sb_size: u32,
}

impl<'a> Parse<'a> for SmoothingBufferDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SmoothingBufferDescriptor",
            "unexpected tag for smoothing_buffer_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "smoothing_buffer_descriptor length must equal 6",
            });
        }
        let sb_leak_rate =
            ((u32::from(body[0]) & 0x3F) << 16) | (u32::from(body[1]) << 8) | u32::from(body[2]);
        let sb_size =
            ((u32::from(body[3]) & 0x3F) << 16) | (u32::from(body[4]) << 8) | u32::from(body[5]);
        Ok(Self {
            sb_leak_rate,
            sb_size,
        })
    }
}

impl Serialize for SmoothingBufferDescriptor {
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
        let slr = self.sb_leak_rate;
        buf[HEADER_LEN] = ((slr >> 16) & 0x3F) as u8;
        buf[HEADER_LEN + 1] = ((slr >> 8) & 0xFF) as u8;
        buf[HEADER_LEN + 2] = (slr & 0xFF) as u8;
        let ss = self.sb_size;
        buf[HEADER_LEN + 3] = ((ss >> 16) & 0x3F) as u8;
        buf[HEADER_LEN + 4] = ((ss >> 8) & 0xFF) as u8;
        buf[HEADER_LEN + 5] = (ss & 0xFF) as u8;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for SmoothingBufferDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SMOOTHING_BUFFER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            6,
            0b00_111111,
            0xAB,
            0xCD, // sb_leak_rate=0x3FABCD
            0b00_000010,
            0x34,
            0x56, // sb_size=0x023456
        ];
        let d = SmoothingBufferDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.sb_leak_rate, 0x3FABCD);
        assert_eq!(d.sb_size, 0x023456);
    }

    #[test]
    fn serialize_round_trip() {
        let d = SmoothingBufferDescriptor {
            sb_leak_rate: 0x1A2B3C,
            sb_size: 0x2D4E5F,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = SmoothingBufferDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = SmoothingBufferDescriptor::parse(&[0x11, 6, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x11, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = SmoothingBufferDescriptor::parse(&[TAG, 5, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = SmoothingBufferDescriptor {
            sb_leak_rate: 0,
            sb_size: 0,
        };
        let mut tiny = vec![0u8; 4];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
