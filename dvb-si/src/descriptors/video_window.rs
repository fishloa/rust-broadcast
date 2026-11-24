//! Video Window Descriptor — ISO/IEC 13818-1 §2.6.14 (tag 0x08).
//!
//! Describes the position and priority of a video window.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for video_window_descriptor.
pub const TAG: u8 = 0x08;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;
/// Wire width of the two 14-bit fields.
const SIZE_BITS: u32 = 14;
/// Mask of the two 14-bit fields.
const SIZE_MASK: u16 = (1 << SIZE_BITS) - 1;
/// Mask of the 2-bit high part of the second field.
const FIELD2_HI_MASK: u16 = 0x03;
/// Mask of the 4-bit low nibble field.
const NIBBLE_MASK: u8 = 0x0F;
/// Shift of the 6-bit low part of the first field in its wire byte.
const FIELD_HI_SHIFT: u16 = 6;
/// Shift of the 2-bit high part of the second field.
const FIELD2_HI_SHIFT: u16 = 12;
/// Shift of the middle part of the second field.
const FIELD2_MID_SHIFT: u16 = 4;
/// Shift of the low nibble in its wire byte.
const NIBBLE_SHIFT: u16 = 4;
/// Shift of the first field's low bits in the serializer.
const FIELD_LO_SHIFT: u16 = 2;
/// Body offset of the middle byte of the second field.
const MIDDLE_BYTE_OFFSET: usize = 2;
/// Body offset of the low-nibble byte.
const LOW_NIBBLE_OFFSET: usize = 3;

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
        let horizontal_offset = (u16::from(body[0]) << FIELD_HI_SHIFT) | (u16::from(body[1]) >> FIELD_LO_SHIFT);
        let vertical_offset = ((u16::from(body[1]) & FIELD2_HI_MASK) << FIELD2_HI_SHIFT)
            | (u16::from(body[MIDDLE_BYTE_OFFSET]) << FIELD2_MID_SHIFT)
            | (u16::from(body[LOW_NIBBLE_OFFSET]) >> NIBBLE_SHIFT);
        let window_priority = body[LOW_NIBBLE_OFFSET] & NIBBLE_MASK;
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
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        // 14 / 14 / 4-bit fields; wider values used to be masked down
        // silently (audit r03-W3 class).
        let ho = broadcast_common::len::fit_bits(
            u64::from(self.horizontal_offset),
            SIZE_BITS,
            "horizontal_offset",
        )? as u16;
        broadcast_common::len::fit_bits(u64::from(self.vertical_offset), SIZE_BITS, "vertical_offset")?;
        broadcast_common::len::fit_bits(u64::from(self.window_priority), u32::from(NIBBLE_MASK.count_ones()), "window_priority")?;
        buf[HEADER_LEN] = (ho >> FIELD_HI_SHIFT) as u8;
        buf[HEADER_LEN + 1] =
            ((ho & SIZE_MASK) << FIELD_LO_SHIFT) as u8
            | ((self.vertical_offset >> FIELD2_HI_SHIFT) as u8 & FIELD2_HI_MASK as u8);
        buf[HEADER_LEN + MIDDLE_BYTE_OFFSET] = (self.vertical_offset >> FIELD2_MID_SHIFT) as u8;
        buf[HEADER_LEN + LOW_NIBBLE_OFFSET] = ((self.vertical_offset & u16::from(NIBBLE_MASK)) << NIBBLE_SHIFT) as u8
            | broadcast_common::len::fit_mask(self.window_priority, NIBBLE_MASK, "window_priority")?;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for VideoWindowDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "VIDEO_WINDOW";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

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
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }
}
