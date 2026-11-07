//! Error type for RTCP packet parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `private/specs-verified/rtp/rtcp-packet/rtcp.md` (RFC 3550 §6).

/// Result alias for `rtcp-packet` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// A padded RTCP packet whose padding count (RFC 3550 §6.4.1) is zero or
/// larger than the packet body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid padding count {count}: body is {body} byte(s)")]
pub struct PaddingCountError {
    /// The padding-count byte (last byte of the body).
    pub count: usize,
    /// Bytes in the packet body (everything after the common header).
    pub body: usize,
}

/// An RTCP packet parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// Output buffer passed to `serialize_into` was smaller than
    /// `serialized_len()`.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
    /// A field value did not fit its wire bit-width, or the two's-complement
    /// version field was not `2` (RFC 3550 §6.4.1: "The version defined by
    /// this specification is two (2)"), or a derived count (report/source
    /// count, item/reason length) overflowed its field, or SDES/BYE text was
    /// not valid UTF-8 (RFC 3550 §6.5: "encoded according to the UTF-8
    /// encoding").
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),
    /// A caller-supplied argument violated a documented precondition (e.g. an
    /// empty [`CompoundPacket`](crate::CompoundPacket) or one not starting
    /// with SR/RR per RFC 3550 §6.1).
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
}
