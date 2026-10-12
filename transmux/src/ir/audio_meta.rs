//! Typed audio-track metadata recovered from a TS track's PMT `ES_info`
//! descriptor loop (issue #1162 R1).
//!
//! The loop is carried verbatim on [`TrackSpec::es_info_descriptors`]; this
//! module decodes the descriptors that identify *what an audio track is for*,
//! reusing `dvb-si`'s parsers (no second descriptor parser):
//!
//! - `ISO_639_language_descriptor` (ISO/IEC 13818-1 §2.6.18, tag `0x0A`):
//!   repeated `ISO_639_language_code` + `audio_type` (§2.6.19 Table 2-60 /
//!   ETSI EN 300 468 §6.2.22).
//! - DVB `supplementary_audio_descriptor` (ETSI EN 300 468 §6.4.11, extension
//!   descriptor tag `0x7F`, `descriptor_tag_extension` `0x06`): `mix_type`
//!   (Table 154), `editorial_classification` (Table 155), optional language.
//!
//! # Derived [`AudioRole`]
//!
//! Broadcasters signal the same purpose in different places, so the role is a
//! documented heuristic over both descriptors, **not** a spec-defined field.
//! Rules, in priority order:
//!
//! 1. [`AudioRole::AudioDescription`] when any of: an `audio_type` of
//!    `0x03` (visual impaired commentary); a supplementary
//!    `editorial_classification` of `0x01` (audio description for the
//!    visually impaired); or an ISO 639 language code `qad` (the
//!    locally-assigned code France 2 / TF1 use for their AD track, alongside
//!    `editorial_classification = 0x01`).
//! 2. [`AudioRole::HearingImpaired`] when `audio_type = 0x02` (hearing
//!    impaired, e.g. ORF) or `editorial_classification = 0x02` (clean audio
//!    for the hearing impaired).
//! 3. [`AudioRole::Main`] when the loop identifies the track (a language entry
//!    and/or a supplementary descriptor) and nothing above matched, and the
//!    supplementary descriptor (if any) is `editorial_classification = 0x00`
//!    (main audio, ETSI EN 300 468 Annex J.4). Notably `audio_type = 0x01`
//!    (clean effects) does **not** imply non-main: ServusTV tags its main mix
//!    that way, and Annex J.4 lists "main audio" as `audio_type` `0x00` or
//!    `0x01`.
//! 4. [`AudioRole::Unknown`] otherwise: no descriptors, or a supplementary
//!    classification that is none of the above (spoken subtitles, dependent
//!    parametric data, general-audience supplementary audio, user-defined,
//!    reserved).
//!
//! Audio description wins over hearing impaired if a (malformed) loop signals
//! both.

use alloc::string::String;
use alloc::vec::Vec;

use dvb_si::descriptors::any::{AnyDescriptor, parse_loop};
use dvb_si::descriptors::extension::ExtensionBody;
use dvb_si::descriptors::iso_639_language::AudioType as DvbAudioType;

use super::TrackSpec;

/// Language code France 2 / TF1 use for their audio-description track.
const LANG_AUDIO_DESCRIPTION: &str = "qad";

/// `editorial_classification` values (ETSI EN 300 468 Table 155; `0x00` is
/// "main audio" per Annex J.4).
const EDITORIAL_MAIN: u8 = 0x00;
const EDITORIAL_AUDIO_DESCRIPTION: u8 = 0x01;
const EDITORIAL_CLEAN_AUDIO_HI: u8 = 0x02;
const EDITORIAL_SPOKEN_SUBTITLES: u8 = 0x03;
const EDITORIAL_PARAMETRIC: u8 = 0x04;
const EDITORIAL_GENERAL_AUDIENCE: u8 = 0x17;
const EDITORIAL_USER_DEFINED_MIN: u8 = 0x18;
const EDITORIAL_USER_DEFINED_MAX: u8 = 0x1F;

