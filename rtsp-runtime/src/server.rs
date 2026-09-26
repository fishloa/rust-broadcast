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
//! On `SETUP` the server allocates a `Session` id (if none yet) from the
//! caller-supplied random source (RFC 2326 §3.4: session ids should be chosen
//! randomly) and answers with the single transport-spec it chose (§12.39: the
//! first one offered), or `461 Unsupported Transport` when none parses. Once a
//! session exists, a request whose `Session` header names a different id — or
//! a session-scoped request (`PLAY`/`PAUSE`/`RECORD`/`TEARDOWN`) with none —
//! is answered `454 Session Not Found` (§11.3.2, §12.37).

use rtsp_types::{Message, Method, Request, Response, StatusCode, Version, headers};

use crate::error::{Error, Result};
use crate::state::{SessionState, server_next_state};
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

/// The source of `Session` id values a [`ServerSession`] draws from.
type SessionIdSource = Box<dyn FnMut() -> u64 + Send>;

/// A driveable RTSP server session (RFC 2326 §A.2).
pub struct ServerSession {
    state: SessionState,
    session_id: Option<String>,
    session_timeout: Option<u64>,
    session_ids: SessionIdSource,
    negotiated_transport: Option<Transport>,
    server_header: String,
}

impl core::fmt::Debug for ServerSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ServerSession")
            .field("state", &self.state)
            .field("session_id", &self.session_id)
            .field("session_timeout", &self.session_timeout)
            .field("negotiated_transport", &self.negotiated_transport)
            .field("server_header", &self.server_header)
            .finish_non_exhaustive()
    }
}

/// Methods that act on an established session and so must name it
/// (RFC 2326 §12.37).
const SESSION_SCOPED_METHODS: [Method; 4] = [
    Method::Play,
    Method::Pause,
    Method::Record,
    Method::Teardown,
];

impl ServerSession {
    /// Creates a fresh server session in the `Init` state.
    ///
    /// `session_ids` is called once per allocated `Session` id and must return
    /// values from a cryptographically secure random source (RFC 2326 §3.4) —
    /// the id is the only thing tying later requests to this session. The
    /// `tokio` adapter's `io::AsyncRtspServer::accept` supplies the OS RNG.
    pub fn new(session_ids: impl FnMut() -> u64 + Send + 'static) -> Self {
        ServerSession {
            state: SessionState::Init,
            session_id: None,
            session_timeout: None,
            session_ids: Box::new(session_ids),
            negotiated_transport: None,
            server_header: "rtsp-runtime".to_string(),
        }
    }

    /// Sets the timeout (seconds) advertised in the `Session` header on SETUP.
    pub fn with_session_timeout(mut self, seconds: u64) -> Self {
        self.session_timeout = Some(seconds);
        self
    }

    /// Replaces the id source with a counter starting at `seed` — predictable
    /// ids, for tests only.
    pub fn with_session_seed(mut self, seed: u64) -> Self {
        let mut next = seed;
        self.session_ids = Box::new(move || {
            let id = next;
            next = next.wrapping_add(1);
            id
        });
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

        if !self.session_header_matches(&method, header_value(request.header(&headers::SESSION))) {
            let resp = self.build_response(StatusCode::SessionNotFound, cseq, |b| b);
            return Ok((serialize(&Message::from(resp))?, Vec::new()));
        }

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
            let chosen = header_value(request.header(&headers::TRANSPORT))
                .and_then(|t| Transport::parse(t).ok())
                .and_then(|t| t.first().cloned());
            let Some(chosen) = chosen else {
                let resp = self.build_response(StatusCode::UnsupportedTransport, cseq, |b| b);
                return Ok((serialize(&Message::from(resp))?, events));
            };
            // §12.39: the reply carries the one spec the server selected.
            let transport = Transport::single(chosen);
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
        let id = (self.session_ids)();
        format!("{id:016x}")
    }

