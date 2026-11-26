//! Codec identity and the parameters a decoder needs before its first packet.

use alloc::vec::Vec;
use broadcast_core::timescale::Timescale;
use core::num::NonZeroU32;

use crate::audio::ChannelLayout;

/// A coded format.
///
/// [`CodecId::Other`] carries an opaque caller-chosen tag for a format this
/// enum does not name; the seam never interprets it.
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

    /// The codec a token names: [`Self::name`] read back.
    ///
    /// Case, spaces, `.`, `-` and `_` are ignored, so `h264`, `H.264` and
    /// `H264` all name [`Self::H264`]. [`Self::Other`] carries a caller-chosen
    /// tag that has no textual form, so a token that normalises to `other`
    /// names no codec and this returns `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if same_token(name, "other") {
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

/// Whether `input` names `label` once case and punctuation are ignored.
fn same_token(input: &str, label: &str) -> bool {
    fn token(text: &str) -> impl Iterator<Item = char> + '_ {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
    }
    token(input).eq(token(label))
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

/// What a decoder needs before the first packet, or what an encoder reports
/// once it knows its output.
///
/// `extradata` is the out-of-band configuration in the form the codec's own
/// container record carries it (for example an `avcC` or `AudioSpecificConfig`
/// payload); the seam treats it as opaque bytes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecParameters {
    codec: CodecId,
    extradata: Vec<u8>,
    time_base: Timescale,
    audio: Option<AudioParameters>,
    video: Option<VideoParameters>,
}

impl CodecParameters {
    /// Parameters for `codec` in `time_base`, with no extradata and no stream
    /// kind yet.
    #[must_use]
    pub const fn new(codec: CodecId, time_base: Timescale) -> Self {
        Self {
            codec,
            extradata: Vec::new(),
            time_base,
            audio: None,
            video: None,
        }
    }

    /// Replace the opaque out-of-band configuration.
    #[must_use]
    pub fn with_extradata(mut self, extradata: Vec<u8>) -> Self {
        self.extradata = extradata;
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

    /// Coded format.
    #[must_use]
    pub const fn codec(&self) -> CodecId {
        self.codec
    }

    /// Opaque out-of-band configuration.
    #[must_use]
    pub fn extradata(&self) -> &[u8] {
        &self.extradata
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
}
