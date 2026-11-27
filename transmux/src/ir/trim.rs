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
