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

const COMPONENT_TYPE_ENHANCED_AC3_MASK: u8 = 0x80;
const COMPONENT_TYPE_FULL_SERVICE_MASK: u8 = 0x40;
const COMPONENT_TYPE_SERVICE_TYPE_SHIFT: u8 = 3;
const COMPONENT_TYPE_SERVICE_TYPE_MASK: u8 = 0x07;
const COMPONENT_TYPE_CHANNELS_MASK: u8 = 0x07;

/// AC-3 / Enhanced AC-3 service type — EN 300 468 Annex D Table D.4.
///
/// 3-bit field `[5:3]` of the component_type byte. Values 0–7 are assigned
/// by the spec; the `Unknown` variant carries any value outside that range
/// (should not occur for a 3-bit field but preserves the raw value for
/// round-trip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Ac3ServiceType {
    /// Complete Main (CM) — valid when `full_service` is set.
    CompleteMain,
    /// Music and Effects (ME) — valid when `full_service` is not set.
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
    /// Voice Over (VO).
    VoiceOver,
    /// Unknown/reserved service type value.
    Unknown(u8),
}

impl Ac3ServiceType {
    /// Construct from the raw 3-bit value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::CompleteMain,
            1 => Self::MusicAndEffects,
            2 => Self::VisuallyImpaired,
            3 => Self::HearingImpaired,
            4 => Self::Dialogue,
            5 => Self::Commentary,
            6 => Self::Emergency,
            7 => Self::VoiceOver,
            _ => Self::Unknown(v),
        }
    }

    /// Return the raw 3-bit value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::CompleteMain => 0,
            Self::MusicAndEffects => 1,
            Self::VisuallyImpaired => 2,
            Self::HearingImpaired => 3,
            Self::Dialogue => 4,
            Self::Commentary => 5,
            Self::Emergency => 6,
            Self::VoiceOver => 7,
            Self::Unknown(v) => v,
        }
    }

    /// Returns a human-readable name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::CompleteMain => "Complete Main (CM)",
            Self::MusicAndEffects => "Music and Effects (ME)",
            Self::VisuallyImpaired => "Visually Impaired (VI)",
            Self::HearingImpaired => "Hearing Impaired (HI)",
            Self::Dialogue => "Dialogue (D)",
            Self::Commentary => "Commentary (C)",
            Self::Emergency => "Emergency (E)",
            Self::VoiceOver => "Voice Over (VO)",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// AC-3 / Enhanced AC-3 channel mode — EN 300 468 Annex D Table D.5.
///
/// 3-bit field `[2:0]` of the component_type byte. Values 0–6 are assigned
/// by the spec; value 7 is reserved for future use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Ac3ChannelMode {
    /// Mono.
    Mono,
    /// 1+1 Mode (dual mono).
    OnePlusOne,
    /// 2 channel (stereo).
    Stereo,
    /// 2 channel Surround encoded (Dolby surround).
    SurroundEncodedStereo,
    /// Multichannel audio (> 2 channels).
    Multichannel,
    /// Multichannel audio (> 5.1 channels).
    MultichannelAbove51,
    /// Multiple programmes in independent substreams.
    MultipleSubstreams,
    /// Reserved for future use (code 7).
    Reserved,
    /// Unknown value outside the 3-bit domain.
    Unknown(u8),
}

impl Ac3ChannelMode {
    /// Construct from the raw 3-bit value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Mono,
            1 => Self::OnePlusOne,
            2 => Self::Stereo,
            3 => Self::SurroundEncodedStereo,
            4 => Self::Multichannel,
            5 => Self::MultichannelAbove51,
            6 => Self::MultipleSubstreams,
            7 => Self::Reserved,
            _ => Self::Unknown(v),
        }
    }

    /// Return the raw 3-bit value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Mono => 0,
            Self::OnePlusOne => 1,
            Self::Stereo => 2,
            Self::SurroundEncodedStereo => 3,
            Self::Multichannel => 4,
            Self::MultichannelAbove51 => 5,
            Self::MultipleSubstreams => 6,
            Self::Reserved => 7,
            Self::Unknown(v) => v,
        }
    }

    /// Returns a human-readable name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::OnePlusOne => "1+1 Mode",
            Self::Stereo => "2 channel (stereo)",
            Self::SurroundEncodedStereo => "2 channel Surround encoded (Dolby surround)",
            Self::Multichannel => "Multichannel audio (> 2 channels)",
            Self::MultichannelAbove51 => "Multichannel audio (> 5.1 channels)",
            Self::MultipleSubstreams => "Multiple programmes in independent substreams",
            Self::Reserved => "reserved",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// Decoded AC-3 component_type — ETSI EN 300 468 Annex D Table D.1.
///
/// The component_type byte packs bit-fields describing the audio service type,
/// number of channels, and whether the stream is AC-3 or Enhanced AC-3:
///
/// - `[7]` — Enhanced AC-3 flag (Table D.2)
/// - `[6]` — Full service flag (Table D.3)
/// - `[5:3]` — Service type flags (Table D.4)
/// - `[2:0]` — Number of channels flags (Table D.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Ac3ComponentType {
    /// `false` = AC-3, `true` = Enhanced AC-3 (`[7]`).
    pub enhanced_ac3: bool,
    /// `true` if this is a full service (suitable for solo presentation) (`[6]`).
    pub full_service: bool,
    /// Decoded service type (`[5:3]`).
    pub service_type: Ac3ServiceType,
    /// Number of audio channels (`[2:0]`).
    pub channels: Ac3ChannelMode,
}

