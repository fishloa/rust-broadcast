//! Server-side RTSP session engine — RFC 2326 Appendix A.2.
//!
//! [`ServerSession`] is sans-IO: [`ServerSession::handle_request`] parses an
//! inbound request, validates it against the §A.2 server state table (see
//! [`docs/state-machines.md`](../docs/state-machines.md)), and returns the
//! serialized response bytes plus typed [`ServerEvent`]s. A method not valid in
//! the current state yields a `455 Method Not Valid In This State`
//! (see [`docs/methods-and-status.md`](../docs/methods-and-status.md)); state
//! advances only when a `2xx` is actually sent.
//!
//! On `SETUP` the server allocates a `Session` id (if none yet) and echoes the
//! client's `Transport` back (a minimal negotiation: the first offered spec is
//! accepted, else `461 Unsupported Transport`).

use rtsp_types::{headers, Message, Method, Request, Response, StatusCode, Version};

use crate::error::{Error, Result};
use crate::state::{server_next_state, SessionState};
use crate::transport::Transport;

/// A message body type: owned bytes.
type Body = Vec<u8>;

/// An event produced by [`ServerSession::handle_request`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    /// A request was accepted and the state machine advanced (2xx sent).
    RequestAccepted {
        /// The request method.
        method: Method,
        /// The `CSeq` echoed on the response.
        cseq: u32,
        /// The state after handling.
        state: SessionState,
    },
    /// A request was rejected in the current state; a `455` was returned.
    MethodNotValid {
        /// The rejected method.
        method: Method,
        /// The state the request was rejected in.
        state: SessionState,
    },
    /// A `SETUP` completed and a session id was allocated / reused.
    SessionSetup {
        /// The session id.
        session_id: String,
        /// The negotiated transport.
        transport: Transport,
    },
}

/// A driveable RTSP server session (RFC 2326 §A.2).
#[derive(Debug)]
pub struct ServerSession {
    state: SessionState,
    session_id: Option<String>,
    session_timeout: Option<u64>,
    next_session_seed: u64,
    negotiated_transport: Option<Transport>,
    server_header: String,
}

impl Default for ServerSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerSession {
    /// Creates a fresh server session in the `Init` state.
    pub fn new() -> Self {
        ServerSession {
            state: SessionState::Init,
            session_id: None,
            session_timeout: None,
            next_session_seed: 0x1234_5678,
            negotiated_transport: None,
            server_header: "rtsp-runtime".to_string(),
        }
    }

    /// Sets the timeout (seconds) advertised in the `Session` header on SETUP.
    pub fn with_session_timeout(mut self, seconds: u64) -> Self {
        self.session_timeout = Some(seconds);
        self
    }

    /// Overrides the seed used to allocate session ids (deterministic for tests).
    pub fn with_session_seed(mut self, seed: u64) -> Self {
        self.next_session_seed = seed;
        self
    }

    /// The current session state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The allocated session id, once a SETUP has been handled.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The negotiated transport, once a SETUP has been handled.
    pub fn negotiated_transport(&self) -> Option<&Transport> {
        self.negotiated_transport.as_ref()
    }

    /// Parses an inbound request and returns the serialized response bytes plus
    /// the events produced.
    pub fn handle_request(&mut self, data: &[u8]) -> Result<(Vec<u8>, Vec<ServerEvent>)> {
        let (message, _consumed) =
            Message::<Body>::parse(data).map_err(|e| Error::MessageParse(format!("{e:?}")))?;
        let request = match message {
            Message::Request(r) => r,
            _ => return Err(Error::MessageParse("expected an RTSP request".into())),
        };
        self.handle_parsed(request)
    }

