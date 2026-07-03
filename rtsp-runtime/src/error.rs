//! Error types for the RTSP session engine.
//!
//! Structured [`thiserror`] errors covering the failure surface of the sans-IO
//! engine: RTSP message parse/serialize, state-machine violations (RFC 2326
//! Appendix A — see [`docs/state-machines.md`](../docs/state-machines.md)),
//! CSeq correlation, missing/invalid `Session`, `Transport` header parsing
//! (§12.39 — [`docs/transport-header.md`](../docs/transport-header.md)),
//! interleaved framing (§10.12 — [`docs/interleaved-framing.md`](../docs/interleaved-framing.md)),
//! and authentication (§14 — [`docs/auth.md`](../docs/auth.md)).

use crate::Method;
use crate::state::SessionState;

/// Result alias for the engine's fallible operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced by the RTSP session engine.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The RTSP message could not be parsed by the underlying `rtsp-types` codec.
    #[error("failed to parse RTSP message: {0}")]
    MessageParse(String),

    /// The RTSP message could not be serialized by the underlying codec.
    #[error("failed to serialize RTSP message: {0}")]
    MessageWrite(String),

    /// A method was invoked (client) or received (server) that is not permitted
    /// in the current session state. The server maps this to `455 Method Not
    /// Valid In This State` (RFC 2326 §11.3.6, Appendix A).
    #[error("method {method:?} not valid in state {state:?}")]
    MethodNotValidInState {
        /// The offending method.
        method: Method,
        /// The state the object was in when the method was attempted.
        state: SessionState,
    },

    /// A response arrived carrying a `CSeq` that does not match any outstanding
    /// request.
    #[error("no pending request for CSeq {0}")]
    UnknownCSeq(u32),

    /// A response arrived with no parseable `CSeq` header.
    #[error("response is missing a CSeq header")]
    MissingCSeq,

    /// An operation required an established session id but none was negotiated
    /// (no SETUP response has been processed yet).
    #[error("no session established (SETUP not completed)")]
    MissingSession,

    /// The `Transport` header value could not be parsed per RFC 2326 §12.39.
    #[error("invalid Transport header: {0}")]
    TransportParse(String),

    /// An interleaved `$` frame was malformed (RFC 2326 §10.12).
    #[error("invalid interleaved frame: {0}")]
    InterleavedFrame(String),

    /// Authentication failed: no credentials were configured, the challenge
    /// could not be parsed, or the digest computation failed (RFC 2326 §14).
    #[error("authentication failure: {0}")]
    Auth(String),

    /// The request required a request URI but none was set.
    #[error("request URI required but not set")]
    MissingRequestUri,

    /// A socket IO operation failed in the async adapter (feature `tokio`).
    #[error("socket IO error: {0}")]
    Io(String),

    /// A TLS operation failed in the async adapter (feature `tls`): bad server
    /// name, handshake, or certificate configuration (`rtsps://`, RFC 2326 §19).
    #[error("TLS error: {0}")]
    Tls(String),
}
