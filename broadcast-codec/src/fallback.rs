//! What a registry does when the first candidate cannot serve.

/// What a [`Registry`](crate::Registry) does when a candidate cannot serve.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fallback {
    /// Try only the first candidate and return its error.
    Strict,
    /// Skip a candidate that answers
    /// [`Unsupported`](crate::CodecError::Unsupported) or
    /// [`Refused`](crate::CodecError::Refused) and try the next one;
    /// [`Failed`](crate::CodecError::Failed) still stops at once.
    NextCandidate,
}

impl Fallback {
    /// Label of the fallback mode.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::NextCandidate => "next candidate",
        }
    }
}

broadcast_core::impl_spec_display!(Fallback);
