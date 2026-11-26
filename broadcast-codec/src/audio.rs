//! Decoded audio: channel layout, sample layout, sample buffers and one frame.

use alloc::vec::Vec;
use core::num::{NonZeroU16, NonZeroU32};

/// Channels in a mono layout.
const MONO_CHANNELS: u16 = 1;
/// Channels in a stereo layout.
const STEREO_CHANNELS: u16 = 2;
/// Channels in a 5.1 layout (front left/right, centre, LFE, surround left/right).
const SURROUND_5_1_CHANNELS: u16 = 6;
/// Channels in a 7.1 layout (5.1 plus rear left/right).
const SURROUND_7_1_CHANNELS: u16 = 8;

/// The channels a frame carries.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    /// One channel.
    Mono,
    /// Front left and right.
    Stereo,
    /// Six channels: front left/right, centre, LFE, surround left/right.
    Surround5_1,
    /// Eight channels: 5.1 plus rear left/right.
    Surround7_1,
    /// A caller-chosen channel count with no named order.
    Discrete(NonZeroU16),
}

impl ChannelLayout {
    /// Number of channels.
    #[must_use]
    pub const fn count(&self) -> usize {
        let count = match self {
            Self::Mono => MONO_CHANNELS,
            Self::Stereo => STEREO_CHANNELS,
            Self::Surround5_1 => SURROUND_5_1_CHANNELS,
            Self::Surround7_1 => SURROUND_7_1_CHANNELS,
            Self::Discrete(count) => count.get(),
        };
        count as usize
    }

    /// Label of the layout.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::Surround5_1 => "5.1",
            Self::Surround7_1 => "7.1",
            Self::Discrete(_) => "discrete",
        }
    }
}

broadcast_core::impl_spec_display!(ChannelLayout, Discrete);

/// How the samples of an [`AudioFrame`] are laid out.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleLayout {
    /// Sample for sample across channels: `L R L R …`.
    Interleaved,
    /// Channel 0's samples, then channel 1's, and so on.
    Planar,
}

impl SampleLayout {
    /// Label of the layout.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Interleaved => "interleaved",
            Self::Planar => "planar",
        }
    }
}

broadcast_core::impl_spec_display!(SampleLayout);

/// Decoded samples in their native storage width.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum AudioSamples {
    /// Unsigned 8-bit samples.
    U8(Vec<u8>),
    /// Signed 16-bit samples.
    S16(Vec<i16>),
    /// Signed 32-bit samples.
    S32(Vec<i32>),
    /// 32-bit floating point samples.
    F32(Vec<f32>),
    /// 64-bit floating point samples.
    F64(Vec<f64>),
}

impl AudioSamples {
    /// Total number of samples across every channel.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::U8(samples) => samples.len(),
            Self::S16(samples) => samples.len(),
            Self::S32(samples) => samples.len(),
            Self::F32(samples) => samples.len(),
            Self::F64(samples) => samples.len(),
        }
    }

    /// Whether the buffer holds no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One decoded audio frame.
///
/// The checked constructor refuses a sample count that is not a whole number of
/// frames for the given channel count, so [`Self::frames`] never truncates.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrame {
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
    layout: SampleLayout,
    samples: AudioSamples,
    pts: Option<i64>,
}

impl AudioFrame {
    /// A frame from its rate, layout and samples.
    ///
    /// Returns `None` when the sample count is not divisible by the channel
    /// count; the timestamp starts absent and is set by [`Self::with_pts`].
    #[must_use]
    pub fn new(
        sample_rate: NonZeroU32,
        channels: ChannelLayout,
        layout: SampleLayout,
        samples: AudioSamples,
    ) -> Option<Self> {
        let channel_count = channels.count();
        if !samples.len().is_multiple_of(channel_count) {
            return None;
        }
        Some(Self {
            sample_rate,
            channels,
            layout,
            samples,
            pts: None,
        })
    }

    /// Attach the presentation timestamp in the stream's time base.
    #[must_use]
    pub const fn with_pts(mut self, pts: i64) -> Self {
        self.pts = Some(pts);
        self
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

    /// How the samples are laid out.
    #[must_use]
    pub const fn layout(&self) -> SampleLayout {
        self.layout
    }

    /// The sample buffer.
    #[must_use]
    pub const fn samples(&self) -> &AudioSamples {
        &self.samples
    }

    /// Presentation timestamp in the stream's time base, when known.
    #[must_use]
    pub const fn pts(&self) -> Option<i64> {
        self.pts
    }

    /// Samples per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.count()
    }
}
