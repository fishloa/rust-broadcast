//! Error type for the multicast object-delivery wire formats
//! (RFC 5651 LCT / RFC 5775 ALC / RFC 6726 FLUTE / RFC 5740 NORM).

/// Result alias for this crate's parsing / serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// A parse / serialize error.
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
    /// A field value did not fit in its wire bit-width.
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u64,
        /// The field width on the wire.
        bits: u32,
    },
    /// A reserved / version field carried an unexpected value.
    #[error("invalid field {what}: {reason}")]
    InvalidField {
        /// The field name.
        what: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A header-extension was malformed (bad HET/HEL or truncated content).
    #[error("invalid header extension: {reason}")]
    InvalidExtension {
        /// Why the extension is invalid.
        reason: &'static str,
    },
    /// A length field (`HDR_LEN` / `hdr_len`) was inconsistent with the bytes
    /// the parser computed from the flags / fixed fields.
    #[error("inconsistent length {length}: {reason}")]
    InconsistentLength {
        /// The length field value (in 32-bit words).
        length: u8,
        /// Why it is inconsistent.
        reason: &'static str,
    },
}
