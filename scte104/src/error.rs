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
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

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

    /// A reserved field carried a non-reserved value.
    #[error(transparent)]
    ReservedBits(#[from] broadcast_common::error::ReservedBits),

    /// A field carried a value its spec clause forbids (the offending value
    /// is carried).
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),

    /// A field is structurally invalid with no single offending number.
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),

    /// Output buffer for `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// A length or count field's value does not fit its wire field
    /// (see `broadcast_common::len`, #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),
}
