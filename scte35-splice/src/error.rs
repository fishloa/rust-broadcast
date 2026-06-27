//! Error type returned by every parser + builder in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that SCTE 35 parsers and builders can return.
///
/// Spec references quote clauses from ANSI/SCTE 35 2023r1 where applicable.
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

    /// CRC-32 validation failed for a splice_info_section (§9.6.1).
    #[error("CRC-32 mismatch: computed {computed:#010x}, expected {expected:#010x}")]
    CrcMismatch {
        /// CRC we calculated over the section bytes.
        computed: u32,
        /// CRC carried at the end of the section.
        expected: u32,
    },

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

    /// `splice_command_type` byte did not match the value the invoked command
    /// parser expected (§9.6.1, Table 7).
    #[error("unexpected splice_command_type {got:#04x} for {what} (expected {expected:#04x})")]
    UnexpectedCommandType {
        /// Command type byte actually read.
        got: u8,
        /// Command parser invoked.
        what: &'static str,
        /// Command type the parser expected.
        expected: u8,
    },

    /// A field carried a value the structure cannot represent or that violates
    /// a spec constraint (e.g. a 33-bit field given a value ≥ 2^33).
    #[error("invalid value for {field}: {reason}")]
    InvalidValue {
        /// Field being validated.
        field: &'static str,
        /// Why the value is rejected.
        reason: &'static str,
    },

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
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },
}
