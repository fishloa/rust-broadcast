//! Error type for caption/subtitle conversion.

use crate::matrix::{SourceFormat, Support, TargetFormat};
use alloc::string::String;

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// A caption/subtitle conversion error.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input was empty.
    #[error("empty input")]
    EmptyInput,
    /// The input did not parse as a well-formed WebVTT document.
    #[error("invalid WebVTT: {0}")]
    InvalidWebVtt(String),
    /// The input did not parse as a well-formed SRT document.
    #[error("invalid SRT: {0}")]
    InvalidSrt(String),
    /// A cue timing field was not a valid `(hh:)mm:ss.ttt` / `(hh:)mm:ss,ttt`
    /// timestamp.
    #[error("invalid timestamp {0:?}")]
    InvalidTimestamp(String),
    /// A `cc_data()` byte string failed to parse (feature `cc-data`).
    #[cfg(feature = "cc-data")]
    #[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
    #[error("cc_data() parse error: {0}")]
    CcData(#[from] cc_data::Error),
    /// A Teletext data-field wire payload failed to parse (feature
    /// `teletext`).
    #[cfg(feature = "teletext")]
    #[cfg_attr(docsrs, doc(cfg(feature = "teletext")))]
    #[error("teletext data field parse error: {0}")]
    DvbVbi(#[from] dvb_vbi::Error),
    /// The requested `from -> to` conversion is not implemented losslessly
    /// or lossily by this crate. See [`crate::matrix::MATRIX`] (and the
    /// crate root docs) for the full conversion matrix and the reason for
    /// every unsupported/not-yet-implemented pair.
    #[error("{from} -> {to} is {support}: {reason}")]
    Unsupported {
        /// The requested source format.
        from: SourceFormat,
        /// The requested target format.
        to: TargetFormat,
        /// Why this pair is not `Lossless`/`Lossy` (i.e. is `Unsupported` or
        /// `NotImplemented`).
        support: Support,
        /// A human-readable explanation, copied from the matrix entry.
        reason: &'static str,
    },
}