/// `audio_type` value of an `ISO_639_language_descriptor` entry — ISO/IEC
/// 13818-1 §2.6.19 Table 2-60.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioType {
    /// `0x00` — undefined.
    Undefined,
    /// `0x01` — clean effects.
    CleanEffects,
    /// `0x02` — hearing impaired.
    HearingImpaired,
    /// `0x03` — visual impaired commentary.
    VisualImpairedCommentary,
    /// Any other value (user private `0x04..=0x7F`, DVB-extended `0x80..=0x84`,
    /// reserved), carried losslessly.
    Other(u8),
}

impl AudioType {
    fn from_dvb(t: DvbAudioType) -> Self {
        match t {
            DvbAudioType::Undefined => Self::Undefined,
            DvbAudioType::CleanEffects => Self::CleanEffects,
            DvbAudioType::HearingImpaired => Self::HearingImpaired,
            DvbAudioType::VisualImpairedCommentary => Self::VisualImpairedCommentary,
            other => Self::Other(other.to_u8()),
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::CleanEffects => "clean effects",
            Self::HearingImpaired => "hearing impaired",
            Self::VisualImpairedCommentary => "visual impaired commentary",
            Self::Other(_) => "other",
        }
    }
}
broadcast_common::impl_spec_display!(AudioType, Other);

/// `mix_type` — ETSI EN 300 468 Table 154.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MixType {
    /// `0` — a dependent stream, to be mixed by the receiver with a separate
    /// complete and independent stream.
    Dependent,
    /// `1` — a complete and independent stream.
    Independent,
}

impl MixType {
    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dependent => "dependent",
            Self::Independent => "independent",
        }
    }
}
broadcast_common::impl_spec_display!(MixType);

/// `editorial_classification` — ETSI EN 300 468 Table 155 (value `0x00` is
/// "main audio" per Annex J.4's audio-purpose table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EditorialClassification {
    /// `0x00` — main audio (Annex J.4).
    MainAudio,
    /// `0x01` — audio description for the visually impaired.
    AudioDescription,
    /// `0x02` — clean audio for the hearing impaired.
    CleanAudioHearingImpaired,
    /// `0x03` — spoken subtitles for the visually impaired.
    SpokenSubtitles,
    /// `0x04` — dependent parametric data stream (not embedded).
    DependentParametricData,
    /// `0x17` — unspecific supplementary audio for the general audience.
    GeneralAudience,
    /// `0x18..=0x1F` — user defined.
    UserDefined(u8),
    /// `0x05..=0x16` — reserved for future use.
    Reserved(u8),
}

impl EditorialClassification {
    fn from_u8(v: u8) -> Self {
        match v {
            EDITORIAL_MAIN => Self::MainAudio,
            EDITORIAL_AUDIO_DESCRIPTION => Self::AudioDescription,
            EDITORIAL_CLEAN_AUDIO_HI => Self::CleanAudioHearingImpaired,
            EDITORIAL_SPOKEN_SUBTITLES => Self::SpokenSubtitles,
            EDITORIAL_PARAMETRIC => Self::DependentParametricData,
            EDITORIAL_GENERAL_AUDIENCE => Self::GeneralAudience,
            EDITORIAL_USER_DEFINED_MIN..=EDITORIAL_USER_DEFINED_MAX => Self::UserDefined(v),
            v => Self::Reserved(v),
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MainAudio => "main audio",
            Self::AudioDescription => "audio description",
            Self::CleanAudioHearingImpaired => "clean audio for the hearing impaired",
            Self::SpokenSubtitles => "spoken subtitles",
            Self::DependentParametricData => "dependent parametric data",
            Self::GeneralAudience => "general audience supplementary audio",
            Self::UserDefined(_) => "user defined",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(EditorialClassification, UserDefined, Reserved);

/// What an audio track is for, derived from its PMT descriptors — see the
/// [module docs](self) for the exact (heuristic) rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioRole {
    /// The programme's main audio.
    Main,
    /// Audio description for the visually impaired.
    AudioDescription,
    /// Hearing-impaired / clean-audio service.
    HearingImpaired,
    /// Not determinable from the descriptors.
    Unknown,
}

impl AudioRole {
    /// Label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::AudioDescription => "audio description",
            Self::HearingImpaired => "hearing impaired",
            Self::Unknown => "unknown",
        }
    }
}
broadcast_common::impl_spec_display!(AudioRole);

