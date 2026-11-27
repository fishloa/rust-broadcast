//! Codec identity and the parameters a decoder needs before its first packet.

use alloc::string::String;
use alloc::vec::Vec;
use broadcast_core::timescale::Timescale;
use core::num::NonZeroU32;

use crate::audio::ChannelLayout;
use crate::padding_error::PaddingError;
use crate::profile::CodecProfile;
use crate::profile_error::ProfileError;

/// The token that introduces an [`CodecId::Other`] tag in the text form.
pub(crate) const OTHER_TOKEN: &str = "other";
/// Separates [`OTHER_TOKEN`] from the tag's digits.
const OTHER_SEPARATOR: char = ':';
/// An `other:` tag is written as this many hexadecimal digits.
pub(crate) const OTHER_TAG_DIGITS: usize = 8;
/// Base of those digits.
pub(crate) const OTHER_TAG_RADIX: u32 = 16;

/// A coded format.
///
/// [`CodecId::Other`] carries a caller-chosen tag for a format this enum does
/// not name; the seam never interprets it. Its text form is `other:` followed
/// by the tag as eight lower-case hexadecimal digits (`other:00000007`), which
/// both [`CodecId::from_name`] and a preference accept. The tag should be the
/// format's registered four-character code where it has one; two providers
/// that give the same tag to different formats must avoid that collision
/// themselves.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecId {
    /// ITU-T H.264 / ISO/IEC 14496-10 advanced video coding.
    H264,
    /// ITU-T H.265 / ISO/IEC 23008-2 high efficiency video coding.
    Hevc,
    /// ITU-T H.266 / ISO/IEC 23090-3 versatile video coding.
    Vvc,
    /// AOMedia AV1.
    Av1,
    /// Google VP9.
    Vp9,
    /// ISO/IEC 13818-2 MPEG-2 video.
    Mpeg2Video,
    /// ISO/IEC 14496-3 AAC.
    Aac,
    /// Dolby AC-3.
    Ac3,
    /// Dolby E-AC-3.
    Eac3,
    /// Dolby AC-4.
    Ac4,
    /// DTS.
    Dts,
    /// IETF RFC 6716 Opus.
    Opus,
    /// Xiph FLAC.
    Flac,
    /// ISO/IEC 11172-3 / 13818-3 MPEG audio (layers I to III).
    MpegAudio,
    /// ISO/IEC 23008-3 MPEG-H 3D audio.
    MpegH3dAudio,
    /// Linear pulse-code modulation.
    Pcm,
    /// A format this enum does not name.
    Other(u32),
}

impl CodecId {
    /// Label of the coded format.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::Vvc => "VVC",
            Self::Av1 => "AV1",
            Self::Vp9 => "VP9",
            Self::Mpeg2Video => "MPEG-2 video",
            Self::Aac => "AAC",
            Self::Ac3 => "AC-3",
            Self::Eac3 => "E-AC-3",
            Self::Ac4 => "AC-4",
            Self::Dts => "DTS",
            Self::Opus => "Opus",
            Self::Flac => "FLAC",
            Self::MpegAudio => "MPEG audio",
            Self::MpegH3dAudio => "MPEG-H 3D audio",
            Self::Pcm => "PCM",
            Self::Other(_) => "other",
        }
    }

    /// Whether a stream of this codec needs a configuration record before its
    /// first access unit.
    ///
    /// A bijective codec does (H.264, AAC, Opus, …); MPEG-2 video, MPEG audio,
    /// PCM and an unnamed format do not, so an encoder that reports no
    /// extradata for one of those is stating the whole truth rather than
    /// writing the record in-band.
    #[must_use]
    pub const fn requires_configuration_record(&self) -> bool {
        matches!(
            self,
            Self::H264
                | Self::Hevc
                | Self::Vvc
                | Self::Av1
                | Self::Vp9
                | Self::Aac
                | Self::Ac3
                | Self::Eac3
                | Self::Ac4
                | Self::Dts
                | Self::Opus
                | Self::Flac
                | Self::MpegH3dAudio
        )
    }

    /// The codec a token names: [`Self::name`] read back.
    ///
    /// Case, spaces, `.`, `-` and `_` are ignored, so `h264`, `H.264` and
    /// `H264` all name [`Self::H264`]. [`Self::Other`] is named by
    /// `other:<8 hex digits>`; `other` alone names no codec and this returns
    /// `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some((token, digits)) = name.trim().split_once(OTHER_SEPARATOR)
            && same_token(token, OTHER_TOKEN)
        {
            return other_from_digits(digits);
        }
        if same_token(name, OTHER_TOKEN) {
            return None;
        }
        [
            Self::H264,
            Self::Hevc,
            Self::Vvc,
            Self::Av1,
            Self::Vp9,
            Self::Mpeg2Video,
            Self::Aac,
            Self::Ac3,
            Self::Eac3,
            Self::Ac4,
            Self::Dts,
            Self::Opus,
            Self::Flac,
            Self::MpegAudio,
            Self::MpegH3dAudio,
            Self::Pcm,
        ]
        .into_iter()
        .find(|codec| same_token(name, codec.name()))
    }
}

