//! Error types for the scte104 crate.
//!
//! One structured `thiserror` enum used by every parser and serializer,
//! mirroring the convention established by `scte35-splice`.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by SCTE 104 parsers and builders.
///
/// ANSI/SCTE 104 2023 §8, §§14.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error("buffer too short: need {need} bytes, have {have} (while parsing {what})")]
    BufferTooShort {
        /// Bytes required to proceed.
        need: usize,
        /// Bytes actually available.
        have: usize,
        /// Human-readable name of the type or field being parsed.
        what: &'static str,
    },

    /// Declared length exceeds the remaining bytes available in the buffer.
    #[error("length {declared} exceeds remaining buffer ({available} bytes) for {what}")]
    LengthOverflow {
        /// Length declared inside the wire header.
        declared: usize,
        /// Bytes actually available.
        available: usize,
        /// What the length describes.
        what: &'static str,
    },

    /// A `op_id` byte did not match the value the invoked operation parser
    /// expected (Table 8-3, Table 8-4).
    #[error("unexpected op_id {got:#06x} for {what} (expected {expected:#06x})")]
    UnexpectedOpId {
        /// op_id actually read.
        got: u16,
        /// Operation parser invoked.
        what: &'static str,
        /// op_id the parser expected.
        expected: u16,
    },

    /// A reserved field carried a non-reserved value.
    #[error("reserved field {field} must be {expected:#x}, got {got:#x}")]
    ReservedSet {
        /// Field name.
        field: &'static str,
        /// Expected value.
        expected: u16,
        /// Actual value.
        got: u16,
    },

    /// A field carried an out-of-range value.
    #[error("invalid value for {field}: {reason}")]
    InvalidValue {
        /// Field being validated.
        field: &'static str,
        /// Why the value is rejected.
        reason: &'static str,
    },

    /// Output buffer for `serialize_into` was too small.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },
}
