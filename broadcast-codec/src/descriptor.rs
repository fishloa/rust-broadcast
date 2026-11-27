//! One implementation a provider offers.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec::CodecId;
use crate::delay::DecoderTrim;
use crate::direction::Direction;
use crate::profile::CodecProfile;
use crate::profile_error::ProfileError;
use crate::sample_format::SampleFormat;
use crate::video::PixelFormat;

/// One implementation a [`CodecProvider`](crate::CodecProvider) offers.
///
/// `provider` is the offering provider's name and `implementation` is the
/// provider's own name for this implementation (for example
/// `h264_videotoolbox`); the two are equal when a provider offers exactly one.
/// The format lists are what the implementation accepts for
/// [`Direction::Encode`] or produces for [`Direction::Decode`]; an empty list
/// means the provider does not state them. `profiles` names the codec variants
/// this implementation handles, as [`CodecProfile`] keys the caller and the
/// registry share; an empty list means "not stated", not "all", so the
/// registry keeps such an implementation for every profile. A stated list is
/// binding: the registry never offers an implementation a stream of a profile
/// the implementation does not list. `trim` says who applies the container's
/// gapless trim and what the implementation's own delay is: the caller through
/// [`Registry::trimmed_decoder`](crate::Registry::trimmed_decoder), or the
/// implementation itself.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodecDescriptor {
    provider: String,
    implementation: String,
    codec: CodecId,
    direction: Direction,
    hardware: bool,
    lossless: bool,
    trim: DecoderTrim,
    sample_formats: Vec<SampleFormat>,
    pixel_formats: Vec<PixelFormat>,
    profiles: Vec<CodecProfile>,
}

impl CodecDescriptor {
    /// A software, lossy implementation of `codec` in `direction`.
    #[must_use]
    pub fn new(
        provider: impl Into<String>,
        implementation: impl Into<String>,
        codec: CodecId,
        direction: Direction,
    ) -> Self {
        Self {
            provider: provider.into(),
            implementation: implementation.into(),
            codec,
            direction,
            hardware: false,
            lossless: false,
            trim: DecoderTrim::Caller(crate::delay::DecoderDelay::Unknown),
            sample_formats: Vec::new(),
            pixel_formats: Vec::new(),
            profiles: Vec::new(),
        }
    }

    /// Record whether the implementation runs on dedicated hardware.
    #[must_use]
    pub const fn with_hardware(mut self, hardware: bool) -> Self {
        self.hardware = hardware;
        self
    }

    /// Record whether the implementation is lossless.
    #[must_use]
    pub const fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = lossless;
        self
    }

    /// State who applies the container's gapless trim and, when the caller
    /// does, the implementation's own delay.
    #[must_use]
    pub const fn with_trim(mut self, trim: DecoderTrim) -> Self {
        self.trim = trim;
        self
    }

    /// State the sample formats the implementation accepts or produces.
    #[must_use]
    pub fn with_sample_formats(mut self, sample_formats: Vec<SampleFormat>) -> Self {
        self.sample_formats = sample_formats;
        self
    }

    /// State the pixel formats the implementation accepts or produces.
    #[must_use]
    pub fn with_pixel_formats(mut self, pixel_formats: Vec<PixelFormat>) -> Self {
        self.pixel_formats = pixel_formats;
        self
    }

    /// State the codec variants the implementation handles.
    ///
    /// # Errors
    /// [`ProfileError::Mismatch`] when a profile belongs to another codec.
    pub fn with_profiles(
        mut self,
        profiles: impl IntoIterator<Item = CodecProfile>,
    ) -> Result<Self, ProfileError> {
        let mut stated = Vec::new();
        for profile in profiles {
            if profile.codec() != self.codec {
                return Err(ProfileError::mismatch(profile, self.codec));
            }
            stated.push(profile);
        }
        self.profiles = stated;
        Ok(self)
    }

    /// Name of the provider that offers this implementation.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The provider's own name for this implementation.
    #[must_use]
    pub fn implementation(&self) -> &str {
        &self.implementation
    }

    /// Coded format the implementation handles.
    #[must_use]
    pub const fn codec(&self) -> CodecId {
        self.codec
    }

    /// Which way the implementation converts.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Whether the implementation runs on dedicated hardware.
    #[must_use]
    pub const fn hardware(&self) -> bool {
        self.hardware
    }

    /// Whether the implementation is lossless.
    #[must_use]
    pub const fn lossless(&self) -> bool {
        self.lossless
    }

    /// Who applies the container's gapless trim, and the implementation's own
    /// delay when the caller does.
    #[must_use]
    pub const fn trim(&self) -> DecoderTrim {
        self.trim
    }

    /// Sample formats the implementation accepts or produces, in the order the
    /// provider prefers them.
    #[must_use]
    pub fn sample_formats(&self) -> &[SampleFormat] {
        &self.sample_formats
    }

    /// Pixel formats the implementation accepts or produces, in the order the
    /// provider prefers them.
    #[must_use]
    pub fn pixel_formats(&self) -> &[PixelFormat] {
        &self.pixel_formats
    }

    /// Codec variants the implementation handles; empty means "not stated".
    #[must_use]
    pub fn profiles(&self) -> &[CodecProfile] {
        &self.profiles
    }
}
