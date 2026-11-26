//! The codec profiles the codec specifications define.
//!
//! The variant names are the profiles' own names: ITU-T H.264 Annex A, ITU-T
//! H.265 Annex A, ITU-T H.266 Annex A, the AOMedia AV1 specification section
//! 7.2, the VP9 bitstream specification section 6.2, ISO/IEC 13818-2 section 6
//! and ISO/IEC 14496-3 section 1.5.2. A profile identifier number belongs to
//! the bitstream that carries it, so this crate names none: an adapter that
//! reads a profile index maps it to one of these variants.

use alloc::format;
use alloc::string::String;

use crate::codec::{CodecId, compact, same_token, token};

/// Separates the codec token from the profile token in the text form.
const TOKEN_SEPARATOR: char = '/';
/// An [`CodecProfile::Other`] tag is written as this many hexadecimal digits.
const OTHER_TAG_DIGITS: usize = 8;
/// Base of those digits.
const OTHER_TAG_RADIX: u32 = 16;

/// The H.264 profiles (ITU-T H.264 Annex A).
const H264_PROFILES: &[CodecProfile] = &[
    CodecProfile::H264Baseline,
    CodecProfile::H264ConstrainedBaseline,
    CodecProfile::H264Main,
    CodecProfile::H264Extended,
    CodecProfile::H264High,
    CodecProfile::H264High10,
    CodecProfile::H264High422,
    CodecProfile::H264High444Predictive,
];

/// The HEVC profiles (ITU-T H.265 Annex A).
const HEVC_PROFILES: &[CodecProfile] = &[
    CodecProfile::HevcMain,
    CodecProfile::HevcMain10,
    CodecProfile::HevcMainStillPicture,
    CodecProfile::HevcRext,
];

/// The VVC profiles (ITU-T H.266 Annex A).
const VVC_PROFILES: &[CodecProfile] =
    &[CodecProfile::VvcMain10, CodecProfile::VvcMain10FourFourFour];

/// The AV1 profiles (AOMedia AV1 specification section 7.2).
const AV1_PROFILES: &[CodecProfile] = &[
    CodecProfile::Av1Main,
    CodecProfile::Av1High,
    CodecProfile::Av1Professional,
];

/// The VP9 profiles (VP9 bitstream specification section 6.2).
const VP9_PROFILES: &[CodecProfile] = &[
    CodecProfile::Vp9Profile0,
    CodecProfile::Vp9Profile1,
    CodecProfile::Vp9Profile2,
    CodecProfile::Vp9Profile3,
];

/// The MPEG-2 video profiles (ISO/IEC 13818-2 section 6).
const MPEG2_PROFILES: &[CodecProfile] = &[
    CodecProfile::Mpeg2Simple,
    CodecProfile::Mpeg2Main,
    CodecProfile::Mpeg2High,
    CodecProfile::Mpeg2FourTwoTwo,
];

/// The AAC profiles (ISO/IEC 14496-3 section 1.5.2).
const AAC_PROFILES: &[CodecProfile] = &[
    CodecProfile::AacLc,
    CodecProfile::AacHe,
    CodecProfile::AacHeV2,
    CodecProfile::AacLd,
    CodecProfile::AacEld,
    CodecProfile::AacXhe,
];

/// The named profiles of a codec this module has to offer.
const NO_PROFILES: &[CodecProfile] = &[];

