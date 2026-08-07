//! Error type for RIST RTCP message parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `rist-runtime/docs/tr-06-1-simple-profile.md` (VSF TR-06-1:2020).

/// Result alias for `rist-runtime` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// A RIST RTCP message parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than required.
    #[error("buffer too short: need {need}, have {have}")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
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

    /// The RTCP version field was not 2 (RFC 3550 §6.4.1).
    #[error("invalid version: expected 2, got {0}")]
    InvalidVersion(u8),

    /// The RIST APP name field was not `0x52495354` ("RIST").
    #[error("invalid RIST APP name: expected 0x52495354, got 0x{0:08X}")]
    InvalidAppName(u32),

    /// The APP subtype field did not match an expected value.
    #[error("invalid subtype: {0}")]
    InvalidSubtype(u8),

    /// The FMT field in an RTPFB packet did not match the expected value.
    #[error("invalid FMT: expected {expected}, got {got}")]
    InvalidFmt {
        /// The expected FMT value.
        expected: u8,
        /// The actual FMT value.
        got: u8,
    },

    /// The RTCP packet type did not match the expected value.
    #[error("invalid packet type: expected {expected}, got {got}")]
    InvalidPacketType {
        /// The expected PT value.
        expected: u8,
        /// The actual PT value.
        got: u8,
    },

    /// A Range NACK contained more than 16 range entries per the spec limit.
    #[error("too many range requests: max {max}, got {got}")]
    TooManyRanges {
        /// Maximum allowed.
        max: usize,
        /// Actual count.
        got: usize,
    },

    /// The padding field was not a multiple of 4 bytes.
    #[error("padding length {0} is not a multiple of 4")]
    InvalidPaddingLength(usize),

    /// An error from the underlying `rtcp-packet` crate.
    #[error(transparent)]
    Rtcp(#[from] rtcp_packet::Error),
}
