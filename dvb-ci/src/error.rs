//! Error type for EN 50221 Common Interface parsing/serialization.

/// Result alias for `dvb-ci`.
pub type Result<T> = core::result::Result<T, Error>;

/// A Common Interface parse or serialize error.
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
    /// Output buffer smaller than `serialized_len()`.
    #[error("output buffer too small: need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// An object's 3-byte `apdu_tag` did not match the expected value.
    #[error("unexpected apdu_tag: got {got:#08X}, expected {expected:#08X} ({what})")]
    UnexpectedApduTag {
        /// The tag read from the wire (24-bit, in the low 3 bytes).
        got: u32,
        /// The tag the parser expected.
        expected: u32,
        /// The object being parsed.
        what: &'static str,
    },
    /// A 1-byte `spdu_tag` did not match the expected value.
    #[error("unexpected spdu_tag: got {got:#04X}, expected {expected:#04X} ({what})")]
    UnexpectedSpduTag {
        /// The tag read from the wire.
        got: u8,
        /// The tag the parser expected.
        expected: u8,
        /// The object being parsed.
        what: &'static str,
    },
    /// A 1-byte `tpdu_tag` did not match the expected value.
    #[error("unexpected tpdu_tag: got {got:#04X}, expected {expected:#04X} ({what})")]
    UnexpectedTpduTag {
        /// The tag read from the wire.
        got: u8,
        /// The tag the parser expected.
        expected: u8,
        /// The object being parsed.
        what: &'static str,
    },
    /// The ASN.1-style `length_field` was malformed (truncated, or a
    /// declared length running past the buffer).
    #[error("invalid length_field: {0}")]
    InvalidLength(&'static str),
    /// A `length_field` encoded a value too large to represent (the spec caps
    /// any length at 65535, i.e. at most 3 length bytes).
    #[error("length_field value too large to encode: {0}")]
    LengthTooLarge(usize),
    /// A declared `length_field` did not match the actual body length, or the
    /// body did not fit the fixed shape the object requires.
    #[error("length mismatch for {what}: declared {declared}, body has {actual}")]
    LengthMismatch {
        /// What was being parsed.
        what: &'static str,
        /// The `length_field` value.
        declared: usize,
        /// The body length actually available.
        actual: usize,
    },
    /// A fixed-shape object had a body length the spec does not allow.
    #[error("invalid object body for {what}: {reason}")]
    InvalidObject {
        /// The object being parsed.
        what: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A `resource_identifier()` did not map to any CI-extension resource that
    /// [`crate::ci_ext::CiExtApdu`] can dispatch.
    #[error("unknown CI-extension resource: {resource_id:#010X}")]
    UnknownResource {
        /// The 32-bit `resource_identifier()`.
        resource_id: u32,
    },
}