/// One variant of a codec, named the way the codec's specification names it.
///
/// A profile is a typed key two sides share: a stream says which profile it
/// carries, an implementation says which profiles it handles, and the registry
/// never offers a stream to an implementation that states it cannot handle it.
/// The text form is the codec's token and the profile token joined by `/`
/// (`h264/high10`, `aac/lc`); [`CodecProfile::token`] writes it and
/// [`CodecProfile::from_token`] reads it back.
///
/// [`CodecProfile::Other`] carries a codec and a caller-chosen tag for a
/// profile this enum does not name; the seam never interprets the tag.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecProfile {
    /// H.264 Baseline.
    H264Baseline,
    /// H.264 Constrained Baseline.
    H264ConstrainedBaseline,
    /// H.264 Main.
    H264Main,
    /// H.264 Extended.
    H264Extended,
    /// H.264 High.
    H264High,
    /// H.264 High 10.
    H264High10,
    /// H.264 High 4:2:2.
    H264High422,
    /// H.264 High 4:4:4 Predictive.
    H264High444Predictive,
    /// HEVC Main.
    HevcMain,
    /// HEVC Main 10.
    HevcMain10,
    /// HEVC Main Still Picture.
    HevcMainStillPicture,
    /// HEVC format range extensions.
    HevcRext,
    /// VVC Main 10.
    VvcMain10,
    /// VVC Main 10 4:4:4.
    VvcMain10FourFourFour,
    /// AV1 Main.
    Av1Main,
    /// AV1 High.
    Av1High,
    /// AV1 Professional.
    Av1Professional,
    /// VP9 Profile 0.
    Vp9Profile0,
    /// VP9 Profile 1.
    Vp9Profile1,
    /// VP9 Profile 2.
    Vp9Profile2,
    /// VP9 Profile 3.
    Vp9Profile3,
    /// MPEG-2 video Simple.
    Mpeg2Simple,
    /// MPEG-2 video Main.
    Mpeg2Main,
    /// MPEG-2 video High.
    Mpeg2High,
    /// MPEG-2 video 4:2:2.
    Mpeg2FourTwoTwo,
    /// AAC Low Complexity.
    AacLc,
    /// AAC High Efficiency.
    AacHe,
    /// AAC High Efficiency v2.
    AacHeV2,
    /// AAC Low Delay.
    AacLd,
    /// AAC Enhanced Low Delay.
    AacEld,
    /// AAC extended High Efficiency.
    AacXhe,
    /// A profile this enum does not name, of `codec`, tagged `tag`.
    Other {
        /// The codec the profile belongs to.
        codec: CodecId,
        /// A caller-chosen tag; the seam never interprets it.
        tag: u32,
    },
}

impl CodecProfile {
    /// The codec the profile belongs to.
    #[must_use]
    pub const fn codec(&self) -> CodecId {
        match self {
            Self::H264Baseline
            | Self::H264ConstrainedBaseline
            | Self::H264Main
            | Self::H264Extended
            | Self::H264High
            | Self::H264High10
            | Self::H264High422
            | Self::H264High444Predictive => CodecId::H264,
            Self::HevcMain | Self::HevcMain10 | Self::HevcMainStillPicture | Self::HevcRext => {
                CodecId::Hevc
            }
            Self::VvcMain10 | Self::VvcMain10FourFourFour => CodecId::Vvc,
            Self::Av1Main | Self::Av1High | Self::Av1Professional => CodecId::Av1,
            Self::Vp9Profile0 | Self::Vp9Profile1 | Self::Vp9Profile2 | Self::Vp9Profile3 => {
                CodecId::Vp9
            }
            Self::Mpeg2Simple | Self::Mpeg2Main | Self::Mpeg2High | Self::Mpeg2FourTwoTwo => {
                CodecId::Mpeg2Video
            }
            Self::AacLc
            | Self::AacHe
            | Self::AacHeV2
            | Self::AacLd
            | Self::AacEld
            | Self::AacXhe => CodecId::Aac,
            Self::Other { codec, .. } => *codec,
        }
    }