/// The codec an [`CodecId::Other`] tag's digits name, when they are exactly
/// eight hexadecimal digits.
fn other_from_digits(digits: &str) -> Option<CodecId> {
    if digits.len() != OTHER_TAG_DIGITS || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, OTHER_TAG_RADIX)
        .ok()
        .map(CodecId::Other)
}

/// `label` lower-cased with spaces and punctuation removed: the compact token
/// the codec and profile text forms are built from.
pub(crate) fn compact(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The compact text form a preference and the inventory use: [`CodecId::name`]
/// lower-cased with punctuation removed, or `other:<8 hex digits>`.
pub(crate) fn token(codec: CodecId) -> String {
    match codec {
        CodecId::Other(tag) => alloc::format!("{OTHER_TOKEN}{OTHER_SEPARATOR}{tag:08x}"),
        named => compact(named.name()),
    }
}

/// Whether `input` names `label` once case and punctuation are ignored.
pub(crate) fn same_token(input: &str, label: &str) -> bool {
    compact(input) == compact(label)
}

broadcast_core::impl_spec_display!(CodecId, Other);

/// The audio half of a codec's parameters.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioParameters {
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
}

impl AudioParameters {
    /// Audio parameters from a non-zero sample rate and a channel layout.
    #[must_use]
    pub const fn new(sample_rate: NonZeroU32, channels: ChannelLayout) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }

    /// Samples per second.
    #[must_use]
    pub const fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    /// Channel layout.
    #[must_use]
    pub const fn channels(&self) -> ChannelLayout {
        self.channels
    }
}

/// The video half of a codec's parameters.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoParameters {
    coded_width: NonZeroU32,
    coded_height: NonZeroU32,
}

impl VideoParameters {
    /// Video parameters from the coded (pre-crop) dimensions.
    #[must_use]
    pub const fn new(coded_width: NonZeroU32, coded_height: NonZeroU32) -> Self {
        Self {
            coded_width,
            coded_height,
        }
    }

    /// Coded width in pixels.
    #[must_use]
    pub const fn coded_width(&self) -> NonZeroU32 {
        self.coded_width
    }

    /// Coded height in pixels.
    #[must_use]
    pub const fn coded_height(&self) -> NonZeroU32 {
        self.coded_height
    }
}

/// Where the configuration record that describes a stream lives.
///
/// A bijective codec needs one before its first coded access unit: an
/// `avcC`/`hvcC`/`vvcC` decoder configuration record, an AAC
/// `AudioSpecificConfig`, a `dOps`, a `dac3` and so on. The seam carries it
/// either as [`CodecParameters::extradata`] or as an explicit statement that
/// the encoder emits it in-band, which is what lets a caller tell an encoder
/// that wrote the record into its first access unit from one that merely had
/// none to report.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConfigurationRecord {
    /// Nothing states a record: the stream describes itself, or the container
    /// carries none for this codec.
    #[default]
    Absent,
    /// [`CodecParameters::extradata`] carries it.
    OutOfBand,
    /// The encoder writes it in-band, in its first access unit.
    InBand,
}

impl ConfigurationRecord {
    /// Label of this form.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::OutOfBand => "out-of-band",
            Self::InBand => "in-band",
        }
    }
}

