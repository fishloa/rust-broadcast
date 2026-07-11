//! Error type for RTCP packet parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `rtcp-packet/docs/rtcp.md` (RFC 3550 §6).

/// Result alias for `rtcp-packet` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An RTCP packet parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than required.
    #[error("buffer too short: need {need}, have {have} ({what})")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
        /// What was being parsed/serialized.
        what: &'static str,
    },
    /// Output buffer passed to `serialize_into` was smaller than
    /// `serialized_len()`.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// A field value did not fit its wire bit-width, or the two's-complement
    /// version field was not `2` (RFC 3550 §6.4.1: "The version defined by
    /// this specification is two (2)"), or a derived count (report/source
    /// count, item/reason length) overflowed its field, or SDES/BYE text was
    /// not valid UTF-8 (RFC 3550 §6.5: "encoded according to the UTF-8
    /// encoding").
    #[error("field {field} value {value} invalid: {reason}")]
    InvalidValue {
        /// The offending field/derived-count name.
        field: &'static str,
        /// The offending value.
        value: u64,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A caller-supplied argument violated a documented precondition (e.g. an
    /// empty [`CompoundPacket`](crate::CompoundPacket) or one not starting
    /// with SR/RR per RFC 3550 §6.1).
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
}
