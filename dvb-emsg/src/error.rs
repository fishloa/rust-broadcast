//! Error type for `emsg` (MPEG-DASH Event Message Box) parsing and serialization.

/// Result alias for `emsg` parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// An `emsg` parse / serialize error.
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
    /// The box type at bytes `[4:8]` was not `emsg`.
    #[error("not an emsg box: type {found:?}")]
    NotEmsg {
        /// The 4-byte box type actually found.
        found: [u8; 4],
    },
    /// The `size` field is inconsistent with the available bytes (it must be at
    /// least the fixed header + cover the box, and 0 / 1 large-size forms are
    /// not supported for `emsg`).
    #[error("invalid emsg box size {size}: {reason}")]
    InvalidSize {
        /// The `size` field value.
        size: u32,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// The `FullBox` `version` was neither 0 nor 1.
    #[error("unsupported emsg version {version} (expected 0 or 1)")]
    UnsupportedVersion {
        /// The version byte read from the wire.
        version: u8,
    },
    /// A null-terminated UTF-8 string field was malformed (no terminator, or
    /// invalid UTF-8).
    #[error("invalid {field} string: {reason}")]
    InvalidString {
        /// Which string field (`scheme_id_uri` / `value`).
        field: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A field value did not fit in its wire bit-width (e.g. a box larger than
    /// `u32::MAX`).
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u64,
        /// The field width on the wire.
        bits: u32,
    },
}