    /// RFC 2326 §12.37: a `Session` header must name this session's id; a
    /// session-scoped method must carry one once a session exists.
    fn session_header_matches(&self, method: &Method, header: Option<&str>) -> bool {
        match (header, &self.session_id) {
            (Some(value), Some(ours)) => {
                let theirs = value.split(';').next().unwrap_or_default().trim();
                ids_equal(theirs.as_bytes(), ours.as_bytes())
            }
            (Some(_), None) => false,
            (None, Some(_)) => !SESSION_SCOPED_METHODS.contains(method),
            (None, None) => true,
        }
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

/// Equality that does not stop at the first differing byte; lengths are not
/// secret.
fn ids_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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

    /// A session drawing ids from a counter unique per call site in this test
    /// binary, standing in for a random source.
    fn test_session() -> ServerSession {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
        ServerSession::new(|| NEXT.fetch_add(0x2545_f491_4f6c_dd1d, Ordering::Relaxed))
    }

    fn req(bytes: &str) -> Vec<u8> {
        bytes.replace('\n', "\r\n").into_bytes()
    }

    #[test]
    fn setup_transitions_init_to_ready() {
        let mut s = test_session();
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
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::SessionSetup { .. }))
        );
    }

    #[test]
    fn play_in_init_returns_455() {
        let mut s = test_session();
        let (resp, events) = s
            .handle_request(&req("PLAY rtsp://h/s RTSP/1.0\nCSeq: 1\n\n"))
            .unwrap();
        assert_eq!(s.state(), SessionState::Init);
        assert!(String::from_utf8_lossy(&resp).contains("455"));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::MethodNotValid { .. }))
        );
    }

    fn setup(s: &mut ServerSession, transport: &str) -> String {
        let (resp, _) = s
            .handle_request(&req(&format!(
                "SETUP rtsp://h/s RTSP/1.0\nCSeq: 1\nTransport: {transport}\n\n"
            )))
            .unwrap();
        String::from_utf8(resp).unwrap()
    }

    #[test]
    fn session_ids_differ_across_sessions_and_carry_64_bits() {
        let mut a = test_session();
        let mut b = test_session();
        setup(&mut a, "RTP/AVP/TCP;interleaved=0-1");
        setup(&mut b, "RTP/AVP/TCP;interleaved=0-1");
        let (a, b) = (a.session_id().unwrap(), b.session_id().unwrap());
        assert_ne!(a, b, "two sessions must not share an id");
        assert!(a.len() >= 16, "id {a} is shorter than 64 bits of hex");
    }

    #[test]
    fn request_with_wrong_session_header_gets_454() {
        let mut s = test_session();
        setup(&mut s, "RTP/AVP/TCP;interleaved=0-1");
        let (resp, _) = s
            .handle_request(&req(
                "PLAY rtsp://h/s RTSP/1.0\nCSeq: 2\nSession: not-this-one\n\n",
            ))
            .unwrap();
        assert!(
            String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 454"),
            "got: {}",
            String::from_utf8_lossy(&resp)
        );
        assert_eq!(s.state(), SessionState::Ready);
        // Missing Session on a session-scoped method is rejected the same way.
        let (resp, _) = s
            .handle_request(&req("TEARDOWN rtsp://h/s RTSP/1.0\nCSeq: 3\n\n"))
            .unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 454"));
        assert_eq!(s.state(), SessionState::Ready);
        // The right id (with a timeout parameter) is accepted.
        let sid = s.session_id().unwrap().to_string();
        let (resp, _) = s
            .handle_request(&req(&format!(
                "PLAY rtsp://h/s RTSP/1.0\nCSeq: 4\nSession: {sid};timeout=60\n\n"
            )))
            .unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 200"));
        assert_eq!(s.state(), SessionState::Playing);
    }

    #[test]
    fn setup_answers_exactly_one_transport() {
        let mut s = test_session();
        let text = setup(
            &mut s,
            "RTP/AVP;unicast;client_port=8000-8001,RTP/AVP/TCP;unicast;interleaved=0-1",
        );
        let line = text.lines().find(|l| l.starts_with("Transport:")).unwrap();
        let answered = Transport::parse(line["Transport:".len()..].trim()).unwrap();
        assert_eq!(answered.specs.len(), 1, "got: {line}");
        assert_eq!(answered.first().unwrap().client_port, Some((8000, 8001)));
        assert_eq!(s.negotiated_transport().unwrap().specs.len(), 1);
    }

    #[test]
    fn unparseable_transport_gets_461() {
        let mut s = test_session();
        let text = setup(&mut s, "not a transport;;");
        assert!(text.starts_with("RTSP/1.0 461"), "got: {text}");
        assert_eq!(s.state(), SessionState::Init);
    }
}
