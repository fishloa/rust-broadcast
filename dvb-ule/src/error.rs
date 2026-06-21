//! Error type for ULE (RFC 4326 / RFC 5163) parsing and serialization.

/// Result alias for ULE parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A ULE parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error("buffer too short: need {need}, have {have} ({what})")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
        /// What was being parsed.
        what: &'static str,
    },
    /// The output buffer passed to `serialize_into` was too small.
    #[error("output buffer too small: need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// The SNDU `Length` field is inconsistent with the available bytes.
    ///
    /// `Length` counts from the byte after the Type field up to and including
    /// the CRC (RFC 4326 §4.2).
    #[error("invalid SNDU length {length}: {reason}")]
    InvalidLength {
        /// The `Length` field value.
        length: u16,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// The 32-bit CRC trailer did not match the recomputed value (RFC 4326 §4.6).
    #[error("SNDU CRC mismatch: computed {computed:#010X}, found {found:#010X}")]
    CrcMismatch {
        /// CRC recomputed over the SNDU.
        computed: u32,
        /// CRC read from the trailer.
        found: u32,
    },
    /// A field value did not fit in its wire bit-width.
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u32,
        /// The field width on the wire.
        bits: u32,
    },
    /// An extension header was malformed (bad H-LEN/H-Type or truncated body).
    #[error("invalid extension header: {reason}")]
    InvalidExtensionHeader {
        /// Why the extension header is invalid.
        reason: &'static str,
    },
    /// A TS packet payload was the wrong size or carried an invalid Payload
    /// Pointer (RFC 4326 §6/§7).
    #[error("TS mapping error: {reason}")]
    TsMapping {
        /// Why the TS payload could not be processed.
        reason: &'static str,
    },
}