impl Ac3ComponentType {
    /// Decode a component_type byte per ETSI EN 300 468 Annex D Table D.1.
    ///
    /// Bit layout: `[7]` = enhanced AC-3 flag, `[6]` = full service flag,
    /// `[5:3]` = service type, `[2:0]` = number of channels.
    #[must_use]
    pub fn from_byte(byte: u8) -> Self {
        let enhanced_ac3 = (byte & COMPONENT_TYPE_ENHANCED_AC3_MASK) != 0;
        let full_service = (byte & COMPONENT_TYPE_FULL_SERVICE_MASK) != 0;
        let service_type = Ac3ServiceType::from_u8(
            (byte >> COMPONENT_TYPE_SERVICE_TYPE_SHIFT) & COMPONENT_TYPE_SERVICE_TYPE_MASK,
        );
        let channels = Ac3ChannelMode::from_u8(byte & COMPONENT_TYPE_CHANNELS_MASK);
        Self {
            enhanced_ac3,
            full_service,
            service_type,
            channels,
        }
    }

    /// Encode back to the wire byte. Lossless: `from_byte(b).to_byte() == b`.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        (self.enhanced_ac3 as u8) << 7
            | (self.full_service as u8) << 6
            | (self.service_type.to_u8() & COMPONENT_TYPE_SERVICE_TYPE_MASK)
                << COMPONENT_TYPE_SERVICE_TYPE_SHIFT
            | (self.channels.to_u8() & COMPONENT_TYPE_CHANNELS_MASK)
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
        Some(Ac3ComponentType::from_byte(self.component_type?))
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
    fn decode_component_type_full_service_cm_stereo() {
        // 0x42 = 0b01000010: bit7=0 (AC-3), bit6=1 (full service),
        // bits[5:3]=000 (CM), bits[2:0]=010 (stereo).
        let ct = Ac3ComponentType::from_byte(0x42);
        assert!(!ct.enhanced_ac3);
        assert!(ct.full_service);
        assert_eq!(ct.service_type, Ac3ServiceType::CompleteMain);
        assert_eq!(ct.channels, Ac3ChannelMode::Stereo);
    }

    #[test]
    fn decode_component_type_enhanced_me_1plus1() {
        // 0x89 = 0b10001001: bit7=1 (E-AC-3), bit6=0 (not full service),
        // bits[5:3]=001 (ME), bits[2:0]=001 (1+1).
        let ct = Ac3ComponentType::from_byte(0x89);
        assert!(ct.enhanced_ac3);
        assert!(!ct.full_service);
        assert_eq!(ct.service_type, Ac3ServiceType::MusicAndEffects);
        assert_eq!(ct.channels, Ac3ChannelMode::OnePlusOne);
    }

    #[test]
    fn decode_component_type_vi_surround() {
        // 0x53 = 0b01_010_011: enhanced=0, full_service=1, service=010 (VI),
        // channels=011 (stereo + Dolby surround).
        let ct = Ac3ComponentType::from_byte(0x53);
        assert!(!ct.enhanced_ac3);
        assert!(ct.full_service);
        assert_eq!(ct.service_type, Ac3ServiceType::VisuallyImpaired);
        assert_eq!(ct.channels, Ac3ChannelMode::SurroundEncodedStereo);
    }

    #[test]
    fn decode_component_type_emergency_mono() {
        // enhanced=0, full=0, service=110(E), channels=000(mono)
        // = 0b00_110_000 = 0x30
        let ct = Ac3ComponentType::from_byte(0x30);
        assert!(!ct.enhanced_ac3);
        assert!(!ct.full_service);
        assert_eq!(ct.service_type, Ac3ServiceType::Emergency);
        assert_eq!(ct.channels, Ac3ChannelMode::Mono);
    }

    #[test]
    fn decode_component_type_reserved_channels() {
        // channels=7 (reserved): enhanced=1, full=1, service=000(CM), channels=111
        // = 0b11_000_111 = 0xC7
        let ct = Ac3ComponentType::from_byte(0xC7);
        assert!(ct.enhanced_ac3);
        assert!(ct.full_service);
        assert_eq!(ct.service_type, Ac3ServiceType::CompleteMain);
        assert_eq!(ct.channels, Ac3ChannelMode::Reserved);
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
    fn component_type_round_trip_all_bytes() {
        for b in 0u8..=255 {
            let ct = Ac3ComponentType::from_byte(b);
            assert_eq!(ct.to_byte(), b, "round-trip failed for byte {b:#04x}");
        }
    }

    #[test]
    fn service_type_round_trip() {
        for v in 0u8..=7 {
            let st = Ac3ServiceType::from_u8(v);
            assert_eq!(st.to_u8(), v, "service_type round-trip failed for {v}");
        }
        // Unknown fallback preserves the raw value.
        assert_eq!(Ac3ServiceType::Unknown(42).to_u8(), 42);
    }

    #[test]
    fn channel_mode_round_trip() {
        for v in 0u8..=7 {
            let cm = Ac3ChannelMode::from_u8(v);
            assert_eq!(cm.to_u8(), v, "channel_mode round-trip failed for {v}");
        }
        assert_eq!(Ac3ChannelMode::Unknown(42).to_u8(), 42);
    }

    #[test]
    fn service_type_name() {
        assert_eq!(Ac3ServiceType::CompleteMain.name(), "Complete Main (CM)");
        assert_eq!(Ac3ServiceType::Dialogue.name(), "Dialogue (D)");
        assert_eq!(Ac3ServiceType::Unknown(99).name(), "unknown");
    }

    #[test]
    fn channel_mode_name() {
        assert_eq!(Ac3ChannelMode::Mono.name(), "Mono");
        assert_eq!(
            Ac3ChannelMode::SurroundEncodedStereo.name(),
            "2 channel Surround encoded (Dolby surround)"
        );
        assert_eq!(Ac3ChannelMode::Reserved.name(), "reserved");
        assert_eq!(Ac3ChannelMode::Unknown(99).name(), "unknown");
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
