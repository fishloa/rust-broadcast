//! Error types for ST 2022-6/-7 parsing.
//!
//! Field-by-field semantics are documented in the curated spec transcription,
//! `private/specs-verified/smpte/st2022/st2022-6-framing.md` (SMPTE ST 2022-6:2012 §6.4).

use crate::header::ClockFrequency;

/// Result alias for `st2022` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced when parsing ST 2022-6/-7 structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer shorter than
    /// required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// Output buffer smaller than the serialized form requires.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// A field value did not fit its wire bit-width, or a derived count was
    /// inconsistent (e.g. the header extension length).
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),
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
