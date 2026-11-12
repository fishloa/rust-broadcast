//! Error types for the RTSP session engine.
//!
//! Structured [`thiserror`] errors covering the failure surface of the sans-IO
//! engine: RTSP message parse/serialize, state-machine violations (RFC 2326
//! Appendix A — see `private/specs-verified/rtp/rtsp-runtime/state-machines.md`),
//! CSeq correlation, missing/invalid `Session`, `Transport` header parsing
//! (§12.39 — `private/specs-verified/rtp/rtsp-runtime/transport-header.md`),
//! interleaved framing (§10.12 — `private/specs-verified/rtp/rtsp-runtime/interleaved-framing.md`),
//! and authentication (§14 — `private/specs-verified/rtp/rtsp-runtime/auth.md`).

use crate::Method;
use crate::StatusCode;
use crate::state::SessionState;

/// Result alias for the engine's fallible operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced by the RTSP session engine.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An RTSP message could not be parsed or framed; `source` carries the
    /// codec's or the framing check's own error.
    #[error("failed to parse RTSP {what}")]
    MessageParse {
        /// What was being parsed.
        what: MessageKind,
        /// The underlying parse or framing error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The RTSP message could not be serialized by the underlying codec.
    #[error("failed to serialize RTSP message")]
    MessageWrite(#[source] rtsp_types::WriteError),

    /// The SDP session description could not be written (RFC 4566 §5):
    /// `what` names the step (`"write"`, or `"utf-8"` when the writer's bytes
    /// were not valid UTF-8).
    #[error("failed to render the SDP session description ({what})")]
    SdpWrite {
        /// Which step failed.
        what: &'static str,
        /// The underlying IO error.
        #[source]
        source: std::io::Error,
    },

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

    /// The `Transport` header value could not be parsed per RFC 2326 §12.39.
    #[error("invalid Transport header: {0}")]
    TransportParse(String),

    /// A header value could not be serialized safely (control character in a value,
    /// or a name that is not an RFC 2326 §15.1 token / session-id character).
    #[error("cannot serialize header: {0}")]
    HeaderSerialize(String),

    /// The `Session` header value could not be parsed per RFC 2326 §12.37.
    #[error("invalid Session header: {0}")]
    SessionParse(String),

    /// An interleaved `$` frame was malformed (RFC 2326 §10.12).
    #[error("invalid interleaved frame: {0}")]
    InterleavedFrame(String),

    /// A request in a multi-request flow (e.g.
    /// [`AsyncRtspClient::start_record`](crate::AsyncRtspClient::start_record))
    /// was answered with a non-2xx status.
    #[error("{method:?} rejected with {status:?}")]
    UnexpectedResponse {
        /// The method that was rejected.
        method: Method,
        /// The status the server answered with.
        status: StatusCode,
    },

    /// A request in a multi-request flow was answered with something other
    /// than a response (RFC 2326 §7.1).
    #[error("unexpected event for a request: {event:?}")]
    UnexpectedEvent {
        /// The event the flow did not expect.
        event: crate::client::ClientEvent,
    },

    /// A caller-supplied URL cannot be used for the request it names (e.g. a
    /// cannot-be-a-base `rtsp:cam` has no authority to derive a control URL
    /// on, RFC 2326 Appendix C.1.1).
    #[error("invalid URL: {what}")]
    InvalidUrl {
        /// What is wrong with the URL.
        what: &'static str,
    },

    /// The SDP body returned by `DESCRIBE` could not be parsed.
    #[error("invalid SDP: {0}")]
    InvalidSdp(String),

    /// Two tracks negotiated the same interleaved channel.
    #[error("interleaved channel collision: {channel}")]
    InterleavedChannelCollision {
        /// The channel that collided.
        channel: u8,
    },

    /// A `SETUP` answer's `Transport` is not interleaved TCP with two distinct
    /// channels, or names a setup the plan does not have (RFC 2326 §12.39).
    #[error("unusable negotiated transport")]
    UnusableTransport,

    /// The interleaved channel pairs for the tracks of a play flow do not fit
    /// in the `u8` channel space from the requested first channel (§10.12).
    #[error("interleaved channels exhausted")]
    InterleavedChannelExhausted,

    /// Authentication failed: no credentials were configured, the challenge
    /// could not be parsed, or the digest computation failed (RFC 2326 §14).
    #[error("authentication failure")]
    Auth(#[from] broadcast_auth::Error),

    /// The server answered `401 Unauthorized` without a `WWW-Authenticate`
    /// challenge, so there is nothing to answer (RFC 2326 §14).
    #[error("authentication failure: 401 without WWW-Authenticate")]
    MissingAuthChallenge,

    /// A socket IO operation failed in the async adapter (feature `tokio`).
    #[error("socket IO error during {context}")]
    Io {
        /// The operation that failed.
        context: IoContext,
        /// The underlying socket error.
        #[source]
        source: std::io::Error,
    },

    /// A bounded wait in the async adapter expired (feature `tokio`).
    #[error("timed out waiting for {what}")]
    Timeout {
        /// Which bounded wait expired: `"connect"`, `"handshake"`, `"read"` or `"write"`.
        what: &'static str,
    },

    /// The server name for an `rtsps://` connection is not a valid TLS server
    /// name (feature `tls`).
    #[error("TLS error: invalid server name {server_name:?}")]
    Tls {
        /// The server name that was rejected.
        server_name: String,
        /// The underlying rustls error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The server answered `454 Session Not Found` (RFC 2326 §11.3.5) to a
    /// request on our session — the session is gone (timed out, or torn down
    /// server-side), so the caller must re-`SETUP`. Surfaced by the send-only
    /// push drain (a `GET_PARAMETER` keepalive is the usual trigger) instead
    /// of being silently discarded.
    #[error("RTSP session not found (454 {method:?}): the session is gone; re-SETUP")]
    SessionNotFound {
        /// The method whose request the server rejected with 454.
        method: Method,
    },
}

/// What an [`Error::Io`] was doing when the socket operation failed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoContext {
    /// Dialling the peer.
    Connect,
    /// The TLS handshake over an established connection.
    TlsHandshake,
    /// The peer closed the connection while a response or message was awaited.
    Read,
    /// A read or write failure surfaced by the framed codec.
    Socket,
}

impl IoContext {
    /// The label shown in [`Error::Io`]'s message.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::TlsHandshake => "TLS handshake",
            Self::Read => "read",
            Self::Socket => "socket",
        }
    }
}