    /// Label of the profile, as the codec's specification names it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::H264Baseline => "Baseline",
            Self::H264ConstrainedBaseline => "Constrained Baseline",
            Self::H264Main => "Main",
            Self::H264Extended => "Extended",
            Self::H264High => "High",
            Self::H264High10 => "High 10",
            Self::H264High422 => "High 4:2:2",
            Self::H264High444Predictive => "High 4:4:4 Predictive",
            Self::HevcMain => "Main",
            Self::HevcMain10 => "Main 10",
            Self::HevcMainStillPicture => "Main Still Picture",
            Self::HevcRext => "RExt",
            Self::VvcMain10 => "Main 10",
            Self::VvcMain10FourFourFour => "Main 10 4:4:4",
            Self::Av1Main => "Main",
            Self::Av1High => "High",
            Self::Av1Professional => "Professional",
            Self::Vp9Profile0 => "Profile 0",
            Self::Vp9Profile1 => "Profile 1",
            Self::Vp9Profile2 => "Profile 2",
            Self::Vp9Profile3 => "Profile 3",
            Self::Mpeg2Simple => "Simple",
            Self::Mpeg2Main => "Main",
            Self::Mpeg2High => "High",
            Self::Mpeg2FourTwoTwo => "4:2:2",
            Self::AacLc => "LC",
            Self::AacHe => "HE",
            Self::AacHeV2 => "HE v2",
            Self::AacLd => "LD",
            Self::AacEld => "ELD",
            Self::AacXhe => "xHE",
            Self::Other { .. } => "other",
        }
    }

    /// The compact lower-case token of the profile alone: the label with case,
    /// spaces and punctuation removed (`baseline`, `high10`), or an
    /// [`Self::Other`] tag as eight lower-case hexadecimal digits.
    ///
    /// The inventory's profiles column prints this, because the row already
    /// names the codec.
    #[must_use]
    pub fn profile_token(&self) -> String {
        match self {
            Self::Other { tag, .. } => format!("{tag:08x}"),
            _ => compact(self.name()),
        }
    }

    /// The profile's full text form: its codec's token and the profile token
    /// joined by `/` (`h264/high10`, `other:00000007/deadbeef`).
    ///
    /// [`Self::from_token`] reads it back exactly.
    #[must_use]
    pub fn token(&self) -> String {
        format!(
            "{}{TOKEN_SEPARATOR}{}",
            token(self.codec()),
            self.profile_token()
        )
    }

    /// The profile a full text form names: [`Self::token`] read back.
    ///
    /// Case, spaces, `.`, `-` and `_` are ignored on a named profile, so
    /// `h264/high 10` names [`Self::H264High10`]. A profile of a codec whose
    /// tag is `other:` is named by eight hexadecimal digits after the `/`;
    /// anything else is `None`.
    #[must_use]
    pub fn from_token(text: &str) -> Option<Self> {
        let (codec_token, profile_token) = text.trim().split_once(TOKEN_SEPARATOR)?;
        let codec = CodecId::from_name(codec_token)?;
        Self::from_parts(codec, profile_token)
    }

    /// The profile `profile_token` names for `codec`, without the codec half of
    /// the text form.
    pub(crate) fn from_parts(codec: CodecId, profile_token: &str) -> Option<Self> {
        if matches!(codec, CodecId::Other(_)) {
            let digits = profile_token.len() == OTHER_TAG_DIGITS
                && profile_token.bytes().all(|byte| byte.is_ascii_hexdigit());
            if !digits {
                return None;
            }
            let tag = u32::from_str_radix(profile_token, OTHER_TAG_RADIX).ok()?;
            return Some(Self::Other { codec, tag });
        }
        named_in(codec)
            .iter()
            .copied()
            .find(|profile| same_token(profile_token, profile.name()))
    }
}

/// The named profiles of `codec`, in the order they are listed above.
fn named_in(codec: CodecId) -> &'static [CodecProfile] {
    match codec {
        CodecId::H264 => H264_PROFILES,
        CodecId::Hevc => HEVC_PROFILES,
        CodecId::Vvc => VVC_PROFILES,
        CodecId::Av1 => AV1_PROFILES,
        CodecId::Vp9 => VP9_PROFILES,
        CodecId::Mpeg2Video => MPEG2_PROFILES,
        CodecId::Aac => AAC_PROFILES,
        _ => NO_PROFILES,
    }
}

broadcast_core::impl_spec_display!(CodecProfile);
