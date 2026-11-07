//! Error type returned by every parser + builder in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that SCTE 35 parsers and builders can return.
///
/// Spec references quote clauses from ANSI/SCTE 35 2023r1 where applicable.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// CRC-32 validation failed for a splice_info_section (§9.6.1).
    #[error(transparent)]
    CrcMismatch(#[from] broadcast_common::error::CrcMismatch),

    /// `table_id` byte was not the expected `0xFC` (§9.6.1).
    #[error("unexpected table_id {table_id:#04x} for splice_info_section (expected 0xFC)")]
    UnexpectedTableId {
        /// Byte value actually read.
        table_id: u8,
    },

    /// A `splice_descriptor_tag` byte did not match the value the invoked
    /// descriptor parser expected (§10.2.1).
    #[error("unexpected splice_descriptor_tag {tag:#04x} for {what} (expected {expected:#04x})")]
    UnexpectedDescriptorTag {
        /// Tag byte actually read.
        tag: u8,
        /// Descriptor parser invoked.
        what: &'static str,
        /// Tag the parser expected.
        expected: u8,
    },

    /// A field carried a value its spec clause forbids (the offending value
    /// is carried).
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),

    /// A field is structurally invalid with no single offending number (a
    /// missing or inconsistent part).
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),

    /// A declared length (section_length, descriptor_length, …) ran past the
    /// bytes available in the buffer.
    #[error("length {declared} exceeds remaining buffer ({available} bytes) for {what}")]
    LengthOverflow {
        /// Length declared inside the wire header.
        declared: usize,
        /// Bytes actually available.
        available: usize,
        /// What the length describes.
        what: &'static str,
    },

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// A length or count field's value does not fit its wire field
    /// (see `broadcast_common::len`, #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),
}
