//! Error type returned by every EBML and Matroska parser in this crate.
//!
//! The shared failure kinds ([`BufferTooShort`](broadcast_core::error::BufferTooShort),
//! [`OutputBufferTooSmall`](broadcast_core::error::OutputBufferTooSmall),
//! [`InvalidValue`](broadcast_core::error::InvalidValue) and
//! [`InvalidField`](broadcast_core::error::InvalidField)) are wrapped
//! transparently; the rest are EBML- and Matroska-specific.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants EBML and Matroska parsers and builders return.
#[derive(Debug, Error, PartialEq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Input ended before a field could be read (RFC 8794 §4).
    #[error("input too short for {what}: need {need} bytes, have {have}")]
    Truncated {
        /// Field being read.
        what: &'static str,
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },

    /// A VINT declared or needs a width outside 1..=8 octets (RFC 8794 §4.3).
    #[error("VINT width {width} is outside 1..=8 octets")]
    VintWidth {
        /// The offending width.
        width: usize,
    },

    /// The first octet of a VINT was zero, which cannot start one (§4.3).
    #[error("VINT starts with a zero octet")]
    VintZeroStart,

    /// An Element ID is invalid: zero or all-one data bits, or not the
    /// shortest encoding (RFC 8794 §5).
    #[error("invalid Element ID 0x{value:X}")]
    InvalidElementId {
        /// The offending raw id.
        value: u32,
    },

    /// An Element ID needed more than the four octets this crate reads.
    #[error("Element ID width {width} exceeds 4 octets")]
    ElementIdWidth {
        /// The offending width.
        width: usize,
    },

    /// An Element Data Size did not fit a `u64`.
    #[error("Element Data Size overflows 64 bits")]
    SizeOverflow,

    /// An element declared a size larger than the bytes available.
    #[error("element 0x{id:X} declares {declared} bytes, only {available} available")]
    ElementOverruns {
        /// Element ID whose size overran its parent.
        id: u32,
        /// Declared payload size.
        declared: u64,
        /// Bytes left in the parent.
        available: usize,
    },

    /// An element's declared size did not match its encoded payload.
    #[error("element 0x{id:X} declares {declared} bytes but encodes {encoded}")]
    SizeMismatch {
        /// Element ID.
        id: u32,
        /// Declared payload size.
        declared: u64,
        /// Encoded payload size.
        encoded: u64,
    },

    /// An unknown-sized element was not a master element, or not one of the
    /// two Matroska masters that may use an unknown size (§6.2).
    #[error("element 0x{id:X} may not use an unknown size")]
    UnknownSizeNotAllowed {
        /// Element ID.
        id: u32,
    },

    /// An integer payload was wider than eight octets.
    #[error("integer payload width {width} is outside 1..=8 octets")]
    IntegerWidth {
        /// Payload width in octets.
        width: usize,
    },

    /// A float payload was neither four nor eight octets (RFC 8794 §8).
    #[error("float payload width {width} is neither 4 nor 8 octets")]
    FloatWidth {
        /// Payload width in octets.
        width: usize,
    },

    /// A string or UTF-8 payload was not valid UTF-8 (RFC 8794 §8).
    #[error("text payload is not valid UTF-8")]
    InvalidUtf8,

    /// A Block header was malformed (RFC 9559 §12).
    #[error("invalid Block: {what}")]
    InvalidBlock {
        /// What was wrong.
        what: &'static str,
    },

    /// A hostile-input cap was exceeded.
    #[error("limit {limit} exceeded: {value}")]
    LimitExceeded {
        /// Name of the cap.
        limit: &'static str,
        /// The offending value.
        value: u64,
    },

    /// The reader was asked to supply bytes with no outstanding demand.
    #[error("supply without an outstanding need")]
    SupplyWithoutNeed,

    /// The supplied range did not start where the reader asked.
    #[error("supplied offset {got} does not match the requested offset {expected}")]
    SupplyMismatch {
        /// Offset the reader requested.
        expected: u64,
        /// Offset the caller supplied.
        got: u64,
    },

    /// A forward-only source was asked to move backwards.
    #[error("forward-only source cannot seek back from {position} to {offset}")]
    BackwardSeek {
        /// Target offset.
        offset: u64,
        /// Furthest position already requested.
        position: u64,
    },

    /// A seek was requested before the Segment header was read.
    #[error("seek before the Segment header")]
    SeekBeforeSegment,

    /// No Cues entry and no Cluster header names the requested track/time.
    #[error("no index entry for track {track}")]
    NoIndex {
        /// Track number the seek named.
        track: u64,
    },

    /// A parser required a particular element id and found another.
    #[error("expected element 0x{expected:X}, found 0x{found:X}")]
    UnexpectedElement {
        /// Required id.
        expected: u32,
        /// Found id.
        found: u32,
    },

    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_core::error::BufferTooShort),

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_core::error::OutputBufferTooSmall),

    /// A field had an invalid or reserved value.
    #[error(transparent)]
    InvalidValue(#[from] broadcast_core::error::InvalidValue),

    /// A field was missing or structurally invalid.
    #[error(transparent)]
    InvalidField(#[from] broadcast_core::error::InvalidField),
}
