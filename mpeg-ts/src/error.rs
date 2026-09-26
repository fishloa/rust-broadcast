//! Error type returned by every parser and builder in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ITU-T H.222.0 (= ISO/IEC 13818-1) where applicable.
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

    /// CRC-32 validation failed for a table section.
    #[error("CRC-32 mismatch: computed {computed:#010x}, expected {expected:#010x}")]
    CrcMismatch {
        /// CRC we calculated over the section bytes.
        computed: u32,
        /// CRC carried at the end of the section.
        expected: u32,
    },

    /// TS sync byte was not the expected `0x47`.
    #[error("invalid TS sync byte: expected 0x47, got {found:#04x}")]
    InvalidSyncByte {
        /// The byte actually read at position 0.
        found: u8,
    },

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },

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

    /// A length or count field's value does not fit its wire field
    /// (see `broadcast_common::len`, #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),

    /// `Section::payload` did not match what `section_length` implies (see
    /// `Section::serialize_into`, #1074 W5). Serializing a hand-built
    /// `Section` whose fields disagree used to index past `serialized_len()`
    /// and panic (or write the CRC over payload bytes) instead of erroring.
    #[error(
        "section payload is {actual} bytes but section_length ({section_length}) implies {expected}"
    )]
    SectionPayloadLengthMismatch {
        /// `section_length` field the caller set.
        section_length: u16,
        /// Payload length `section_length` implies (accounting for the
        /// extension header + CRC on long-form sections).
        expected: usize,
        /// Actual `payload.len()`.
        actual: usize,
    },
}
