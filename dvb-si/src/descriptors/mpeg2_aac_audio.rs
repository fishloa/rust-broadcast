//! MPEG-2 AAC Audio Descriptor — ISO/IEC 13818-1 §2.6.68 (tag 0x2B).
//!
//! Identifies the profile, channel configuration, and additional
//! information for an MPEG-2 AAC audio elementary stream.

use super::aac_additional_info::AacAdditionalInfo;
use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for MPEG-2_AAC_audio_descriptor.
pub const TAG: u8 = 0x2B;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 3;

/// MPEG-2 AAC Audio Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Mpeg2AacAudioDescriptor {
    /// MPEG-2 AAC profile.
    pub mpeg_2_aac_profile: u8,
    /// MPEG-2 AAC channel configuration.
    pub mpeg_2_aac_channel_configuration: u8,
    /// MPEG-2 AAC additional information (Table 2-95).
    pub mpeg_2_aac_additional_information: AacAdditionalInfo,
}

impl<'a> Parse<'a> for Mpeg2AacAudioDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Mpeg2AacAudioDescriptor",
            "unexpected tag for MPEG-2_AAC_audio_descriptor",
        )?;
        if body.len() < (BODY_LEN as usize) {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "MPEG-2_AAC_audio_descriptor too short",
            });
        }
        Ok(Self {
            mpeg_2_aac_profile: body[0],
            mpeg_2_aac_channel_configuration: body[1],
            mpeg_2_aac_additional_information: AacAdditionalInfo::from_u8(body[2]),
        })
    }
}

impl Serialize for Mpeg2AacAudioDescriptor {
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
        buf[HEADER_LEN] = self.mpeg_2_aac_profile;
        buf[HEADER_LEN + 1] = self.mpeg_2_aac_channel_configuration;
        buf[HEADER_LEN + 2] = self.mpeg_2_aac_additional_information.to_u8();
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for Mpeg2AacAudioDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MPEG2_AAC_AUDIO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let orig = Mpeg2AacAudioDescriptor {
            mpeg_2_aac_profile: 0x01,
            mpeg_2_aac_channel_configuration: 0x02,
            mpeg_2_aac_additional_information: AacAdditionalInfo::AacWithBandwidthExtension,
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = Mpeg2AacAudioDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn round_trip_reserved() {
        let orig = Mpeg2AacAudioDescriptor {
            mpeg_2_aac_profile: 0xAA,
            mpeg_2_aac_channel_configuration: 0xBB,
            mpeg_2_aac_additional_information: AacAdditionalInfo::Reserved(0xFE),
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let reparsed = Mpeg2AacAudioDescriptor::parse(&buf).unwrap();
        assert_eq!(orig, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = Mpeg2AacAudioDescriptor::parse(&[0x02, 3, 0x00, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        // present-but-short body (1 byte < BODY_LEN=3) → descriptor's own check.
        let err = Mpeg2AacAudioDescriptor::parse(&[TAG, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }
}
