//! AC-3 Descriptor — ETSI EN 300 468 Annex D (tag 0x6A).
//!
//! Carried inside PMT's ES_info loop for AC-3 audio components. The layout
//! is a flag byte followed by four optional 1-byte fields and an optional
//! free-form additional_info trailer.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for AC-3 audio.
pub const TAG: u8 = 0x6A;
const HEADER_LEN: usize = 2;

const FLAG_COMPONENT_TYPE: u8 = 0x80;
const FLAG_BSID: u8 = 0x40;
const FLAG_MAINID: u8 = 0x20;
const FLAG_ASVC: u8 = 0x10;

/// Decoded AC-3 component_type — ETSI EN 300 468 Annex D.
///
/// The component_type byte packs bit-fields describing the audio service type,
/// number of channels, and whether the stream is AC-3 or Enhanced AC-3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ac3ComponentType {
    /// `false` = AC-3, `true` = Enhanced AC-3.
    pub enhanced_ac3: bool,
    /// `true` if this is a full service (suitable for solo presentation).
    pub full_service: bool,
    /// Decoded service type.
    pub service_type: Ac3ServiceType,
    /// Number of audio channels.
    pub channels: Ac3ChannelMode,
}

/// AC-3 / Enhanced AC-3 service type — EN 300 468 Annex D Table D.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ac3ServiceType {
    /// Complete Main (CM).
    CompleteMain,
    /// Music and Effects (ME).
    MusicAndEffects,
    /// Visually Impaired (VI).
    VisuallyImpaired,
    /// Hearing Impaired (HI).
    HearingImpaired,
    /// Dialogue (D).
    Dialogue,
    /// Commentary (C).
    Commentary,
    /// Emergency (E).
    Emergency,
    /// Voice Over (VO) or Karaoke (depending on full_service and channels).
    VoiceOverOrKaraoke,
    /// Unknown/reserved service type value.
    Unknown(u8),
}

impl Ac3ServiceType {
    /// Returns a human-readable name.
    #[must_use]
    /// Returns a human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::CompleteMain => "Complete Main (CM)",
            Self::MusicAndEffects => "Music and Effects (ME)",
            Self::VisuallyImpaired => "Visually Impaired (VI)",
            Self::HearingImpaired => "Hearing Impaired (HI)",
            Self::Dialogue => "Dialogue (D)",
            Self::Commentary => "Commentary (C)",
            Self::Emergency => "Emergency (E)",
            Self::VoiceOverOrKaraoke => "Voice Over (VO) / Karaoke",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// AC-3 / Enhanced AC-3 channel mode — EN 300 468 Annex D Table D.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ac3ChannelMode {
    /// Mono.
    Mono,
    /// 1+1 Mode (dual mono).
    OnePlusOne,
    /// 2 channel (stereo).
    Stereo,
    /// 2 channel Surround encoded (stereo).
    SurroundEncodedStereo,
    /// Multichannel audio (> 2 channels).
    Multichannel,
    /// Multichannel audio (> 5.1 channels).
    MultichannelAbove51,
    /// Multiple programmes in independent substreams.
    MultipleProgrammes,
    /// Unknown/reserved channel mode.
    Unknown(u8),
}

impl Ac3ChannelMode {
    /// Returns a human-readable name.
    #[must_use]
    /// Returns a human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::OnePlusOne => "1+1 Mode",
            Self::Stereo => "2 channel (stereo)",
            Self::SurroundEncodedStereo => "2 channel Surround encoded (stereo)",
            Self::Multichannel => "Multichannel audio (> 2 channels)",
            Self::MultichannelAbove51 => "Multichannel audio (> 5.1 channels)",
            Self::MultipleProgrammes => "Multiple programmes in independent substreams",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// AC-3 Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Ac3Descriptor<'a> {
    /// AC-3 component_type (layout per Annex D).
    pub component_type: Option<u8>,
    /// Bit stream identification.
    pub bsid: Option<u8>,
    /// Main audio service id.
    pub mainid: Option<u8>,
    /// Associated service id.
    pub asvc: Option<u8>,
    /// Raw trailing additional_info bytes.
    pub additional_info: &'a [u8],
}

impl Ac3Descriptor<'_> {
    /// Decodes the optional `component_type` field per ETSI EN 300 468 Annex D.
    ///
    /// Returns `None` when `component_type` is `None`.
    #[must_use]
    pub fn decoded_component_type(&self) -> Option<Ac3ComponentType> {
        let ct = self.component_type?;
        let enhanced_ac3 = (ct & 0x80) != 0;
        let full_service = (ct & 0x20) != 0;
        let service_bits = (ct >> 3) & 0x03;
        let channel_bits = (ct >> 1) & 0x03;

        let service_type = match service_bits {
            0b00 if !full_service => Ac3ServiceType::MusicAndEffects,
            0b00 => Ac3ServiceType::CompleteMain,
            0b01 => Ac3ServiceType::VisuallyImpaired,
            0b10 => Ac3ServiceType::HearingImpaired,
            0b11 => Ac3ServiceType::VoiceOverOrKaraoke,
            _ => Ac3ServiceType::Unknown(service_bits),
        };

        let channels = match channel_bits {
            0b00 => Ac3ChannelMode::Mono,
            0b01 => Ac3ChannelMode::OnePlusOne,
            0b10 => Ac3ChannelMode::Stereo,
            0b11 => Ac3ChannelMode::SurroundEncodedStereo,
            _ => Ac3ChannelMode::Unknown(channel_bits),
        };

        Some(Ac3ComponentType {
            enhanced_ac3,
            full_service,
            service_type,
            channels,
        })
    }
}

