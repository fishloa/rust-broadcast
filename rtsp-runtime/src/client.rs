//! Client-side RTSP session engine — RFC 2326 Appendix A.1.
//!
//! [`ClientSession`] is a sans-IO driver: request-builder methods return the
//! outbound bytes to send, and [`ClientSession::handle_data`] consumes inbound
//! bytes (responses and interleaved `$` frames) and returns typed
//! [`ClientEvent`]s. It holds the session state, the next `CSeq`, the negotiated
//! `Session` id and timeout, optional credentials, and the digest
//! [`Authenticator`].
//!
//! Behaviour implemented here (see [`docs/state-machines.md`](../docs/state-machines.md),
//! [`docs/methods-and-status.md`](../docs/methods-and-status.md),
//! [`docs/auth.md`](../docs/auth.md)):
//!
//! - Request builders reject any method not valid in the current state
//!   ([`Error::MethodNotValidInState`]) before emitting bytes.
//! - Every request carries an incrementing `CSeq`, the `Session` id once known,
//!   and (once authenticated) a freshly-computed `Authorization` header.
//! - A `2xx` response advances the state per the §A.1 table; a `3xx` resets it
//!   to `Init`.
//! - A `401` with configured credentials transparently re-sends the request
//!   with `Authorization` (a new `CSeq`), including on `stale=true`.
//! - The `Session` id and timeout are captured from the SETUP response.
//! - Interleaved frames are surfaced as [`ClientEvent::MediaData`].

use std::collections::HashMap;

use rtsp_types::{Message, Method, Request, StatusCode, Version, headers};

use crate::auth::{Authenticator, Credentials, RequestContext};
use crate::error::{Error, Result};
use crate::interleaved::{self, MAGIC};
use crate::state::{SessionState, client_next_state};
use crate::transport::Transport;

/// A message body type: owned bytes.
type Body = Vec<u8>;

/// Record of a request the client has sent and is awaiting a response for.
#[derive(Debug, Clone)]
struct Pending {
    method: Method,
    uri: String,
    /// The full request, retained so it can be re-signed and re-sent on a 401.
    request: Request<Body>,
    /// Whether an auth retry has already been attempted for this logical request.
    auth_retried: bool,
}

/// An event produced by [`ClientSession::handle_data`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// A response was correlated to a request and the state machine updated.
    Response {
        /// The `CSeq` of the correlated request.
        cseq: u32,
        /// The method that was responded to.
        method: Method,
        /// The response status code.
        status: StatusCode,
        /// The response body (e.g. the SDP for a DESCRIBE), possibly empty.
        body: Vec<u8>,
    },
    /// The engine transparently re-sent a request with an `Authorization`
    /// header after a `401`. The caller MUST write `request` to the socket.
    AuthRetry {
        /// The method being retried.
        method: Method,
        /// The `CSeq` assigned to the retried request.
        cseq: u32,
        /// The serialized retried request bytes to send.
        request: Vec<u8>,
    },
    /// Interleaved binary media data (RFC 2326 §10.12).
    MediaData {
        /// The interleaved channel id.
        channel: u8,
        /// The payload bytes (one upper-layer PDU).
        data: Vec<u8>,
    },
}

