//! Error type for ULE (RFC 4326 / RFC 5163) parsing and serialization.

/// Result alias for ULE parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A ULE parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// The output buffer passed to `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
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
    #[error(transparent)]
    CrcMismatch(#[from] broadcast_common::error::CrcMismatch),
    /// A value did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),
    /// An extension header was malformed (bad H-LEN/H-Type or truncated body).
    #[error("invalid extension header: {reason}")]
    InvalidExtensionHeader {
        /// Why the extension header is invalid.
        reason: &'static str,
    },
}
