//! Error types for ST 2022-6/-7 parsing.
//!
//! Field-by-field semantics are documented in the curated spec transcription,
//! `st2022/docs/st2022-6-framing.md` (SMPTE ST 2022-6:2012 §6.4).

use crate::header::ClockFrequency;

/// Result alias for `st2022` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced when parsing ST 2022-6/-7 structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than
    /// required.
    #[error("buffer too short: need {need}, have {have} for {what}")]
    BufferTooShort {
        /// Minimum bytes required.
        need: usize,
        /// Bytes actually available.
        have: usize,
        /// What was being parsed.
        what: &'static str,
    },
    /// A field value did not fit its wire bit-width, or a derived count was
    /// inconsistent (e.g. the header extension length).
    #[error("field {field} value {value} invalid: {reason}")]
    InvalidValue {
        /// The offending field name.
        field: &'static str,
        /// The offending value.
        value: u64,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// `clock_frequency` (`CF`, §6.4) requires the 32-bit Video Timestamp
    /// row to be present exactly when `clock_frequency` !=
    /// [`ClockFrequency::NoTimestamp`], but the header disagreed.
    #[error(
        "clock_frequency {clock_frequency:?} requires video_timestamp to be present={expected_present}, \
         but found present={found_present}"
    )]
    VideoTimestampMismatch {
        /// The `clock_frequency` value that determines whether a timestamp
        /// is required.
        clock_frequency: ClockFrequency,
        /// Whether a video timestamp is required for this `clock_frequency`.
        expected_present: bool,
        /// Whether a video timestamp was actually present.
        found_present: bool,
    },
}
