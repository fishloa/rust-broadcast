//! Error type returned by every parser in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ETSI TS 102 773 v1.4.1 where applicable.
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

    /// `packet_type` byte is not in Table 1 (0x00..=0x02, 0x10..=0x12, 0x20..=0x21, 0x30..=0x33).
    #[error("invalid T2-MI packet_type {found:#04x} — reserved per ETSI TS 102 773 Table 1")]
    InvalidPacketType {
        /// Byte value actually read.
        found: u8,
    },

    /// A reserved bit was not in the expected state.
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

    /// Payload length declared more bits than remaining bytes.
    #[error(
        "payload length mismatch: {declared_bits} bits declared, {remaining_bytes} bytes remaining"
    )]
    PayloadLengthMismatch {
        /// Declared payload_len_bits from header.
        declared_bits: u16,
        /// Actual remaining bytes.
        remaining_bytes: usize,
    },

    /// Input buffer was shorter than required for CRC validation.
    #[error("buffer too short for CRC validation")]
    Truncated,

    /// CRC-32 value did not match expected checksum.
    #[error("CRC-32 mismatch: expected {expected:#010x}, computed {computed:#010x}")]
    InvalidCrc {
        /// Expected CRC from trailer.
        expected: u32,
        /// Computed CRC over payload.
        computed: u32,
    },
}
