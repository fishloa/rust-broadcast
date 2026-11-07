//! Error type for the multicast object-delivery wire formats
//! (RFC 5651 LCT / RFC 5775 ALC / RFC 6726 FLUTE / RFC 5740 NORM).

/// Result alias for this crate's parsing / serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// A parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// The output buffer passed to `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
    /// A value did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),
    /// A reserved / version field carried an unexpected value.
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),
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
