//! Error type for the RTMP ingest session engine.
//!
//! Structured [`thiserror`] errors covering the failure surface of the
//! sans-IO engine: handshake framing (§5.2), chunk stream parsing (§5.3),
//! message reassembly (§6), AMF0 decoding (`[AMF0]`), and session state-machine
//! violations (§7.2) — see [`docs/rtmp.md`](../docs/rtmp.md).

/// Errors produced by the RTMP ingest session engine.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtmpError {
    /// Not enough input bytes were available to parse a complete structure
    /// (handshake message, chunk header, or reassembled RTMP message).
    #[error("buffer too short for {what}: need {need}, have {have}")]
    BufferTooShort {
        /// Number of bytes required to complete the parse.
        need: usize,
        /// Number of bytes actually available.
        have: usize,
        /// What was being parsed when the buffer ran out.
        what: &'static str,
    },

    /// The input did not conform to the expected wire layout (bad handshake
    /// version, invalid chunk `fmt`/CSID encoding, malformed AMF0 value, or
    /// similar).
    #[error("malformed {what}")]
    Malformed {
        /// What was being parsed when the malformed data was found.
        what: &'static str,
    },

    /// An operation was attempted that is not valid in the session's current
    /// state (e.g. a command received before the handshake completed, or
    /// `publish` before `createStream`).
    #[error("unexpected state: {what}")]
    UnexpectedState {
        /// Description of the state violation.
        what: &'static str,
    },

    /// The input used a feature or value this engine does not (yet) support
    /// (e.g. AMF3, an unrecognised command name).
    #[error("unsupported: {what}")]
    Unsupported {
        /// What is unsupported.
        what: &'static str,
    },
}
