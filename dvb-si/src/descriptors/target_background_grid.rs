//! Target Background Grid Descriptor — ISO/IEC 13818-1 §2.6.12 (tag 0x07).
//!
//! Describes the target background grid size and aspect ratio.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for target_background_grid_descriptor.
pub const TAG: u8 = 0x07;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;

/// Target Background Grid Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetBackgroundGridDescriptor {
    /// Horizontal size (14 bits).
    pub horizontal_size: u16,
    /// Vertical size (14 bits).
    pub vertical_size: u16,
    /// Aspect ratio information (4 bits).
    pub aspect_ratio_information: u8,
}

impl<'a> Parse<'a> for TargetBackgroundGridDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TargetBackgroundGridDescriptor",
            "unexpected tag for target_background_grid_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "target_background_grid_descriptor length must equal 4",
            });
        }
        let horizontal_size = (u16::from(body[0]) << 6) | (u16::from(body[1]) >> 2);
        let vertical_size = ((u16::from(body[1]) & 0x03) << 12)
            | (u16::from(body[2]) << 4)
            | (u16::from(body[3]) >> 4);
        let aspect_ratio_information = body[3] & 0x0F;
        Ok(Self {
            horizontal_size,
            vertical_size,
            aspect_ratio_information,
        })
    }
}

impl Serialize for TargetBackgroundGridDescriptor {
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
        let hs = self.horizontal_size;
        buf[HEADER_LEN] = (hs >> 6) as u8;
        buf[HEADER_LEN + 1] = ((hs & 0x3F) << 2) as u8 | ((self.vertical_size >> 12) as u8 & 0x03);
        buf[HEADER_LEN + 2] = (self.vertical_size >> 4) as u8;
        buf[HEADER_LEN + 3] =
            ((self.vertical_size & 0x0F) << 4) as u8 | (self.aspect_ratio_information & 0x0F);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for TargetBackgroundGridDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TARGET_BACKGROUND_GRID";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            4,
            0b0001_0101,
            0b0101_0000, // horizontal_size = 1364 (0x0554), low 2 bits = 0
            0b1010_1010,
            0b1010_0011, // vertical_size 14 bits = 0x0AAA (2730), aspect_ratio = 3
        ];
        let d = TargetBackgroundGridDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.horizontal_size, 0x0554);
        // vertical_size: low 2 bits of byte1 (=0) | byte2 | high 4 bits of byte3
        // 0b00_10101010_1010 = 0x0AAA = 2730
        assert_eq!(d.vertical_size, 0x0AAA);
        assert_eq!(d.aspect_ratio_information, 3);
    }

    #[test]
    fn serialize_round_trip() {
        let d = TargetBackgroundGridDescriptor {
            horizontal_size: 1920,
            vertical_size: 1080,
            aspect_ratio_information: 0x0F,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = TargetBackgroundGridDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn horizontal_size_max() {
        let d = TargetBackgroundGridDescriptor {
            horizontal_size: 0x3FFF,
            vertical_size: 0,
            aspect_ratio_information: 0,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = TargetBackgroundGridDescriptor::parse(&buf).unwrap();
        assert_eq!(reparsed.horizontal_size, 0x3FFF);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = TargetBackgroundGridDescriptor::parse(&[0x08, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x08, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = TargetBackgroundGridDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = TargetBackgroundGridDescriptor {
            horizontal_size: 0,
            vertical_size: 0,
            aspect_ratio_information: 0,
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
