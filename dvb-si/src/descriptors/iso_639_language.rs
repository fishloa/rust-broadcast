//! ISO 639 Language Descriptor — MPEG-2 ISO/IEC 13818-1 §2.6.19 (tag 0x0A).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::LangCode;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for iso_639_language_descriptor.
pub const TAG: u8 = 0x0A;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 4;

/// Audio type — ISO/IEC 13818-1 §2.6.19 Table 2-63.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioType {
    /// 0x00 — undefined.
    Undefined,
    /// 0x01 — clean effects.
    CleanEffects,
    /// 0x02 — hearing impaired.
    HearingImpaired,
    /// 0x03 — visual impaired commentary.
    VisualImpairedCommentary,
    /// 0x04–0x7F — user private.
    UserPrivate(u8),
    /// 0x80 — primary.
    Primary,
    /// 0x81 — native.
    Native,
    /// 0x82 — emergency.
    Emergency,
    /// 0x83 — primary commentary.
    PrimaryCommentary,
    /// 0x84 — alternate commentary.
    AlternateCommentary,
    /// 0x85–0xFF — reserved.
    Reserved(u8),
}

impl AudioType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Undefined,
            0x01 => Self::CleanEffects,
            0x02 => Self::HearingImpaired,
            0x03 => Self::VisualImpairedCommentary,
            0x04..=0x7F => Self::UserPrivate(v),
            0x80 => Self::Primary,
            0x81 => Self::Native,
            0x82 => Self::Emergency,
            0x83 => Self::PrimaryCommentary,
            0x84 => Self::AlternateCommentary,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Undefined => 0x00,
            Self::CleanEffects => 0x01,
            Self::HearingImpaired => 0x02,
            Self::VisualImpairedCommentary => 0x03,
            Self::UserPrivate(v) => v,
            Self::Primary => 0x80,
            Self::Native => 0x81,
            Self::Emergency => 0x82,
            Self::PrimaryCommentary => 0x83,
            Self::AlternateCommentary => 0x84,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::CleanEffects => "clean effects",
            Self::HearingImpaired => "hearing impaired",
            Self::VisualImpairedCommentary => "visual impaired commentary",
            Self::UserPrivate(_) => "user private",
            Self::Primary => "primary",
            Self::Native => "native",
            Self::Emergency => "emergency",
            Self::PrimaryCommentary => "primary commentary",
            Self::AlternateCommentary => "alternate commentary",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// One (language code, audio type) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LanguageEntry {
    /// Three-character ISO 639-2 language code (e.g. `LangCode(*b"eng")`).
    pub language_code: LangCode,
    /// Audio type (ETSI EN 300 468 §6.2.22).
    pub audio_type: AudioType,
}

/// ISO 639 Language Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Iso639LanguageDescriptor {
    /// One or more language entries.
    pub entries: Vec<LanguageEntry>,
}

impl<'a> Parse<'a> for Iso639LanguageDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Iso639LanguageDescriptor",
            "unexpected tag for iso_639_language_descriptor",
        )?;
        if body.len() % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "iso_639_language_descriptor length not a multiple of 4",
            });
        }
        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            entries.push(LanguageEntry {
                language_code: LangCode([chunk[0], chunk[1], chunk[2]]),
                audio_type: AudioType::from_u8(chunk[3]),
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for Iso639LanguageDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
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
        buf[1] = (self.entries.len() * ENTRY_LEN) as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 3].copy_from_slice(&e.language_code.0);
            buf[pos + 3] = e.audio_type.to_u8();
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for Iso639LanguageDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ISO_639_LANGUAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_language_entry() {
        let bytes = [TAG, 4, b'e', b'n', b'g', 0x00];
        let d = Iso639LanguageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].language_code, LangCode(*b"eng"));
        assert_eq!(d.entries[0].audio_type, AudioType::Undefined);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [TAG, 8, b'e', b'n', b'g', 1, b'f', b'r', b'a', 2];
        let d = Iso639LanguageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[1].language_code, LangCode(*b"fra"));
        assert_eq!(d.entries[1].audio_type, AudioType::HearingImpaired);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = Iso639LanguageDescriptor::parse(&[0x0B, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0B, .. }));
    }

    #[test]
    fn parse_rejects_short_header() {
        let err = Iso639LanguageDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_4() {
        let bytes = [TAG, 5, b'e', b'n', b'g', 0, 0];
        let err = Iso639LanguageDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = Iso639LanguageDescriptor {
            entries: vec![
                LanguageEntry {
                    language_code: LangCode(*b"eng"),
                    audio_type: AudioType::Undefined,
                },
                LanguageEntry {
                    language_code: LangCode(*b"fra"),
                    audio_type: AudioType::CleanEffects,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = Iso639LanguageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = Iso639LanguageDescriptor {
            entries: vec![LanguageEntry {
                language_code: LangCode(*b"eng"),
                audio_type: AudioType::Undefined,
            }],
        };
        assert_eq!(d.serialized_len() - 2, 4);
    }

    #[test]
    fn audio_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let at = AudioType::from_u8(b);
            assert_eq!(at.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }

    #[test]
    fn audio_type_name_for_known() {
        assert_eq!(AudioType::Undefined.name(), "undefined");
        assert_eq!(AudioType::CleanEffects.name(), "clean effects");
        assert_eq!(AudioType::HearingImpaired.name(), "hearing impaired");
        assert_eq!(
            AudioType::VisualImpairedCommentary.name(),
            "visual impaired commentary"
        );
        assert_eq!(AudioType::Reserved(0x55).name(), "reserved");
    }

    #[test]
    fn audio_type_round_trip_known_values() {
        for b in [0x00u8, 0x03, 0x40, 0x80, 0x84, 0x85, 0xFF] {
            assert_eq!(
                AudioType::from_u8(b).to_u8(),
                b,
                "round-trip failed for byte 0x{b:02X}"
            );
        }
    }

    #[test]
    fn audio_type_primary_name() {
        assert_eq!(AudioType::from_u8(0x80).name(), "primary");
    }
}
