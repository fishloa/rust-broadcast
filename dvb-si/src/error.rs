//! Error type returned by every parser in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ETSI EN 300 468 v1.19.1 where applicable.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// CRC-32 validation failed for a table section.
    #[error(transparent)]
    CrcMismatch(#[from] broadcast_common::error::CrcMismatch),

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
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// A fixed marker did not match (TS sync byte, expected `0x47`).
    #[error(transparent)]
    InvalidSync(#[from] broadcast_common::error::InvalidSync),

    /// A serializer's length or count value did not fit its wire field.
    /// Returned instead of silently narrowing (`as u8`/`as u16`/masking),
    /// which would emit a misframed section (see #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),
}

impl From<mpeg_ts::Error> for Error {
    fn from(e: mpeg_ts::Error) -> Self {
        match e {
            mpeg_ts::Error::BufferTooShort(e) => Error::BufferTooShort(e),
            mpeg_ts::Error::CrcMismatch(e) => Error::CrcMismatch(e),
            mpeg_ts::Error::InvalidSync(e) => Error::InvalidSync(e),
            mpeg_ts::Error::OutputBufferTooSmall(e) => Error::OutputBufferTooSmall(e),
            mpeg_ts::Error::SectionLengthOverflow {
                declared,
                available,
            } => Error::SectionLengthOverflow {
                declared,
                available,
            },
            // mpeg_ts::Error is #[non_exhaustive]; catch future variants generically.
            _ => Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need: 0,
                have: 0,
                what: "mpeg_ts (unknown error)",
            }),
        }
    }
}
