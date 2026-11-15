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
    InvalidWebVtt(WebVttError),
    /// The input did not parse as a well-formed SRT document.
    #[error("invalid SRT: {0}")]
    InvalidSrt(SrtError),
    /// A cue timing field was not a valid `(hh:)mm:ss.ttt` / `(hh:)mm:ss,ttt`
    /// timestamp.
    #[error("invalid timestamp {timestamp:?}")]
    InvalidTimestamp {
        /// The offending timestamp text.
        timestamp: String,
    },
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

/// Why a document is not a well-formed WebVTT document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WebVttError {
    /// The document does not open with the `WEBVTT` signature.
    #[error("missing WEBVTT signature on line 1")]
    MissingSignature,
    /// A cue identifier line has no timing line after it.
    #[error("cue identifier with no timing line")]
    IdentifierWithoutTiming,
    /// A cue's timing line is malformed.
    #[error("malformed timing line: {line}")]
    MalformedTiming {
        /// The offending timing line.
        line: String,
    },
    /// A cue's end time precedes its start time.
    #[error("cue end time is before its start time")]
    EndBeforeStart,
}

/// Why a document is not a well-formed SRT document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SrtError {
    /// A blank block (impossible: leading/trailing blank lines are trimmed,
    /// so this names an internal invariant breach).
    #[error("empty block")]
    EmptyBlock,
    /// The first line of a block is neither a timing line nor a sequence
    /// number.
    #[error("expected a sequence number, got {line:?}")]
    BadSequenceNumber {
        /// The offending first line.
        line: String,
    },
    /// A sequence-number line has no timing line after it.
    #[error("sequence number with no timing line")]
    SequenceWithoutTiming,
    /// A timing line has no `-->` separator.
    #[error("no '-->' in timing line: {line:?}")]
    NoArrow {
        /// The offending timing line.
        line: String,
    },
    /// A cue's end time precedes its start time.
    #[error("cue end time is before its start time")]
    EndBeforeStart,
}