broadcast_core::impl_spec_display!(ConfigurationRecord);

/// What a decoder needs before the first packet, or what an encoder reports
/// once it knows its output.
///
/// [`extradata`](Self::extradata) is the out-of-band configuration in the form
/// the codec's own container record carries it; the seam treats it as opaque
/// bytes and [`configuration_record`](Self::configuration_record) says where
/// it comes from. Per [`CodecId`]:
///
/// | codec | record |
/// |---|---|
/// | [`CodecId::H264`] | an `avcC` `AVCDecoderConfigurationRecord` body, the record `avcC` carries (ISO/IEC 14496-15:2017 §5.3.3.1) |
/// | [`CodecId::Hevc`] | an `hvcC` `HEVCDecoderConfigurationRecord` body (ISO/IEC 14496-15:2017 §8.3.3.1) |
/// | [`CodecId::Vvc`] | a `vvcC` `VvcDecoderConfigurationRecord` body (ISO/IEC 14496-15:2017 §10.3) |
/// | [`CodecId::Av1`] / [`CodecId::Vp9`] | an `av1C` / `vpcC` box body |
/// | [`CodecId::Aac`] | the `AudioSpecificConfig` payload, not including an `esds` header (ISO/IEC 14496-3 §1.6.2.1) |
/// | [`CodecId::Ac3`]/[`CodecId::Eac3`]/[`CodecId::Ac4`] | a `dac3`/`dec3`/`dac4` box body |
/// | [`CodecId::Opus`] | a `dOps` box body (RFC 7845 §5.1) |
/// | [`CodecId::Flac`] | an `fLaC`/`dfLa` body |
/// | [`CodecId::Dts`] | a `ddts` box body |
/// | [`CodecId::MpegH3dAudio`] | an `mhaC` box body |
/// | [`CodecId::Mpeg2Video`] / [`CodecId::MpegAudio`] | an `esds` body, FullBox header and `ES_Descriptor` included |
/// | [`CodecId::Pcm`] / [`CodecId::Other`] | none stated; the seam defines no record |
///
/// An encoder that writes the record in-band says
/// [`ConfigurationRecord::InBand`] instead of reporting empty bytes, so a caller
/// can derive it from the first access unit.
///
/// `profile` is the codec variant the stream is in, when the caller knows it;
/// the registry never offers the stream to an implementation that states it
/// cannot handle that profile.
///
/// `initial_padding` and `trailing_padding` are the container's gapless trim:
/// samples per channel a lossy encoder added before the first real sample and
/// after the last. `initial_padding` is 0 when there is none or the container
/// does not say; `trailing_padding` is `None` when the container does not say,
/// which is distinct from `Some(0)`. Both are audio-only.
///
/// `pre_roll` is the samples a decoder must consume after a seek or a splice
/// before its output is correct. It is not a trim and is carried for both
/// audio and video; `None` means the container states none.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecParameters {
    codec: CodecId,
    profile: Option<CodecProfile>,
    extradata: Vec<u8>,
    configuration: ConfigurationRecord,
    time_base: Timescale,
    audio: Option<AudioParameters>,
    video: Option<VideoParameters>,
    initial_padding: u32,
    trailing_padding: Option<u32>,
    pre_roll: Option<u32>,
}

impl CodecParameters {
    /// Parameters for `codec` in `time_base`, with no extradata and no stream
    /// kind yet.
    #[must_use]
    pub const fn new(codec: CodecId, time_base: Timescale) -> Self {
        Self {
            codec,
            profile: None,
            extradata: Vec::new(),
            configuration: ConfigurationRecord::Absent,
            time_base,
            audio: None,
            video: None,
            initial_padding: 0,
            trailing_padding: None,
            pre_roll: None,
        }
    }

    /// State the codec profile the stream is in.
    ///
    /// # Errors
    /// [`ProfileError::Mismatch`] when the profile belongs to another codec.
    pub fn with_profile(mut self, profile: CodecProfile) -> Result<Self, ProfileError> {
        if profile.codec() != self.codec {
            return Err(ProfileError::mismatch(profile, self.codec));
        }
        self.profile = Some(profile);
        Ok(self)
    }