impl<'a> Parse<'a> for Ac3Descriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Ac3Descriptor",
            "unexpected tag for AC-3 descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor body is empty (length=0)",
            });
        }
        let flags = body[0];
        let mut pos = 1;
        let mut read_one = |set: bool| -> Result<Option<u8>> {
            if !set {
                return Ok(None);
            }
            if pos >= body.len() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AC-3 descriptor flags claim more bytes than length permits",
                });
            }
            let b = body[pos];
            pos += 1;
            Ok(Some(b))
        };

        let component_type = read_one(flags & FLAG_COMPONENT_TYPE != 0)?;
        let bsid = read_one(flags & FLAG_BSID != 0)?;
        let mainid = read_one(flags & FLAG_MAINID != 0)?;
        let asvc = read_one(flags & FLAG_ASVC != 0)?;
        let additional_info = &body[pos..];
        Ok(Self {
            component_type,
            bsid,
            mainid,
            asvc,
            additional_info,
        })
    }
}

impl Serialize for Ac3Descriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 1
            + usize::from(self.component_type.is_some())
            + usize::from(self.bsid.is_some())
            + usize::from(self.mainid.is_some())
            + usize::from(self.asvc.is_some())
            + self.additional_info.len()
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
        buf[1] = (len - HEADER_LEN) as u8;
        let mut flags: u8 = 0;
        if self.component_type.is_some() {
            flags |= FLAG_COMPONENT_TYPE;
        }
        if self.bsid.is_some() {
            flags |= FLAG_BSID;
        }
        if self.mainid.is_some() {
            flags |= FLAG_MAINID;
        }
        if self.asvc.is_some() {
            flags |= FLAG_ASVC;
        }
        // The low 4 bits are reserved_future_use and must be set to 1.
        buf[2] = flags | 0x0F;
        let mut pos = 3;
        for b in [self.component_type, self.bsid, self.mainid, self.asvc]
            .into_iter()
            .flatten()
        {
            buf[pos] = b;
            pos += 1;
        }
        buf[pos..pos + self.additional_info.len()].copy_from_slice(self.additional_info);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for Ac3Descriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AC3";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_with_all_fields() {
        let bytes = [
            TAG,
            5,
            FLAG_COMPONENT_TYPE | FLAG_BSID | FLAG_MAINID | FLAG_ASVC,
            0x11,
            0x22,
            0x33,
            0x44,
        ];
        let d = Ac3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, Some(0x11));
        assert_eq!(d.bsid, Some(0x22));
        assert_eq!(d.mainid, Some(0x33));
        assert_eq!(d.asvc, Some(0x44));
        assert_eq!(d.additional_info, &[] as &[u8]);
    }

    #[test]
    fn parse_with_only_component_type() {
        let bytes = [TAG, 2, FLAG_COMPONENT_TYPE, 0x07];
        let d = Ac3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, Some(0x07));
        assert_eq!(d.bsid, None);
    }

    #[test]
    fn parse_with_additional_info_only() {
        let bytes = [TAG, 3, 0x00, 0xAA, 0xBB];
        let d = Ac3Descriptor::parse(&bytes).unwrap();
        assert_eq!(d.component_type, None);
        assert_eq!(d.additional_info, &[0xAA, 0xBB]);
    }

    #[test]
    fn decode_component_type_cm_stereo() {
        // bit7=0 (AC-3), bit5=1 (full service), bits[4:3]=00 (CM), bits[2:1]=10 (stereo)
        // 0_?_1_00_10_0 = 0b0010_0100 = 0x24
        let d = Ac3Descriptor {
            component_type: Some(0x24),
            bsid: None,
            mainid: None,
            asvc: None,
            additional_info: &[],
        };
        let ct = d.decoded_component_type().unwrap();
        assert!(!ct.enhanced_ac3);
        assert!(ct.full_service);
        assert!(matches!(ct.service_type, Ac3ServiceType::CompleteMain));
        assert!(matches!(ct.channels, Ac3ChannelMode::Stereo));
    }

    #[test]
    fn decode_component_type_none() {
        let d = Ac3Descriptor {
            component_type: None,
            bsid: None,
            mainid: None,
            asvc: None,
            additional_info: &[],
        };
        assert!(d.decoded_component_type().is_none());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            Ac3Descriptor::parse(&[0x7A, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7A, .. }
        ));
    }

    #[test]
    fn parse_rejects_flags_past_length() {
        // flags claim component_type but length=1 covers only the flags byte.
        let bytes = [TAG, 1, FLAG_COMPONENT_TYPE];
        assert!(matches!(
            Ac3Descriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = Ac3Descriptor {
            component_type: Some(0x40),
            bsid: Some(8),
            mainid: None,
            asvc: None,
            additional_info: &[0xFE, 0xED],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(Ac3Descriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn parse_rejects_empty_body() {
        let bytes = [TAG, 0];
        assert!(matches!(
            Ac3Descriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }
}
