//! The one failure type a decoder or encoder reports.

use alloc::borrow::Cow;
use core::fmt;

/// Why a decoder or encoder refused an operation.
///
/// The `what` of each message-bearing variant accepts a `&'static str` or an
/// owned `String`: an implementation names a fixed thing, while the registry
/// names a provider, implementation or codec chosen at run time.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CodecError {
    /// The codec's input queue is full; receive before sending more.
    Full,
    /// This input is not acceptable. The codec is still usable.
    Refused {
        /// What was refused.
        what: Cow<'static, str>,
    },
    /// The codec is unusable until it is reset.
    Failed {
        /// What failed.
        what: Cow<'static, str>,
    },
    /// The codec does not support this input or operation.
    Unsupported {
        /// What is unsupported.
        what: Cow<'static, str>,
    },
    /// The decoder cannot adopt new parameters; build a fresh one.
    CannotReconfigure {
        /// What it cannot reconfigure.
        what: Cow<'static, str>,
    },
}

impl CodecError {
    /// A [`Self::Refused`] for `what`.
    #[must_use]
    pub fn refused(what: impl Into<Cow<'static, str>>) -> Self {
        Self::Refused { what: what.into() }
    }

    /// A [`Self::Failed`] for `what`.
    #[must_use]
    pub fn failed(what: impl Into<Cow<'static, str>>) -> Self {
        Self::Failed { what: what.into() }
    }

    /// An [`Self::Unsupported`] for `what`.
    #[must_use]
    pub fn unsupported(what: impl Into<Cow<'static, str>>) -> Self {
        Self::Unsupported { what: what.into() }
    }

    /// A [`Self::CannotReconfigure`] for `what`.
    #[must_use]
    pub fn cannot_reconfigure(what: impl Into<Cow<'static, str>>) -> Self {
        Self::CannotReconfigure { what: what.into() }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("codec input queue is full"),
            Self::Refused { what } => write!(f, "codec refused {what}"),
            Self::Failed { what } => write!(f, "codec failed on {what}"),
            Self::Unsupported { what } => write!(f, "codec does not support {what}"),
            Self::CannotReconfigure { what } => write!(f, "codec cannot reconfigure {what}"),
        }
    }
}

impl core::error::Error for CodecError {}
