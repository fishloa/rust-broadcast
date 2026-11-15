//! Error type for the RTMP ingest session engine.
//!
//! Structured [`thiserror`] errors covering the failure surface of the
//! sans-IO engine: handshake framing (§5.2), chunk stream parsing (§5.3),
//! message reassembly (§6), AMF0 decoding (`[AMF0]`), and session state-machine
//! violations (§7.2) — see `private/specs-verified/rtmp/rtmp-runtime/rtmp.md`.

use crate::amf0::Amf0Value;

/// Errors produced by the RTMP ingest session engine.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum RtmpError {
    /// Not enough input bytes were available to parse a complete structure
    /// (handshake message, chunk header, or reassembled RTMP message).
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// The input did not conform to the expected wire layout (bad handshake
    /// version, invalid chunk `fmt`/CSID encoding, a length that overruns
    /// its buffer, or similar).
    #[error("malformed {what}")]
    Malformed {
        /// What was being parsed when the malformed data was found.
        what: &'static str,
    },

    /// A Set Chunk Size announced a value this engine cannot adopt: zero, a
    /// non-zero reserved top bit (§5.4.1), or above `MAX_CHUNK_SIZE`.
    #[error("chunk size {value} is outside the range this engine accepts")]
    ChunkSizeOutOfRange {
        /// The announced value, as it was on the wire.
        value: u32,
    },

    /// A Set Peer Bandwidth carried a limit type byte outside `0..=2`
    /// (§5.4.5).
    #[error("set peer bandwidth limit type {value} is not 0..=2")]
    InvalidLimitType {
        /// The offending byte.
        value: u8,
    },

    /// A User Control message carried an event type §6.7 does not define.
    #[error("unsupported user control event type {value}")]
    UnsupportedUserControlEvent {
        /// The offending event type.
        value: u16,
    },

    /// An AMF0 value used a marker this engine does not decode (`[AMF0]`
    /// §2.1's reserved, legacy and AMF3-switch markers).
    #[error("unsupported AMF0 marker {marker:#04x}")]
    UnsupportedAmf0Marker {
        /// The offending marker byte.
        marker: u8,
    },

    /// AMF0 data nested containers deeper than
    /// [`MAX_AMF0_DEPTH`](crate::amf0::MAX_AMF0_DEPTH).
    #[error("AMF0 nesting depth exceeds {max}")]
    Amf0DepthExceeded {
        /// The limit that was reached.
        max: usize,
    },

    /// An AMF0 String carried more than the 65535 bytes its 16-bit length
    /// prefix can hold; AMF0 has no Long String variant to fall back on
    /// inside these messages.
    #[error("{len} bytes do not fit the AMF0 string length prefix")]
    Amf0StringTooLong {
        /// The string's length.
        len: usize,
    },

    /// The first value of a Command Message body was not an AMF0 String
    /// (§7.1.1: the command name).
    #[error("command name is not an AMF0 string: {}", displayed(.value))]
    CommandNameNotString {
        /// The value that stood where the name belongs.
        value: Amf0Value,
    },

    /// The second value of a Command Message body was not an AMF0 Number
    /// (§7.1.1: the transaction id).
    #[error("command transaction id is not an AMF0 number: {}", displayed(.value))]
    CommandTransactionIdNotNumber {
        /// The value that stood where the transaction id belongs.
        value: Amf0Value,
    },

    /// The handshake version byte was not [`RTMP_VERSION`](crate::handshake::RTMP_VERSION)
    /// (§5.2.2).
    #[error("handshake version {value} is not {expected}")]
    HandshakeVersion {
        /// The version byte received.
        value: u8,
        /// The version this engine speaks.
        expected: u8,
    },

    /// An AMF0 Date carried a non-zero reserved time-zone field (`[AMF0]`
    /// §2.13: it MUST be `0x0000`).
    #[error("AMF0 date reserved time zone is {value}, must be 0")]
    Amf0DateTimeZone {
        /// The offending field.
        value: u16,
    },

    /// An AMF0 value was not valid UTF-8 where the format requires text
    /// (`[AMF0]` §2.4/§2.5/§2.14).
    #[error("invalid AMF0 {what}")]
    Amf0Utf8 {
        /// What held the invalid text.
        what: &'static str,
    },

    /// Reassembly held more distinct chunk streams than the per-connection
    /// bound allows.
    #[error("more than {max} concurrent chunk streams")]
    TooManyChunkStreams {
        /// The bound that was reached.
        max: usize,
    },

    /// The in-progress reassembly buffer grew past the per-connection
    /// budget (#1108/RTMP-W1).
    #[error("buffered input exceeds {max} bytes")]
    InputBudgetExceeded {
        /// The budget, in bytes.
        max: usize,
    },

    /// Assembling a message hit a chunk-stream state that cannot complete:
    /// a continuation before any chunk, a header on a completed message, a
    /// stream without a message type id, or a chunk that does not match its
    /// message's declared length.
    #[error("inconsistent chunk stream {chunk_stream_id}: {what}")]
    ChunkStreamState {
        /// The chunk stream id whose state did not hold.
        chunk_stream_id: u32,
        /// What was inconsistent.
        what: &'static str,
    },

    /// An operation was attempted, or a message arrived, that is not valid
    /// in the session's current state (e.g. a command received before the
    /// handshake completed, or `publish` before `createStream`). The server
    /// maps a rejected `publish` to `NetStream.Publish.BadName`; a client
    /// sees a rejected stream key as this variant after `publish`.
    #[error("unexpected state: {what}")]
    UnexpectedState {
        /// Description of the state violation.
        what: &'static str,
    },

    /// A [`crate::chunk::Message`] was built with a `chunk_stream_id` outside
    /// the basic header's encodable range (2..=65599, RTMP §5.3.1.1), so it
    /// cannot be written.
    #[error("chunk stream id {chunk_stream_id} is outside the encodable range 2..=65599")]
    InvalidChunkStreamId {
        /// The offending chunk stream id.
        chunk_stream_id: u32,
    },

    /// A length or count did not fit the wire field it is written to
    /// (#1129): a serializer that narrowed a `usize` with `as u32` (then
    /// packed it into a 24-bit field) would have silently wrapped and
    /// emitted a misframed chunk stream; this is returned instead.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),

    /// The output buffer passed to `serialize_into` was smaller than
    /// `serialized_len()`.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
}

/// Render one AMF0 value for an error message; the value's `Debug` form is
/// the only lossless rendering it has.
fn displayed(value: &Amf0Value) -> alloc::string::String {
    alloc::format!("{value:?}")
}

impl RtmpError {
    /// The error for an announced chunk size this engine cannot adopt.
    pub(crate) const fn chunk_size_out_of_range(value: u32) -> Self {
        RtmpError::ChunkSizeOutOfRange { value }
    }
}