    fn handle_parsed(&mut self, request: Request<Body>) -> Result<(Vec<u8>, Vec<ServerEvent>)> {
        let method = request.method().clone();
        let cseq = header_value(request.header(&headers::CSEQ))
            .and_then(|s| s.trim().parse::<u32>().ok())
            .ok_or(Error::MissingCSeq)?;

        // Validate the method against the current state (§A.2).
        let next_state = match server_next_state(self.state, &method) {
            Ok(s) => s,
            Err(_) => {
                let resp = self.build_response(StatusCode::MethodNotValidInThisState, cseq, |b| b);
                let bytes = serialize(&Message::from(resp))?;
                return Ok((
                    bytes,
                    vec![ServerEvent::MethodNotValid {
                        method,
                        state: self.state,
                    }],
                ));
            }
        };

        let mut events = Vec::new();

        // SETUP: allocate session + negotiate transport.
        if method == Method::Setup {
            let transport = match header_value(request.header(&headers::TRANSPORT)) {
                Some(t) => Transport::parse(t)?,
                None => {
                    let resp = self.build_response(StatusCode::UnsupportedTransport, cseq, |b| b);
                    return Ok((serialize(&Message::from(resp))?, events));
                }
            };
            if transport.first().is_none() {
                let resp = self.build_response(StatusCode::UnsupportedTransport, cseq, |b| b);
                return Ok((serialize(&Message::from(resp))?, events));
            }
            let session_id = self
                .session_id
                .clone()
                .unwrap_or_else(|| self.allocate_session());
            self.session_id = Some(session_id.clone());
            self.negotiated_transport = Some(transport.clone());

            let sid = session_id.clone();
            let session_hdr = match self.session_timeout {
                Some(t) => format!("{sid};timeout={t}"),
                None => sid.clone(),
            };
            let transport_hdr = transport.to_header_value();
            let resp = self.build_response(StatusCode::Ok, cseq, |b| {
                b.header(headers::SESSION, session_hdr)
                    .header(headers::TRANSPORT, transport_hdr)
            });
            self.state = next_state;
            events.push(ServerEvent::SessionSetup {
                session_id,
                transport,
            });
            events.push(ServerEvent::RequestAccepted {
                method,
                cseq,
                state: self.state,
            });
            return Ok((serialize(&Message::from(resp))?, events));
        }

        // Other methods: echo Session if we have one, send 200, transition.
        let session_hdr = self.session_id.clone();
        let resp = self.build_response(StatusCode::Ok, cseq, |mut b| {
            if let Some(sid) = &session_hdr {
                b = b.header(headers::SESSION, sid.clone());
            }
            b
        });
        self.state = next_state;
        if method == Method::Teardown {
            self.session_id = None;
            self.negotiated_transport = None;
        }
        events.push(ServerEvent::RequestAccepted {
            method,
            cseq,
            state: self.state,
        });
        Ok((serialize(&Message::from(resp))?, events))
    }

    fn allocate_session(&mut self) -> String {
        let id = self.next_session_seed;
        // Advance deterministically for any subsequent allocation.
        self.next_session_seed = self.next_session_seed.wrapping_add(1);
        format!("{id:08}")
    }

    /// Builds a response with Version 1.0, CSeq, and Server headers, plus any
    /// headers added by `f`.
    fn build_response<F>(&self, status: StatusCode, cseq: u32, f: F) -> Response<Body>
    where
        F: FnOnce(rtsp_types::ResponseBuilder) -> rtsp_types::ResponseBuilder,
    {
        let builder = Response::builder(Version::V1_0, status)
            .header(headers::CSEQ, cseq.to_string())
            .header(headers::SERVER, self.server_header.clone());
        f(builder).build(Vec::new())
    }
}

fn header_value(h: Option<&headers::HeaderValue>) -> Option<&str> {
    h.map(|v| v.as_str())
}

fn serialize(message: &Message<Body>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    message
        .write(&mut out)
        .map_err(|e| Error::MessageWrite(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(bytes: &str) -> Vec<u8> {
        bytes.replace('\n', "\r\n").into_bytes()
    }

    #[test]
    fn setup_transitions_init_to_ready() {
        let mut s = ServerSession::new();
        let (resp, events) = s
            .handle_request(&req(
                "SETUP rtsp://h/s RTSP/1.0\nCSeq: 1\nTransport: RTP/AVP/TCP;interleaved=0-1\n\n",
            ))
            .unwrap();
        assert_eq!(s.state(), SessionState::Ready);
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("200"));
        assert!(text.contains("Session:"));
        assert!(text.contains("Transport:"));
        assert!(events
            .iter()
            .any(|e| matches!(e, ServerEvent::SessionSetup { .. })));
    }

    #[test]
    fn play_in_init_returns_455() {
        let mut s = ServerSession::new();
        let (resp, events) = s
            .handle_request(&req("PLAY rtsp://h/s RTSP/1.0\nCSeq: 1\n\n"))
            .unwrap();
        assert_eq!(s.state(), SessionState::Init);
        assert!(String::from_utf8_lossy(&resp).contains("455"));
        assert!(events
            .iter()
            .any(|e| matches!(e, ServerEvent::MethodNotValid { .. })));
    }
}
