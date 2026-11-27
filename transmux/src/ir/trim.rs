//! Gapless trim in the IR: a track's priming and end padding, and a sample's
//! own trim.
//!
//! A lossy audio encoder adds samples that are not part of the programme, and
//! a container says how many to discard. [`GaplessTrim`] is the track-level
//! statement, in samples per channel at the track's sample rate;
//! [`SampleTrim`] is the per-packet statement a container that signals it
//! packet by packet makes. Both default to "none stated".

/// A track's gapless trim, in samples per channel at the track's sample rate.
///
/// `initial` is the priming lead-in before the first real sample and
/// `trailing` is the end padding; `None` means the container states no trailing
/// padding, which is distinct from `Some(0)`. Audio-only: a video track states
/// [`GaplessTrim::NONE`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GaplessTrim {
    initial: u32,
    trailing: Option<u32>,
}

impl GaplessTrim {
    /// No trim stated.
    pub const NONE: Self = Self {
        initial: 0,
        trailing: None,
    };

    /// A trim of `initial` samples before the programme and, when the container
    /// says, `trailing` after it.
    #[must_use]
    pub const fn new(initial: u32, trailing: Option<u32>) -> Self {
        Self { initial, trailing }
    }

    /// Samples per channel to discard from the start.
    #[must_use]
    pub const fn initial(&self) -> u32 {
        self.initial
    }

    /// Samples per channel to discard from the end, when the container says.
    #[must_use]
    pub const fn trailing(&self) -> Option<u32> {
        self.trailing
    }

    /// Whether the track states no trim at all.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        self.initial == 0 && self.trailing.is_none()
    }
}

/// One sample's gapless trim, in samples per channel at the track's sample
/// rate.
///
/// This is the per-packet statement a container makes (Matroska's
/// `DiscardPadding`, for example); a container that states the trim once for
/// the track leaves it at [`SampleTrim::NONE`] and uses
/// [`GaplessTrim`](super::GaplessTrim) instead.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SampleTrim {
    start: u32,
    end: u32,
}

impl SampleTrim {
    /// No per-sample trim stated.
    pub const NONE: Self = Self { start: 0, end: 0 };

    /// Discard `start` samples from the front of this sample's decode and
    /// `end` from its back.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Samples per channel to discard from the start.
    #[must_use]
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// Samples per channel to discard from the end.
    #[must_use]
    pub const fn end(&self) -> u32 {
        self.end
    }

    /// Whether this sample states no trim.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        self.start == 0 && self.end == 0
    }
}

/// The samples a decoder must decode after a seek before its output is
/// correct: a container's pre-roll requirement, not a gapless trim.
///
/// ISO/IEC 14496-12:2026 §10.1 states it as a `prol` or negative `roll`
/// sample-group distance; Matroska states it as `SeekPreRoll` (RFC 9559
/// §5.1.4.1.26).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderPreRoll {
    samples: core::num::NonZeroU32,
}

impl DecoderPreRoll {
    /// A requirement of `samples` samples per channel.
    #[must_use]
    pub const fn new(samples: core::num::NonZeroU32) -> Self {
        Self { samples }
    }

    /// A requirement of `samples` samples per channel, or `None` for zero
    /// (which states no requirement).
    #[must_use]
    pub fn from_samples(samples: u32) -> Option<Self> {
        core::num::NonZeroU32::new(samples).map(Self::new)
    }

    /// Samples per channel that must be decoded before output is correct.
    #[must_use]
    pub const fn samples(&self) -> u32 {
        self.samples.get()
    }
}

/// Which container signal stated a gapless trim.
///
/// The precedence order is [`GaplessSignal::EditList`], then
/// [`GaplessSignal::CodecPreSkip`], then [`GaplessSignal::ITunesSmpb`]:
/// the edit list is the normative statement, the codec's own configuration is
/// next, and the iTunes item is a de-facto fallback.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaplessSignal {
    /// The MP4 `elst` media time (ISO/IEC 14496-12:2026 §8.6.6).
    EditList,
    /// A codec configuration's own priming (Opus `dOps` `PreSkip`,
    /// RFC 7845 §5.1).
    CodecPreSkip,
    /// The iTunes `iTunSMPB` metadata item.
    ITunesSmpb,
}

impl GaplessSignal {
    /// The signal's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::EditList => "edit list",
            Self::CodecPreSkip => "codec pre-skip",
            Self::ITunesSmpb => "iTunSMPB",
        }
    }
}

broadcast_core::impl_spec_display!(GaplessSignal);

/// Two container signals stated different gapless trims.
///
/// The demuxer keeps the higher-precedence signal (whose trim is the track's
/// [`GaplessTrim`]) and names the lower one here; nothing is silently
/// resolved.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaplessTrimFinding {
    /// The signal whose trim was used.
    pub used: GaplessSignal,
    /// The signal whose different trim was ignored.
    pub ignored: GaplessSignal,
}

impl GaplessTrimFinding {
    /// A finding that `used` won over `ignored`.
    #[must_use]
    pub const fn new(used: GaplessSignal, ignored: GaplessSignal) -> Self {
        Self { used, ignored }
    }
}
