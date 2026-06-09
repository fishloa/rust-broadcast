//! Error type returned by every parser in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ETSI EN 300 468 v1.19.1 where applicable.
#[derive(Debug, Error)]
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

    /// `table_id` byte doesn't match any expected value for the parser invoked.
    #[error("unexpected table_id {table_id:#04x} for {what} (expected one of {expected:?})")]
    UnexpectedTableId {
        /// Byte value actually read.
        table_id: u8,
        /// Table names or parser expecting it.
        what: &'static str,
        /// The permitted set.
        expected: &'static [u8],
    },

    /// Descriptor payload failed semantic validation.
    #[error("invalid descriptor (tag {tag:#04x}): {reason}")]
    InvalidDescriptor {
        /// Descriptor tag being parsed.
        tag: u8,
        /// Specific failure reason.
        reason: &'static str,
    },

    /// BCD-encoded value is out of valid range.
    #[error("invalid BCD in {field}: bytes {bytes:02x?}")]
    InvalidBcd {
        /// Field name where the BCD sits.
        field: &'static str,
        /// The (up to 4) raw bytes inspected.
        bytes: [u8; 4],
    },

    /// A decoded value passed to a `set_*` accessor could not be encoded to the
    /// field's fixed wire representation (e.g. a duration ≥ 100 hours, or a date
    /// outside the 16-bit MJD range).
    #[error("value out of range for {field}: {reason}")]
    ValueOutOfRange {
        /// Field being set.
        field: &'static str,
        /// Why the value is not representable.
        reason: &'static str,
    },

    /// A `section_length` declared more bytes than the containing buffer could hold.
    #[error("section_length {declared} exceeds remaining buffer ({available} bytes)")]
    SectionLengthOverflow {
        /// Length bytes declared inside the section header.
        declared: usize,
        /// Bytes actually available after the header.
        available: usize,
    },

    /// A reserved bit was not in the expected state. Most parsers are permissive
    /// about reserved bits; this variant is only emitted when a spec clause
    /// specifically requires a value.
    #[error("reserved bits violation in {field}: {reason}")]
    ReservedBitsViolation {
        /// Where.
        field: &'static str,
        /// Why.
        reason: &'static str,
    },

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },

    /// TS sync byte was not the expected `0x47`.
    #[error("invalid TS sync byte: expected 0x47, got {found:#04x}")]
    InvalidSyncByte {
        /// The byte actually read at position 0.
        found: u8,
    },
}
