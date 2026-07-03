//! MPEG-4 Audio Descriptor — ISO/IEC 13818-1 §2.6.38 (tag 0x1C).
//!
//! Provides the MPEG-4 audio profile and level of the associated
//! elementary stream.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for MPEG-4_audio_descriptor.
pub const TAG: u8 = 0x1C;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 1;

/// MPEG-4 Audio Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Mpeg4AudioDescriptor {
    /// MPEG-4 audio profile and level indication.
    pub mpeg_4_audio_profile_and_level: u8,
}

impl<'a> Parse<'a> for Mpeg4AudioDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Mpeg4AudioDescriptor",
            "unexpected tag for MPEG-4_audio_descriptor",
        )?;
        if body.len() < (BODY_LEN as usize) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "MPEG-4_audio_descriptor too short",
            });
        }
        Ok(Self {
            mpeg_4_audio_profile_and_level: body[0],
        })
    }
}

impl Serialize for Mpeg4AudioDescriptor {
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
        buf[HEADER_LEN] = self.mpeg_4_audio_profile_and_level;
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for Mpeg4AudioDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MPEG4_AUDIO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let orig = Mpeg4AudioDescriptor {
            mpeg_4_audio_profile_and_level: 0xCD,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = Mpeg4AudioDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = Mpeg4AudioDescriptor::parse(&[0x02, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = Mpeg4AudioDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }
}
