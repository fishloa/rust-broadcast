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

    /// An individual-addressing `function_length` is below 2. It counts the whole
    /// `function()` including its own `function_tag` and `function_length`
    /// bytes (ETSI TS 102 773 §5.2.8.2), so no valid value is less than 2.
    #[error(
        "invalid function_length {function_length}: it includes the 2-byte function_tag + function_length header, so must be at least 2"
    )]
    InvalidFunctionLength {
        /// The `function_length` byte actually read.
        function_length: u8,
    },

    /// An individual-addressing `FunctionEntry` whose `tag` does not agree with
    /// its `body` (a typed body under the wrong tag, or a `Raw` body under a
    /// tag that parses back as a typed body), so serialize -> parse would not
    /// be the identity.
    #[error("function tag {tag:#04x} does not match its body ({body}): {reason}")]
    FunctionTagBodyMismatch {
        /// The entry's `tag` byte.
        tag: u8,
        /// Label of the body variant it was paired with.
        body: &'static str,
        /// Why the pairing cannot round-trip.
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

    /// CRC-32 validation failed.
    #[error("CRC-32 mismatch: computed {computed:#010x}, expected {expected:#010x}")]
    CrcMismatch {
        /// CRC we calculated over the payload bytes.
        computed: u32,
        /// CRC carried at the end of the packet.
        expected: u32,
    },

    /// A bit-field error from [`broadcast_common::bits::BitReader`] / [`broadcast_common::bits::BitWriter`].
    #[error("L1 bit-field error: {0}")]
    L1Bits(#[from] broadcast_common::bits::BitError),
}
