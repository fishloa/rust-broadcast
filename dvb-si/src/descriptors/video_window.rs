//! Video Window Descriptor — ISO/IEC 13818-1 §2.6.14 (tag 0x08).
//!
//! Describes the position and priority of a video window.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for video_window_descriptor.
pub const TAG: u8 = 0x08;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;

/// Video Window Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct VideoWindowDescriptor {
    /// Horizontal offset (14 bits).
    pub horizontal_offset: u16,
    /// Vertical offset (14 bits).
    pub vertical_offset: u16,
    /// Window priority (4 bits).
    pub window_priority: u8,
}

impl<'a> Parse<'a> for VideoWindowDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "VideoWindowDescriptor",
            "unexpected tag for video_window_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "video_window_descriptor length must equal 4",
            });
        }
        let horizontal_offset = (u16::from(body[0]) << 6) | (u16::from(body[1]) >> 2);
        let vertical_offset = ((u16::from(body[1]) & 0x03) << 12)
            | (u16::from(body[2]) << 4)
            | (u16::from(body[3]) >> 4);
        let window_priority = body[3] & 0x0F;
        Ok(Self {
            horizontal_offset,
            vertical_offset,
            window_priority,
        })
    }
}

impl Serialize for VideoWindowDescriptor {
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
        let ho = self.horizontal_offset;
        buf[HEADER_LEN] = (ho >> 6) as u8;
        buf[HEADER_LEN + 1] =
            ((ho & 0x3F) << 2) as u8 | ((self.vertical_offset >> 12) as u8 & 0x03);
        buf[HEADER_LEN + 2] = (self.vertical_offset >> 4) as u8;
        buf[HEADER_LEN + 3] =
            ((self.vertical_offset & 0x0F) << 4) as u8 | (self.window_priority & 0x0F);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for VideoWindowDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "VIDEO_WINDOW";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            4,
            0b0000_0100,
            0b0000_0000, // horizontal_offset=256 (0x0100), low 2 bits = 0
            0b0010_0000,
            0b0000_0101, // vertical_offset=512 (0x0200), priority=5
        ];
        let d = VideoWindowDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.horizontal_offset, 0x0100);
        assert_eq!(d.vertical_offset, 0x0200);
        assert_eq!(d.window_priority, 5);
    }

    #[test]
    fn serialize_round_trip() {
        let d = VideoWindowDescriptor {
            horizontal_offset: 640,
            vertical_offset: 480,
            window_priority: 7,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = VideoWindowDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = VideoWindowDescriptor::parse(&[0x09, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x09, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = VideoWindowDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = VideoWindowDescriptor {
            horizontal_offset: 0,
            vertical_offset: 0,
            window_priority: 0,
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
