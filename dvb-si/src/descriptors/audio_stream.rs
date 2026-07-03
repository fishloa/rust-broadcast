//! Audio Stream Descriptor — ISO/IEC 13818-1 §2.6.4 (tag 0x03).
//!
//! Describes the elementary stream as an MPEG audio stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for audio_stream_descriptor.
pub const TAG: u8 = 0x03;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 1;

/// Audio Stream Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AudioStreamDescriptor {
    /// Free format flag.
    pub free_format_flag: bool,
    /// MPEG audio ID.
    pub id: bool,
    /// Layer (0=reserved).
    pub layer: u8,
    /// Variable rate audio indicator.
    pub variable_rate_audio_indicator: bool,
}

impl<'a> Parse<'a> for AudioStreamDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AudioStreamDescriptor",
            "unexpected tag for audio_stream_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "audio_stream_descriptor length must equal 1",
            });
        }
        let b = body[0];
        Ok(Self {
            free_format_flag: (b & 0x80) != 0,
            id: (b & 0x40) != 0,
            layer: (b >> 4) & 0x03,
            variable_rate_audio_indicator: (b & 0x08) != 0,
        })
    }
}

impl Serialize for AudioStreamDescriptor {
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
        buf[HEADER_LEN] = ((self.free_format_flag as u8) << 7)
            | ((self.id as u8) << 6)
            | ((self.layer & 0x03) << 4)
            | ((self.variable_rate_audio_indicator as u8) << 3);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for AudioStreamDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AUDIO_STREAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            1,
            0b1110_1000, // free=1, id=1, layer=2, variable=1, reserved=0
        ];
        let d = AudioStreamDescriptor::parse(&bytes).unwrap();
        assert!(d.free_format_flag);
        assert!(d.id);
        assert_eq!(d.layer, 2);
        assert!(d.variable_rate_audio_indicator);
    }

    #[test]
    fn serialize_round_trip() {
        let d = AudioStreamDescriptor {
            free_format_flag: false,
            id: true,
            layer: 3,
            variable_rate_audio_indicator: false,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = AudioStreamDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = AudioStreamDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = AudioStreamDescriptor::parse(&[0x04, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x04, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = AudioStreamDescriptor {
            free_format_flag: false,
            id: false,
            layer: 0,
            variable_rate_audio_indicator: false,
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
