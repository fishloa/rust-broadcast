//! Error type for DVB SimulCrypt (ETSI TS 103 197) message framing.

/// Result alias for SimulCrypt parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A SimulCrypt parse / serialize error.
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
    /// A value did not fit in its wire width when serializing (e.g. a TLV value
    /// longer than the 16-bit `parameter_length`, or a body longer than the
    /// 16-bit `message_length`).
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: usize,
        /// The field width on the wire.
        bits: u32,
    },
}
