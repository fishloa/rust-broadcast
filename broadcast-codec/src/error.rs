//! The one failure type a decoder or encoder reports.

use core::fmt;

/// Why a decoder or encoder refused an operation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecError {
    /// The codec's input queue is full; receive before sending more.
    Full,
    /// This input is not acceptable. The codec is still usable.
    Refused {
        /// What was refused.
        what: &'static str,
    },
    /// The codec is unusable until it is reset.
    Failed {
        /// What failed.
        what: &'static str,
    },
    /// The codec does not support this input or operation.
    Unsupported {
        /// What is unsupported.
        what: &'static str,
    },
}

impl CodecError {
    /// A [`Self::Refused`] for `what`.
    #[must_use]
    pub const fn refused(what: &'static str) -> Self {
        Self::Refused { what }
    }

    /// A [`Self::Failed`] for `what`.
    #[must_use]
    pub const fn failed(what: &'static str) -> Self {
        Self::Failed { what }
    }

    /// An [`Self::Unsupported`] for `what`.
    #[must_use]
    pub const fn unsupported(what: &'static str) -> Self {
        Self::Unsupported { what }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("codec input queue is full"),
            Self::Refused { what } => write!(f, "codec refused {what}"),
            Self::Failed { what } => write!(f, "codec failed on {what}"),
            Self::Unsupported { what } => write!(f, "codec does not support {what}"),
        }
    }
}

impl core::error::Error for CodecError {}
