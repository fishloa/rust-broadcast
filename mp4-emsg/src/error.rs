//! Error type for `emsg` (MPEG-DASH Event Message Box) parsing and serialization.

/// Result alias for `emsg` parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// An `emsg` parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// The output buffer passed to `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
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
    #[error(transparent)]
    UnsupportedVersion(#[from] broadcast_common::error::UnsupportedVersion),
    /// A null-terminated UTF-8 string field was malformed (no terminator, or
    /// invalid UTF-8).
    #[error("invalid {field} string: {reason}")]
    InvalidString {
        /// Which string field (`scheme_id_uri` / `value`).
        field: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A value did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),
}