/// A driveable RTSP client session (RFC 2326 §A.1).
#[derive(Debug)]
pub struct ClientSession {
    state: SessionState,
    next_cseq: u32,
    session_id: Option<String>,
    session_timeout: Option<u64>,
    credentials: Option<Credentials>,
    authenticator: Option<Authenticator>,
    negotiated_transport: Option<Transport>,
    pending: HashMap<u32, Pending>,
    /// Accumulates inbound bytes across `handle_data` calls (partial frames /
    /// partial messages).
    inbound: Vec<u8>,
    user_agent: String,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientSession {
    /// Creates a fresh client session in the `Init` state with `CSeq` starting
    /// at 1.
    pub fn new() -> Self {
        ClientSession {
            state: SessionState::Init,
            next_cseq: 1,
            session_id: None,
            session_timeout: None,
            credentials: None,
            authenticator: None,
            negotiated_transport: None,
            pending: HashMap::new(),
            inbound: Vec::new(),
            user_agent: "rtsp-runtime".to_string(),
        }
    }

    /// Attaches credentials so the engine can answer `401` challenges (§14).
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Overrides the `User-Agent` header value sent on requests.
    pub fn with_user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    /// The current session state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The negotiated session id, once a SETUP response has been processed.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The session timeout in seconds, if the SETUP response declared one.
    pub fn session_timeout(&self) -> Option<u64> {
        self.session_timeout
    }

    /// The transport negotiated in the SETUP response, if any.
    pub fn negotiated_transport(&self) -> Option<&Transport> {
        self.negotiated_transport.as_ref()
    }

    // --- Request builders -------------------------------------------------

    /// Builds an `OPTIONS` request (state-neutral).
    pub fn options(&mut self, uri: &str) -> Result<Vec<u8>> {
        self.build_request(Method::Options, uri, None, &[])
    }

    /// Builds a `DESCRIBE` request with `Accept: application/sdp` (state-neutral).
    pub fn describe(&mut self, uri: &str) -> Result<Vec<u8>> {
        self.build_request(
            Method::Describe,
            uri,
            None,
            &[(headers::ACCEPT, "application/sdp".to_string())],
        )
    }

    /// Builds a `SETUP` request carrying the given `Transport` (Init/Ready/…).
    pub fn setup(&mut self, uri: &str, transport: &Transport) -> Result<Vec<u8>> {
        self.build_request(
            Method::Setup,
            uri,
            None,
            &[(headers::TRANSPORT, transport.to_header_value())],
        )
    }

    /// Builds a `PLAY` request (valid in Ready/Playing).
    pub fn play(&mut self, uri: &str) -> Result<Vec<u8>> {
        self.build_request(Method::Play, uri, None, &[])
    }

    /// Builds a `PAUSE` request (valid in Playing/Recording).
    pub fn pause(&mut self, uri: &str) -> Result<Vec<u8>> {
        self.build_request(Method::Pause, uri, None, &[])
    }

    /// Builds a `TEARDOWN` request (valid in any non-Init state, and Init).
    pub fn teardown(&mut self, uri: &str) -> Result<Vec<u8>> {
        self.build_request(Method::Teardown, uri, None, &[])
    }

    /// Builds a `GET_PARAMETER` request, optionally with a body (state-neutral;
    /// an empty body is the liveness ping).
    pub fn get_parameter(&mut self, uri: &str, body: &[u8]) -> Result<Vec<u8>> {
        self.build_request_with_body(Method::GetParameter, uri, body, &[])
    }

    fn build_request(
        &mut self,
        method: Method,
        uri: &str,
        _range: Option<&str>,
        extra: &[(headers::HeaderName, String)],
    ) -> Result<Vec<u8>> {
        self.build_request_with_body(method, uri, &[], extra)
    }

    fn build_request_with_body(
        &mut self,
        method: Method,
        uri: &str,
        body: &[u8],
        extra: &[(headers::HeaderName, String)],
    ) -> Result<Vec<u8>> {
        // Reject methods not valid in the current state (state-neutral pass).
        client_next_state(self.state, &method)?;

        let cseq = self.next_cseq;
        let request = self.assemble(method.clone(), uri, cseq, body, extra)?;
        let bytes = serialize(&Message::from(request.clone()))?;
        self.next_cseq += 1;
        self.pending.insert(
            cseq,
            Pending {
                method,
                uri: uri.to_string(),
                request,
                auth_retried: false,
            },
        );
        Ok(bytes)
    }

    /// Assembles a `Request` with CSeq, User-Agent, Session (if known),
    /// Authorization (if authenticated), any extra headers, and the body.
    fn assemble(
        &mut self,
        method: Method,
        uri: &str,
        cseq: u32,
        body: &[u8],
        extra: &[(headers::HeaderName, String)],
    ) -> Result<Request<Body>> {
        let url = rtsp_types::Url::parse(uri)
            .map_err(|e| Error::TransportParse(format!("invalid request URI {uri:?}: {e}")))?;
        let mut builder = Request::builder(method.clone(), Version::V1_0)
            .request_uri(url)
            .header(headers::CSEQ, cseq.to_string())
            .header(headers::USER_AGENT, self.user_agent.clone());
        if let Some(sid) = &self.session_id {
            builder = builder.header(headers::SESSION, sid.clone());
        }
        for (name, value) in extra {
            builder = builder.header(name.clone(), value.clone());
        }
        if let Some(auth) = &mut self.authenticator {
            let ctx = RequestContext::new(<&str>::from(&method), uri);
            let value = auth.authorization(&ctx)?;
            builder = builder.header(headers::AUTHORIZATION, value);
        }
        let request = if body.is_empty() {
            builder.build(Vec::new())
        } else {
            builder.build(body.to_vec())
        };
        Ok(request)
    }

    // --- Inbound handling -------------------------------------------------

    /// Feeds inbound bytes and returns the events produced. Retains any partial
    /// trailing message or frame internally for the next call.
    pub fn handle_data(&mut self, data: &[u8]) -> Result<Vec<ClientEvent>> {
        self.inbound.extend_from_slice(data);
        let mut events = Vec::new();

        loop {
            if self.inbound.is_empty() {
                break;
            }
            if self.inbound[0] == MAGIC {
                // Interleaved frame path.
                match interleaved::InterleavedFrame::parse(&self.inbound)? {
                    Some((frame, consumed)) => {
                        events.push(ClientEvent::MediaData {
                            channel: frame.channel,
                            data: frame.payload,
                        });
                        self.inbound.drain(..consumed);
                    }
                    None => break, // need more bytes
                }
                continue;
            }

            // RTSP message path.
            match Message::<Body>::parse(&self.inbound) {
                Ok((message, consumed)) => {
                    self.inbound.drain(..consumed);
                    self.process_message(message, &mut events)?;
                }
                Err(rtsp_types::ParseError::Incomplete(_)) => break,
                Err(rtsp_types::ParseError::Error) => {
                    return Err(Error::MessageParse("malformed RTSP message".into()));
                }
            }
        }
        Ok(events)
    }

    fn process_message(
        &mut self,
        message: Message<Body>,
        events: &mut Vec<ClientEvent>,
    ) -> Result<()> {
        match message {
            Message::Response(response) => {
                let cseq = header_value(response.header(&headers::CSEQ))
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .ok_or(Error::MissingCSeq)?;
                let status = response.status();

                // 401: attempt a transparent auth retry.
                if status == StatusCode::Unauthorized {
                    if let Some(retry) = self.try_auth_retry(cseq, &response)? {
                        events.push(retry);
                        return Ok(());
                    }
                }

                let pending = self.pending.remove(&cseq).ok_or(Error::UnknownCSeq(cseq))?;

                // Capture Session id + timeout (typically from SETUP).
                if let Some(session_hdr) = header_value(response.header(&headers::SESSION)) {
                    let (id, timeout) = parse_session(session_hdr);
                    self.session_id = Some(id);
                    if timeout.is_some() {
                        self.session_timeout = timeout;
                    }
                }
                // Capture negotiated transport from SETUP response.
                if pending.method == Method::Setup {
                    if let Some(t) = header_value(response.header(&headers::TRANSPORT)) {
                        self.negotiated_transport = Some(Transport::parse(t)?);
                    }
                }

                // State transition.
                if status.is_success() {
                    self.state = client_next_state(self.state, &pending.method)?;
                    // TEARDOWN invalidates the session.
                    if pending.method == Method::Teardown {
                        self.session_id = None;
                        self.session_timeout = None;
                        self.authenticator = None;
                    }
                } else if status.is_redirection() {
                    self.state = SessionState::Init;
                }
                // 4xx (other than the handled 401) / 5xx: no state change.

                events.push(ClientEvent::Response {
                    cseq,
                    method: pending.method,
                    status,
                    body: response.into_body(),
                });
                Ok(())
            }
            Message::Data(data) => {
                events.push(ClientEvent::MediaData {
                    channel: data.channel_id(),
                    data: data.into_body(),
                });
                Ok(())
            }
            Message::Request(_) => {
                // Server-initiated requests (e.g. S->C OPTIONS, REDIRECT,
                // ANNOUNCE) are out of scope for this round; ignore.
                Ok(())
            }
        }
    }

    /// On a 401, build/refresh the authenticator from `WWW-Authenticate` and
    /// re-send the pending request with an `Authorization` header, unless a
    /// retry was already attempted (wrong credentials) or none are configured.
    fn try_auth_retry(
        &mut self,
        cseq: u32,
        response: &rtsp_types::Response<Body>,
    ) -> Result<Option<ClientEvent>> {
        let creds = match &self.credentials {
            Some(c) => c.clone(),
            None => return Ok(None),
        };
        // Only retry if the original request is still pending and hasn't retried.
        let (method, uri, already) = match self.pending.get(&cseq) {
            Some(p) => (p.method.clone(), p.uri.clone(), p.auth_retried),
            None => return Ok(None),
        };

        let challenge = header_value(response.header(&headers::WWW_AUTHENTICATE))
            .ok_or_else(|| Error::Auth("401 without WWW-Authenticate".into()))?;
        let stale = challenge.to_ascii_lowercase().contains("stale=true");

        // Fresh challenge => (re)build the authenticator. On stale=true this
        // picks up the new nonce; on first 401 it establishes the client.
        if self.authenticator.is_none() || already || stale {
            self.authenticator = Some(Authenticator::from_challenge(challenge, creds)?);
        }
        // Guard: if we already retried and it isn't a stale refresh, give up so
        // the caller sees the 401 (wrong credentials).
        if already && !stale {
            return Ok(None);
        }

        // Preserve method-specific headers (Accept/Transport/Range) before we
        // drop the old pending entry, then issue a new request with a fresh CSeq.
        let extra = self.replay_extra(&method, cseq);
        self.pending.remove(&cseq);
        let new_cseq = self.next_cseq;
        let request = self.assemble(method.clone(), &uri, new_cseq, &[], &extra)?;
        let bytes = serialize(&Message::from(request.clone()))?;
        self.next_cseq += 1;
        self.pending.insert(
            new_cseq,
            Pending {
                method: method.clone(),
                uri,
                request,
                auth_retried: true,
            },
        );
        Ok(Some(ClientEvent::AuthRetry {
            method,
            cseq: new_cseq,
            request: bytes,
        }))
    }

    /// Re-derive method-specific headers (e.g. Accept/Transport) for an auth
    /// replay from the previously-sent request.
    fn replay_extra(&self, _method: &Method, old_cseq: u32) -> Vec<(headers::HeaderName, String)> {
        let mut extra = Vec::new();
        if let Some(p) = self.pending.get(&old_cseq) {
            for name in [headers::ACCEPT, headers::TRANSPORT, headers::RANGE] {
                if let Some(v) = header_value(p.request.header(&name)) {
                    extra.push((name, v.to_string()));
                }
            }
        }
        extra
    }
}

/// Extracts the string value of an optional header.
fn header_value(h: Option<&headers::HeaderValue>) -> Option<&str> {
    h.map(|v| v.as_str())
}

/// Parses a `Session` header value into (id, optional timeout seconds).
fn parse_session(value: &str) -> (String, Option<u64>) {
    let mut parts = value.split(';').map(str::trim);
    let id = parts.next().unwrap_or("").to_string();
    let timeout = value
        .split(';')
        .filter_map(|s| s.trim().strip_prefix("timeout="))
        .find_map(|s| s.trim().parse::<u64>().ok());
    (id, timeout)
}

/// Serializes an RTSP message to bytes.
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

    #[test]
    fn play_in_init_bites() {
        let mut c = ClientSession::new();
        assert!(c.play("rtsp://h/s").is_err());
    }

    #[test]
    fn setup_allowed_in_init() {
        let mut c = ClientSession::new();
        let t = Transport::single(crate::transport::TransportSpec::rtp_avp_tcp_interleaved(
            0, 1,
        ));
        assert!(c.setup("rtsp://h/s", &t).is_ok());
    }

    #[test]
    fn cseq_increments() {
        let mut c = ClientSession::new();
        let a = c.options("rtsp://h/s").unwrap();
        let b = c.describe("rtsp://h/s").unwrap();
        assert!(String::from_utf8_lossy(&a).contains("CSeq: 1"));
        assert!(String::from_utf8_lossy(&b).contains("CSeq: 2"));
    }
}
