//! Error type for DVB SimulCrypt (ETSI TS 103 197) message framing.

/// Result alias for SimulCrypt parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A SimulCrypt parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// The output buffer passed to `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
    /// The `message_length` header field is inconsistent with the bytes that
    /// follow it (TS 103 197 Table 1b: it counts the bytes immediately after
    /// the `message_length` field — i.e. the sum of all parameter TLVs).
    #[error("invalid message_length {length}: {reason}")]
    InvalidMessageLength {
        /// The `message_length` field value.
        length: u16,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A parameter TLV was truncated: its `parameter_length` ran past the end
    /// of the message body.
    #[error("truncated parameter (type {ptype:#06X}): need {need}, have {have}")]
    TruncatedParameter {
        /// The `parameter_type` of the offending TLV.
        ptype: u16,
        /// Bytes the `parameter_length` claimed.
        need: usize,
        /// Bytes actually remaining in the message body.
        have: usize,
    },
    /// A value did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),
}
