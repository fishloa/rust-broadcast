//! Error type returned by every parser and builder in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ITU-T H.222.0 (= ISO/IEC 13818-1) where applicable.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_core::error::BufferTooShort),

    /// CRC-32 validation failed for a table section.
    #[error(transparent)]
    CrcMismatch(#[from] broadcast_core::error::CrcMismatch),

    /// A fixed marker did not match (TS sync byte, expected `0x47`).
    #[error(transparent)]
    InvalidSync(#[from] broadcast_core::error::InvalidSync),

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_core::error::OutputBufferTooSmall),

    /// A `section_length` declared more bytes than the containing buffer could hold.
    #[error("section_length {declared} exceeds remaining buffer ({available} bytes)")]
    SectionLengthOverflow {
        /// Length bytes declared inside the section header.
        declared: usize,
        /// Bytes actually available after the header.
        available: usize,
    },

    /// A payload was longer than a single TS packet's fixed payload capacity
    /// (see `OwnedTsPacket::serialize_with_payload`, #1129).
    #[error("payload {len} bytes exceeds the {max}-byte TS packet payload capacity")]
    PayloadTooLarge {
        /// The payload length that did not fit.
        len: usize,
        /// The maximum payload capacity.
        max: usize,
    },

    /// A PCR or ESCR value did not fit the 27 MHz clock: a base wider than
    /// 33 bits, or an extension `>= 300` (the 9-bit wire field can carry
    /// `300..=511`).
    #[error(transparent)]
    ClockOutOfRange(#[from] broadcast_common::clock33::ClockOutOfRange),

    /// A length or count field's value does not fit its wire field
    /// (see `broadcast_core::len`, #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_core::len::FieldOverflow),

    /// The packet has no adaptation field, so it has no PCR (and no OPCR)
    /// slot to rewrite.
    #[error("packet has no adaptation field")]
    NoAdaptationField,

    /// The adaptation field has no PCR (or OPCR) slot: either it is shorter
    /// than the flags byte plus the 6-byte field, or the packet's own flag
    /// byte does not set the corresponding flag.
    #[error("packet has no {which} slot")]
    NoClockReference {
        /// The missing field: `"PCR"` or `"OPCR"`.
        which: &'static str,
    },

    /// The adaptation field is shorter than the field being written into it.
    #[error("adaptation field is {have} bytes, {need} needed")]
    AdaptationFieldTooShort {
        /// Bytes the write needs.
        need: usize,
        /// `adaptation_field_length` the packet declares.
        have: usize,
    },
}
