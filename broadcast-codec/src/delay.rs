//! What a decoder says about its own output delay, and who applies the
//! container's gapless trim.
//!
//! A lossy audio decoder emits samples before the first real one while its
//! internal state settles. That delay is the implementation's, not the
//! container's: the container states the encoder's priming, the implementation
//! states its own warm-up. [`DecoderDelay`] is that statement, and
//! [`DecoderTrim`] joins it to the one decision it interacts with, who applies
//! the container's trim.

/// The samples a decoder implementation emits before the first real one.
///
/// The value is per channel, in the stream's sample rate. It is a statement
/// about the implementation, never about the container: a container's priming
/// lives in [`CodecParameters::initial_padding`](crate::CodecParameters::initial_padding).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecoderDelay {
    /// The implementation emits this many samples before the first real one.
    /// [`DecoderDelay::Declared`] holding zero means its output starts at the
    /// first real sample: it compensates its own delay.
    Declared(u32),
    /// The delay depends on the stream; the decoder instance reports it
    /// through [`Decoder::output_delay`](crate::Decoder::output_delay) once it
    /// knows.
    PerStream,
    /// The implementation does not state its delay.
    Unknown,
}

impl DecoderDelay {
    /// Label of this statement.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Declared(_) => "declared",
            Self::PerStream => "per stream",
            Self::Unknown => "unknown",
        }
    }

    /// The declared delay in samples, when the implementation states a number.
    #[must_use]
    pub const fn samples(&self) -> Option<u32> {
        match self {
            Self::Declared(samples) => Some(*samples),
            Self::PerStream | Self::Unknown => None,
        }
    }
}

broadcast_core::impl_spec_display!(DecoderDelay, Declared);

/// Who applies the container's gapless trim, and what the implementation's own
/// delay is.
///
/// This is one field because the two statements describe one decision: a
/// descriptor that applies the container trim itself also compensates its own
/// delay, and one that leaves the container trim to the caller still states
/// its delay so the caller can discard it too.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecoderTrim {
    /// The caller applies the container's gapless trim, and discards the
    /// implementation's own delay first.
    Caller(DecoderDelay),
    /// The implementation applies the container's gapless trim itself, so its
    /// output starts at the first real sample.
    Implementation,
}

impl DecoderTrim {
    /// Label of this decision.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Caller(_) => "caller",
            Self::Implementation => "implementation",
        }
    }

    /// The implementation's delay, when the caller is the one trimming.
    #[must_use]
    pub const fn delay(&self) -> DecoderDelay {
        match self {
            Self::Caller(delay) => *delay,
            Self::Implementation => DecoderDelay::Unknown,
        }
    }

    /// Whether the implementation applies the container's trim itself.
    #[must_use]
    pub const fn applies_trim(&self) -> bool {
        matches!(self, Self::Implementation)
    }
}

impl core::fmt::Display for DecoderTrim {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Caller(delay) => match delay {
                DecoderDelay::Declared(samples) => write!(f, "{}({samples})", self.name()),
                other => write!(f, "{} ({})", self.name(), other.name()),
            },
            Self::Implementation => f.write_str(self.name()),
        }
    }
}
