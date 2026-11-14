//! Server-side RTSP session engine — RFC 2326 Appendix A.2.
//!
//! [`ServerSession`] is sans-IO: [`ServerSession::handle_request`] parses an
//! inbound request, validates it against the §A.2 server state table (see
//! `private/specs-verified/rtp/rtsp-runtime/state-machines.md`), and returns the
//! serialized response bytes plus typed [`ServerEvent`]s. A method not valid in
//! the current state yields a `455 Method Not Valid In This State`
//! (see `private/specs-verified/rtp/rtsp-runtime/methods-and-status.md`); state
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

use core::time::Duration;

use crate::config::SessionIdSource;
use crate::error::{Error, MessageKind, Result};
use crate::server_config::RtspServerConfig;
use crate::session_id::SessionId;
use crate::state::{SessionState, server_next_state};
use crate::transport::{LowerTransport, Transport};

const SDP_CONTENT_TYPE: &str = "application/sdp";

const IMPLEMENTED_PUBLIC: &str = "OPTIONS, DESCRIBE, ANNOUNCE, GET_PARAMETER, SET_PARAMETER, SETUP, PLAY, PAUSE, RECORD, TEARDOWN";

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
    /// Interleaved binary media data arrived on the same TCP connection
    /// (RFC 2326 §10.12) — an RTCP receiver report during PLAY, or media
    /// during RECORD.
    MediaData {
        /// The interleaved channel id.
        channel: u8,
        /// The payload bytes (one upper-layer PDU).
        data: bytes::Bytes,
    },
}

/// A driveable RTSP server session (RFC 2326 §A.2).
pub struct ServerSession {
    state: SessionState,
    session_id: Option<SessionId>,
    /// Ids allocated so far, which a counter source advances by.
    allocated: u64,
    session_timeout: Option<Duration>,
    session_ids: SessionIdSource,
    negotiated_transport: Option<Transport>,
    server_header: String,
    describe_sdp: Option<Vec<u8>>,
    server_ports: Option<(u16, u16)>,
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
    /// Creates a fresh server session in the `Init` state, configured by
    /// `config`.
    ///
    /// Every allocated `Session` id is drawn from
    /// [`RtspServerConfig::session_ids`] at the `SETUP` that needs it.
    ///
    /// # Errors
    ///
    /// The OS random source failing to initialise is not reachable from a
    /// config (ids are drawn lazily), so this is infallible today; the
    /// [`Result`] leaves room for a validating config field.
    pub fn new(config: RtspServerConfig) -> Result<Self> {
        Ok(ServerSession {
            state: SessionState::Init,
            session_id: None,
            allocated: 0,
            session_timeout: config.session_timeout,
            session_ids: config.session_ids,
            negotiated_transport: None,
            server_header: config.server_header,
            describe_sdp: config.describe_sdp,
            server_ports: config.server_ports,
        })
    }