    /// Replace the opaque out-of-band configuration and state that it is
    /// carried out of band.
    #[must_use]
    pub fn with_extradata(mut self, extradata: Vec<u8>) -> Self {
        self.extradata = extradata;
        self.configuration = ConfigurationRecord::OutOfBand;
        self
    }

    /// State that the encoder writes the configuration record in-band, in its
    /// first access unit, so [`extradata`](Self::extradata) is empty and the
    /// caller derives it from that access unit.
    #[must_use]
    pub const fn with_in_band_configuration(mut self) -> Self {
        self.configuration = ConfigurationRecord::InBand;
        self
    }

    /// State that no configuration record is carried: the stream describes
    /// itself, or its codec needs none.
    #[must_use]
    pub const fn with_no_configuration_record(mut self) -> Self {
        self.configuration = ConfigurationRecord::Absent;
        self
    }

    /// Attach the audio half.
    #[must_use]
    pub const fn with_audio(mut self, audio: AudioParameters) -> Self {
        self.audio = Some(audio);
        self
    }

    /// Attach the video half.
    #[must_use]
    pub const fn with_video(mut self, video: VideoParameters) -> Self {
        self.video = Some(video);
        self
    }

    /// State that a decoder must consume `samples` samples per channel after a
    /// seek or a splice before its output is correct.
    ///
    /// This is a decoder requirement, not a trim: a [`TrimStage`](crate::TrimStage) never
    /// discards it as padding.
    #[must_use]
    pub const fn with_pre_roll(mut self, samples: u32) -> Self {
        self.pre_roll = Some(samples);
        self
    }

    /// Discard `samples` per channel from the start of the decoded stream.
    ///
    /// # Errors
    /// [`PaddingError::Video`] when these parameters describe video.
    pub fn with_initial_padding(mut self, samples: u32) -> Result<Self, PaddingError> {
        if self.video.is_some() {
            return Err(PaddingError::Video);
        }
        self.initial_padding = samples;
        Ok(self)
    }

    /// Discard `samples` per channel from the end of the decoded stream.
    ///
    /// # Errors
    /// [`PaddingError::Video`] when these parameters describe video.
    pub fn with_trailing_padding(mut self, samples: u32) -> Result<Self, PaddingError> {
        if self.video.is_some() {
            return Err(PaddingError::Video);
        }
        self.trailing_padding = Some(samples);
        Ok(self)
    }

    /// Coded format.
    #[must_use]
    pub const fn codec(&self) -> CodecId {
        self.codec
    }

    /// The codec profile the stream is in, when it is known.
    #[must_use]
    pub const fn profile(&self) -> Option<CodecProfile> {
        self.profile
    }

    /// Opaque out-of-band configuration, empty unless
    /// [`configuration_record`](Self::configuration_record) is
    /// [`ConfigurationRecord::OutOfBand`].
    #[must_use]
    pub fn extradata(&self) -> &[u8] {
        &self.extradata
    }

    /// Where the configuration record comes from.
    #[must_use]
    pub const fn configuration_record(&self) -> ConfigurationRecord {
        self.configuration
    }

    /// Tick rate of every `pts`/`dts` the seam carries for this stream.
    #[must_use]
    pub const fn time_base(&self) -> Timescale {
        self.time_base
    }

    /// Audio parameters, when this is an audio stream.
    #[must_use]
    pub const fn audio(&self) -> Option<AudioParameters> {
        self.audio
    }

    /// Video parameters, when this is a video stream.
    #[must_use]
    pub const fn video(&self) -> Option<VideoParameters> {
        self.video
    }

    /// Samples per channel to discard from the start of the decoded stream.
    #[must_use]
    pub const fn initial_padding(&self) -> u32 {
        self.initial_padding
    }

    /// Samples per channel to discard from the end of the decoded stream, when
    /// the container says.
    #[must_use]
    pub const fn trailing_padding(&self) -> Option<u32> {
        self.trailing_padding
    }

    /// Samples per channel a decoder must consume after a seek or a splice
    /// before its output is correct, when the container states a pre-roll
    /// requirement.
    #[must_use]
    pub const fn pre_roll(&self) -> Option<u32> {
        self.pre_roll
    }
}
