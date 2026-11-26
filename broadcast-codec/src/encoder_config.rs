//! The settings every encoder needs, plus the provider's own.

use alloc::string::String;
use alloc::vec::Vec;
use core::num::{NonZeroU32, NonZeroU64};

use broadcast_core::timescale::Timescale;

use crate::codec::{AudioParameters, CodecId, VideoParameters};
use crate::profile::CodecProfile;
use crate::profile_error::ProfileError;

/// The settings an encoder is built with.
///
/// The typed settings are the ones every encoder needs: the coded format, the
/// tick rate its output timestamps use, the profile to produce, the source
/// parameters it is asked to produce, an optional target bit rate and an
/// optional picture group length.
///
/// [`options`](Self::options) carries everything that is provider-specific, in
/// the order it was added. A provider that does not recognise a key MUST return
/// [`CodecError::Refused`](crate::CodecError) naming that key, never ignore it:
/// silently dropping a setting encodes something other than what the caller
/// asked for.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderConfig {
    codec: CodecId,
    time_base: Timescale,
    profile: Option<CodecProfile>,
    audio: Option<AudioParameters>,
    video: Option<VideoParameters>,
    bit_rate: Option<NonZeroU64>,
    gop: Option<NonZeroU32>,
    options: Vec<(String, String)>,
}

impl EncoderConfig {
    /// Settings for `codec` in `time_base`, with no source parameters, no rate
    /// target and no provider options.
    #[must_use]
    pub const fn new(codec: CodecId, time_base: Timescale) -> Self {
        Self {
            codec,
            time_base,
            profile: None,
            audio: None,
            video: None,
            bit_rate: None,
            gop: None,
            options: Vec::new(),
        }
    }

    /// Ask the encoder to produce `profile`.
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

    /// Attach the audio half of the source parameters.
    #[must_use]
    pub const fn with_audio(mut self, audio: AudioParameters) -> Self {
        self.audio = Some(audio);
        self
    }

    /// Attach the video half of the source parameters.
    #[must_use]
    pub const fn with_video(mut self, video: VideoParameters) -> Self {
        self.video = Some(video);
        self
    }

    /// Ask for a target bit rate in bits per second.
    #[must_use]
    pub const fn with_bit_rate(mut self, bit_rate: NonZeroU64) -> Self {
        self.bit_rate = Some(bit_rate);
        self
    }

    /// Ask for a picture group length in frames.
    #[must_use]
    pub const fn with_gop(mut self, gop: NonZeroU32) -> Self {
        self.gop = Some(gop);
        self
    }

    /// Add one provider-specific setting. Keys are matched exactly and
    /// case-sensitively; adding the same key twice keeps both, in order.
    #[must_use]
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.push((key.into(), value.into()));
        self
    }

    /// Coded format to produce.
    #[must_use]
    pub const fn codec(&self) -> CodecId {
        self.codec
    }

    /// Tick rate of the output timestamps.
    #[must_use]
    pub const fn time_base(&self) -> Timescale {
        self.time_base
    }

    /// The codec profile to produce, when one was asked for.
    #[must_use]
    pub const fn profile(&self) -> Option<CodecProfile> {
        self.profile
    }

    /// Audio source parameters, when this is an audio encoder.
    #[must_use]
    pub const fn audio(&self) -> Option<AudioParameters> {
        self.audio
    }

    /// Video source parameters, when this is a video encoder.
    #[must_use]
    pub const fn video(&self) -> Option<VideoParameters> {
        self.video
    }

    /// Target bit rate in bits per second, when one was asked for.
    #[must_use]
    pub const fn bit_rate(&self) -> Option<NonZeroU64> {
        self.bit_rate
    }

    /// Picture group length in frames, when one was asked for.
    #[must_use]
    pub const fn gop(&self) -> Option<NonZeroU32> {
        self.gop
    }

    /// Provider-specific settings, in the order they were added.
    #[must_use]
    pub fn options(&self) -> &[(String, String)] {
        &self.options
    }
}