    /// The current session state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The allocated session id, once a SETUP has been handled.
    pub fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }

    /// The negotiated transport, once a SETUP has been handled.
    pub fn negotiated_transport(&self) -> Option<&Transport> {
        self.negotiated_transport.as_ref()
    }

    /// Parses an inbound request and returns the serialized response bytes plus
    /// the events produced.
    pub fn handle_request(&mut self, data: &[u8]) -> Result<(Vec<u8>, Vec<ServerEvent>)> {
        let (message, _consumed) =
            Message::<Body>::parse(data).map_err(|e| Error::MessageParse {
                what: MessageKind::Request,
                source: Box::new(e),
            })?;
        let request = match message {
            Message::Request(r) => r,
            _ => {
                return Err(Error::MessageParse {
                    what: MessageKind::Request,
                    source: "expected an RTSP request".into(),
                });
            }
        };
        self.handle_parsed(request)
    }

    fn handle_parsed(&mut self, request: Request<Body>) -> Result<(Vec<u8>, Vec<ServerEvent>)> {
        let method = request.method().clone();
        let Some(cseq) =
            header_value(request.header(&headers::CSEQ)).and_then(|s| s.trim().parse::<u32>().ok())
        else {
            let resp = Response::builder(Version::V1_0, StatusCode::BadRequest)
                .header(headers::SERVER, self.server_header.clone())
                .build(Vec::new());
            return Ok((serialize(&Message::from(resp))?, Vec::new()));
        };

        if !is_implemented(&method) {
            let resp = self.build_response(StatusCode::NotImplemented, cseq, |b| b);
            return Ok((serialize(&Message::from(resp))?, Vec::new()));
        }

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
                .and_then(Transport::first_supported);
            let Some(chosen) = chosen else {
                let resp = self.build_response(StatusCode::UnsupportedTransport, cseq, |b| b);
                return Ok((serialize(&Message::from(resp))?, events));
            };
            // §12.39: the reply carries the one spec the server selected.
            let mut chosen = chosen;
            if chosen.lower_transport != Some(LowerTransport::Tcp) && chosen.server_port.is_none() {
                chosen.server_port = self.server_ports;
            }
            let transport = Transport::single(chosen);
            let session_id = match self.session_id {
                Some(id) => id,
                None => self.allocate_session()?,
            };
            self.session_id = Some(session_id);
            self.negotiated_transport = Some(transport.clone());

            let mut session_hdr = crate::session_header::SessionHeader::new(session_id.to_string());
            session_hdr.timeout = self.session_timeout;
            let session_hdr = session_hdr.to_header_value()?;
            let transport_hdr = transport.to_header_value()?;
            let resp = self.build_response(StatusCode::Ok, cseq, |b| {
                b.header(headers::SESSION, session_hdr)
                    .header(headers::TRANSPORT, transport_hdr)
            });
            self.state = next_state;
            events.push(ServerEvent::SessionSetup {
                session_id: session_id.to_string(),
                transport,
            });
            events.push(ServerEvent::RequestAccepted {
                method,
                cseq,
                state: self.state,
            });
            return Ok((serialize(&Message::from(resp))?, events));
        }

        if let (Method::Describe, Some(sdp)) = (&method, self.describe_sdp.clone()) {
            let resp = Response::builder(Version::V1_0, StatusCode::Ok)
                .header(headers::CSEQ, cseq.to_string())
                .header(headers::SERVER, self.server_header.clone())
                .header(headers::CONTENT_TYPE, SDP_CONTENT_TYPE)
                .build(sdp);
            self.state = next_state;
            events.push(ServerEvent::RequestAccepted {
                method,
                cseq,
                state: self.state,
            });
            return Ok((serialize(&Message::from(resp))?, events));
        }

        // Other methods: echo Session if we have one, send 200, transition.
        let session_hdr = self.session_id.map(|id| id.to_string());
        let resp = self.build_response(StatusCode::Ok, cseq, |mut b| {
            if let Some(sid) = &session_hdr {
                b = b.header(headers::SESSION, sid.clone());
            }
            if method == Method::Options {
                b = b.header(headers::PUBLIC, IMPLEMENTED_PUBLIC);
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

    fn allocate_session(&mut self) -> Result<SessionId> {
        let id = self.session_ids.next_id(self.allocated)?;
        self.allocated += 1;
        Ok(id)
    }

    /// RFC 2326 §12.37: a `Session` header must name this session's id; a
    /// session-scoped method must carry one once a session exists.
    fn session_header_matches(&self, method: &Method, header: Option<&str>) -> bool {
        match (header, &self.session_id) {
            (Some(value), Some(ours)) => {
                let theirs = crate::session_header::SessionHeader::parse(value)
                    .map(|h| h.id)
                    .unwrap_or_default();
                broadcast_auth::constant_time_eq(theirs.as_bytes(), ours.to_string().as_bytes())
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

fn is_implemented(method: &Method) -> bool {
    matches!(
        method,
        Method::Options
            | Method::Describe
            | Method::Announce
            | Method::GetParameter
            | Method::SetParameter
            | Method::Setup
            | Method::Play
            | Method::Pause
            | Method::Teardown
            | Method::Record
    )
}

fn header_value(h: Option<&headers::HeaderValue>) -> Option<&str> {
    h.map(|v| v.as_str())
}

fn serialize(message: &Message<Body>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    message.write(&mut out).map_err(Error::MessageWrite)?;
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
        let seed = NEXT.fetch_add(0x2545_f491_4f6c_dd1d, Ordering::Relaxed);
        ServerSession::new(
            RtspServerConfig::default()
                .with_session_ids(crate::config::SessionIdSource::counter(seed)),
        )
        .expect("a session")
    }

    fn req(bytes: &str) -> Vec<u8> {
        bytes.replace('\n', "\r\n").into_bytes()
    }

    #[test]
    fn a_request_without_cseq_is_answered_400() {
        let mut s = test_session();
        let (resp, events) = s
            .handle_request(&req("OPTIONS rtsp://h/s RTSP/1.0\n\n"))
            .unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("RTSP/1.0 400"));
        assert!(events.is_empty());
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
        assert!(
            a.to_string().len() >= 16,
            "id {a} is shorter than 64 bits of hex"
        );
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