impl core::fmt::Display for IoContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// What an [`Error::MessageParse`] was parsing.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// An inbound request (server side).
    Request,
    /// An inbound response (client side).
    Response,
    /// A message whose direction is not yet known: the shared header framing check.
    Framing,
}

impl MessageKind {
    /// The label shown in [`Error::MessageParse`]'s message.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Framing => "message framing",
        }
    }
}

impl core::fmt::Display for MessageKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io {
            context: IoContext::Socket,
            source: e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn io_keeps_its_source_and_kind() {
        let err = Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        let src = err
            .source()
            .and_then(|s| s.downcast_ref::<std::io::Error>());
        assert_eq!(
            src.map(std::io::Error::kind),
            Some(std::io::ErrorKind::ConnectionReset)
        );
    }

    #[test]
    fn message_parse_keeps_the_codec_error() {
        let err = Error::MessageParse {
            what: MessageKind::Response,
            source: Box::new(rtsp_types::ParseError::Error),
        };
        assert!(
            err.source()
                .is_some_and(|s| s.downcast_ref::<rtsp_types::ParseError>().is_some())
        );
    }

    #[test]
    fn message_write_keeps_the_codec_error() {
        let io = std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        let err = Error::MessageWrite(rtsp_types::WriteError::IoError(io));
        assert!(
            err.source()
                .is_some_and(|s| s.downcast_ref::<rtsp_types::WriteError>().is_some())
        );
    }

    fn chain(err: &Error) -> Vec<String> {
        let mut out = vec![err.to_string()];
        let mut cur = err.source();
        while let Some(e) = cur {
            out.push(e.to_string());
            cur = e.source();
        }
        out
    }

    #[test]
    fn no_message_in_a_source_chain_repeats_an_inner_one() {
        let io = || std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe gone");
        let errors = [
            Error::MessageParse {
                what: MessageKind::Request,
                source: Box::new(rtsp_types::ParseError::Error),
            },
            Error::MessageWrite(rtsp_types::WriteError::IoError(io())),
            Error::Io {
                context: IoContext::Connect,
                source: io(),
            },
            Error::from(io()),
            Error::Tls {
                server_name: "bad name".into(),
                source: Box::new(io()),
            },
            Error::Auth(broadcast_auth::Error::ChallengeParse("bad".into())),
        ];
        for err in &errors {
            let msgs = chain(err);
            let (own, inner) = msgs.split_first().unwrap();
            for message in inner {
                assert!(
                    !own.contains(message.as_str()),
                    "{own:?} repeats {message:?}"
                );
            }
        }
    }

    #[test]
    fn contexts_name_what_was_being_done() {
        let tls = Error::Tls {
            server_name: "bad name".into(),
            source: Box::new(std::io::Error::other("x")),
        };
        assert!(tls.to_string().contains("bad name"));
        let parse = Error::MessageParse {
            what: MessageKind::Request,
            source: Box::new(rtsp_types::ParseError::Error),
        };
        assert!(parse.to_string().contains("request"));
        assert_eq!(IoContext::TlsHandshake.to_string(), "TLS handshake");
    }
}