/// One `ISO_639_language_descriptor` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioLanguage {
    /// ISO 639-2 three-letter code, lower-cased (e.g. `"fre"`, `"qad"`).
    pub language: String,
    /// The entry's `audio_type`.
    pub audio_type: AudioType,
}

/// A decoded `supplementary_audio_descriptor` (ETSI EN 300 468 §6.4.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SupplementaryAudio {
    /// `mix_type` (Table 154).
    pub mix_type: MixType,
    /// `editorial_classification` (Table 155).
    pub editorial_classification: EditorialClassification,
    /// The descriptor's own language code, lower-cased, when present.
    pub language: Option<String>,
}

/// Typed audio metadata of one track; see [`TrackSpec::audio_metadata`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioMetadata {
    /// Every `ISO_639_language_descriptor` entry, in loop order.
    pub languages: Vec<AudioLanguage>,
    /// The first `supplementary_audio_descriptor`, if any.
    pub supplementary: Option<SupplementaryAudio>,
    /// Derived role (heuristic, see the module docs).
    pub role: AudioRole,
}

impl AudioMetadata {
    /// Decode a PMT `ES_info` descriptor loop. Malformed descriptors are
    /// skipped (never an error): this is best-effort metadata over a broadcast
    /// signal.
    pub fn from_es_info(es_info: &[u8]) -> Self {
        let mut languages = Vec::new();
        let mut supplementary = None;
        for d in parse_loop(es_info).flatten() {
            match d {
                AnyDescriptor::Iso639Language(d) => {
                    for e in d.entries {
                        languages.push(AudioLanguage {
                            language: e.language_code.as_str().to_ascii_lowercase(),
                            audio_type: AudioType::from_dvb(e.audio_type),
                        });
                    }
                }
                AnyDescriptor::Extension(x) => {
                    if let ExtensionBody::SupplementaryAudio(s) = x.body
                        && supplementary.is_none()
                    {
                        supplementary = Some(SupplementaryAudio {
                            mix_type: if s.mix_type {
                                MixType::Independent
                            } else {
                                MixType::Dependent
                            },
                            editorial_classification: EditorialClassification::from_u8(
                                s.editorial_classification,
                            ),
                            language: s
                                .iso_639_language_code
                                .map(|l| l.as_str().to_ascii_lowercase()),
                        });
                    }
                }
                _ => {}
            }
        }
        let role = derive_role(&languages, supplementary.as_ref());
        Self {
            languages,
            supplementary,
            role,
        }
    }
}

fn derive_role(languages: &[AudioLanguage], sup: Option<&SupplementaryAudio>) -> AudioRole {
    let editorial = sup.map(|s| s.editorial_classification);
    let lang_is_ad = languages
        .iter()
        .any(|l| l.language == LANG_AUDIO_DESCRIPTION)
        || sup
            .and_then(|s| s.language.as_deref())
            .is_some_and(|l| l == LANG_AUDIO_DESCRIPTION);
    if languages
        .iter()
        .any(|l| l.audio_type == AudioType::VisualImpairedCommentary)
        || editorial == Some(EditorialClassification::AudioDescription)
        || lang_is_ad
    {
        return AudioRole::AudioDescription;
    }
    if languages
        .iter()
        .any(|l| l.audio_type == AudioType::HearingImpaired)
        || editorial == Some(EditorialClassification::CleanAudioHearingImpaired)
    {
        return AudioRole::HearingImpaired;
    }
    let identified = !languages.is_empty() || sup.is_some();
    match editorial {
        None | Some(EditorialClassification::MainAudio) if identified => AudioRole::Main,
        _ => AudioRole::Unknown,
    }
}

impl TrackSpec {
    /// Typed audio metadata decoded from [`Self::es_info_descriptors`]
    /// (language(s), `audio_type`, supplementary-audio signalling, derived
    /// [`AudioRole`]). Empty with [`AudioRole::Unknown`] for a non-TS source
    /// or a loop carrying none of those descriptors.
    pub fn audio_metadata(&self) -> AudioMetadata {
        AudioMetadata::from_es_info(&self.es_info_descriptors)
    }
}
