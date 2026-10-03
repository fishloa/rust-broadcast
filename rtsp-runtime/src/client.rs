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
use std::time::{Duration, Instant};

use rtsp_types::{Message, Method, Request, StatusCode, Version, headers};

use crate::auth::{Authenticator, Credentials, RequestContext};
use crate::error::{Error, Result};
use crate::interleaved::{self, MAGIC};
use crate::state::{SessionState, client_next_state};
use crate::transport::Transport;

/// A message body type: owned bytes.
type Body = Vec<u8>;

/// RFC 2326 §12.37: the `Session` timeout when the server declares none.
const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 60;

/// The keepalive is sent at `timeout / KEEPALIVE_FRACTION_DEN` after the last
/// activity, so one lost keepalive still leaves time for a retry.
const KEEPALIVE_FRACTION_DEN: u32 = 2;

/// Most recent `Session` warnings kept by [`ClientSession::session_warnings`]
/// (a hostile server cannot grow memory by sending a bad header on every response).
pub const MAX_SESSION_WARNINGS: usize = 16;

/// Shortest keepalive interval, whatever the session timeout: a hostile or buggy
/// `timeout=` (the parser already maps 0 to the default) must never turn the
/// keepalive into a busy loop.
pub const MIN_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum bytes retained in [`ClientSession::inbound`] while waiting for a
/// complete response or interleaved frame. An unterminated header, or a
/// `Content-Length` that promises more than the peer ever sends, would
/// otherwise grow this buffer without bound (multimux pulls from
/// operator-configured but externally hosted RTSP URLs) — 1 MiB is generous
/// for RTSP headers plus an SDP body (audit run-09 W6).
const MAX_INBOUND_BYTES: usize = 1024 * 1024;

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
    /// When the caller last reported traffic (see [`ClientSession::mark_activity`]).
    last_activity: Option<Instant>,
    /// The URI of the most recent request, reused for the keepalive.
    last_uri: Option<String>,
    /// Recoverable `Session` header problems seen so far (see `session_warnings`).
    session_warnings: Vec<String>,
    /// Total `Session` warnings ever recorded (the list keeps only the latest).
    session_warning_total: usize,
    /// How much of `inbound` the header-terminator search already covered.
    scanned: usize,
    /// `inbound` length below which a re-parse cannot succeed (body wait).
    skip_until: usize,
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
            last_activity: None,
            last_uri: None,
            session_warnings: Vec::new(),
            session_warning_total: 0,
            scanned: 0,
            skip_until: 0,
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

    /// The `CSeq` the next request-builder call (`options`/`describe`/…) will
    /// assign. Lets an IO adapter capture which response it must wait for
    /// *before* building the request, since building it also consumes this
    /// value (audit run-09 W7: `io::AsyncRtspClient::exchange` needs this to
    /// tell "the" response apart from a stray one for an abandoned request).
    pub fn peek_next_cseq(&self) -> u32 {
        self.next_cseq
    }

    fn record_session_warning(&mut self, w: String) {
        self.session_warning_total += 1;
        if self.session_warnings.len() >= MAX_SESSION_WARNINGS {
            self.session_warnings.remove(0);
        }
        self.session_warnings.push(w);
    }

    /// Total number of `Session` warnings recorded, including ones that have
    /// since been dropped from [`session_warnings`](Self::session_warnings).
    pub fn session_warning_count(&self) -> usize {
        self.session_warning_total
    }

    /// Warnings about `Session` response headers (a malformed or zero `timeout`
    /// that fell back to the 60 s default, or an unusable header), oldest first.
    /// Each is also emitted with `log::warn!`.
    pub fn session_warnings(&self) -> &[String] {
        &self.session_warnings
    }

    /// True if a partial message or frame is buffered (bytes fed to
    /// [`handle_data`](Self::handle_data) that did not yet form a whole one). An
    /// adapter uses it to tell a clean end of stream from a truncated one.
    pub fn has_buffered_input(&self) -> bool {
        !self.inbound.is_empty()
    }

    /// Records that a request was written at `now`. The core never reads a clock;
    /// `now` is always caller-supplied. Only requests count: the server's
    /// session timeout is refreshed by requests (RFC 2326 §12.37), not by
    /// inbound media or responses.
    pub fn mark_activity(&mut self, now: Instant) {
        self.last_activity = Some(now);
    }

    /// Next keepalive deadline: `last_activity + timeout/2` once a Session id
    /// exists (timeout = the SETUP response's `timeout=`, else the RFC 2326
    /// §12.37 default of 60 s). `None` before SETUP, after TEARDOWN, or if no
    /// activity was ever recorded.
    pub fn poll_timeout(&self) -> Option<Instant> {
        self.session_id.as_ref()?;
        let timeout =
            Duration::from_secs(self.session_timeout.unwrap_or(DEFAULT_SESSION_TIMEOUT_SECS));
        // `checked_add`: a hostile huge `timeout=` must not overflow `Instant`.
        self.last_activity?
            .checked_add((timeout / KEEPALIVE_FRACTION_DEN).max(MIN_KEEPALIVE_INTERVAL))
    }

    /// If `now >= poll_timeout()`, the bytes of a `GET_PARAMETER` keepalive on
    /// the last request URI (and activity is re-armed at `now`); `Ok(None)`
    /// otherwise.
    pub fn handle_timeout(&mut self, now: Instant) -> Result<Option<Vec<u8>>> {
        match self.poll_timeout() {
            Some(deadline) if now >= deadline => {}
            _ => return Ok(None),
        }
        let Some(uri) = self.last_uri.clone() else {
            return Ok(None);
        };
        let bytes = self.get_parameter(&uri, b"")?;
        self.last_activity = Some(now);
        Ok(Some(bytes))
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
            &[(headers::TRANSPORT, transport.to_header_value()?)],
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

    /// Builds an `ANNOUNCE` request carrying an SDP body (RFC 2326 §10.3).
    pub fn announce(&mut self, uri: &str, sdp: &str) -> Result<Vec<u8>> {
        self.build_request_with_body(
            Method::Announce,
            uri,
            sdp.as_bytes(),
            &[(headers::CONTENT_TYPE, "application/sdp".to_string())],
        )
    }

    /// Builds a `RECORD` request (RFC 2326 §10.11, valid in Ready).
    pub fn record(&mut self, uri: &str) -> Result<Vec<u8>> {
        self.build_request(Method::Record, uri, None, &[])
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
        self.last_uri = Some(uri.to_string());
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
        if self.inbound.len() > MAX_INBOUND_BYTES {
            return Err(Error::MessageParse(format!(
                "inbound buffer of {} bytes exceeds the {MAX_INBOUND_BYTES}-byte maximum \
                 (unterminated header or an unfulfilled Content-Length)",
                self.inbound.len()
            )));
        }
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

            // RTSP message path. `framing::head_end` is a bounded framing check
            // (header cap, parse once per message), not protocol parsing.
            if self.inbound.len() < self.skip_until {
                break;
            }
            if crate::framing::head_end(&self.inbound, self.scanned)?.is_none() {
                self.scanned = self.inbound.len();
                break;
            }
            match Message::<Body>::parse(&self.inbound) {
                Ok((message, consumed)) => {
                    self.inbound.drain(..consumed);
                    self.scanned = 0;
                    self.skip_until = 0;
                    self.process_message(message, &mut events)?;
                }
                Err(rtsp_types::ParseError::Incomplete(needed)) => {
                    if let Some(more) = needed {
                        self.skip_until = self.inbound.len().saturating_add(more.get());
                    }
                    break;
                }
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
                // A response with no parseable CSeq can't be correlated to
                // anything pending. Skip it rather than aborting the whole
                // `handle_data` call — bytes already drained from `inbound`
                // (and events already decoded from them earlier in this same
                // call) must not be discarded for one bad response (RFC 2326
                // gives no reason to treat this as fatal).
                let Some(cseq) = header_value(response.header(&headers::CSEQ))
                    .and_then(|s| s.trim().parse::<u32>().ok())
                else {
                    return Ok(());
                };
                let status = response.status();

                // 401: attempt a transparent auth retry.
                if status == StatusCode::Unauthorized
                    && let Some(retry) = self.try_auth_retry(cseq, &response)?
                {
                    events.push(retry);
                    return Ok(());
                }

                // An unknown or already-answered CSeq (a duplicate response,
                // or a reply to a request the caller gave up waiting for) is
                // likewise skipped rather than fatal, for the same reason.
                let Some(pending) = self.pending.remove(&cseq) else {
                    return Ok(());
                };

                // Capture Session id + timeout (typically from SETUP). Recoverable
                // problems (a malformed or zero timeout) are logged and kept in
                // `session_warnings()`; an unusable header is logged and ignored.
                if let Some(session_hdr) = header_value(response.header(&headers::SESSION)) {
                    match crate::session_header::SessionHeader::parse_with_warnings(session_hdr) {
                        Ok((parsed, warnings)) => {
                            for w in warnings {
                                log::warn!("RTSP Session header {session_hdr:?}: {w}");
                                self.record_session_warning(w);
                            }
                            self.session_id = Some(parsed.id);
                            if let Some(t) = parsed.timeout {
                                self.session_timeout = Some(t.as_secs());
                            }
                        }
                        Err(e) => {
                            let w = format!("unusable Session header {session_hdr:?}: {e}");
                            log::warn!("RTSP {w}");
                            self.record_session_warning(w);
                        }
                    }
                }
                // Capture negotiated transport from SETUP response.
                if pending.method == Method::Setup
                    && let Some(t) = header_value(response.header(&headers::TRANSPORT))
                {
                    self.negotiated_transport = Some(Transport::parse(t)?);
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
        let stale = crate::headers_util::challenge_is_stale(challenge);

        // Always rebuild the authenticator from THIS challenge. A server can
        // rotate its nonce per session or per time window without setting
        // `stale=true` (RFC 2326 §14 doesn't require it); gating the rebuild
        // on `stale` meant a rotated-but-not-stale nonce got signed with the
        // old value, failed again, and was then abandoned instead of retried
        // with the fresh one.
        self.authenticator = Some(Authenticator::from_challenge(challenge, creds)?);
        // Guard: if this pending request was already retried once and it's
        // still a 401 with no `stale=true`, give up so the caller sees it
        // (wrong credentials). `stale=true` is the server's explicit signal
        // that the credentials were fine and only the nonce needs refreshing,
        // so it's allowed one more retry even after an earlier one.
        if already && !stale {
            return Ok(None);
        }

        // Preserve the original body and every non-hop-by-hop header (e.g.
        // Content-Type for an ANNOUNCE's SDP, Accept/Transport/Range for a
        // SETUP/PLAY) before we drop the old pending entry, then issue a new
        // request with a fresh CSeq. RFC 2326 §10.3 requires the retried
        // ANNOUNCE to carry the same SDP body as the original.
        let (body, extra) = self.replay_extra(cseq);
        self.pending.remove(&cseq);
        let new_cseq = self.next_cseq;
        let request = self.assemble(method.clone(), &uri, new_cseq, &body, &extra)?;
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

    /// Re-derives the body and every replayable header (e.g. Content-Type,
    /// Accept/Transport/Range) for an auth replay from the previously-sent
    /// request. `CSeq`, `Session` and `Authorization` are excluded because
    /// `assemble` sets them itself for the retry, and `Content-Length` is
    /// excluded because `RequestBuilder::build` recomputes it from `body`.
    fn replay_extra(&self, old_cseq: u32) -> (Vec<u8>, Vec<(headers::HeaderName, String)>) {
        let Some(p) = self.pending.get(&old_cseq) else {
            return (Vec::new(), Vec::new());
        };
        let hop_by_hop = [
            headers::CSEQ,
            headers::SESSION,
            headers::AUTHORIZATION,
            headers::USER_AGENT,
            headers::CONTENT_LENGTH,
        ];
        let extra = p
            .request
            .headers()
            .filter(|(name, _)| !hop_by_hop.iter().any(|h| h == *name))
            .map(|(name, value)| (name.clone(), value.as_str().to_string()))
            .collect();
        (p.request.body().clone(), extra)
    }
}

/// Extracts the string value of an optional header.
fn header_value(h: Option<&headers::HeaderValue>) -> Option<&str> {
    h.map(|v| v.as_str())
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

    #[test]
    fn announce_emits_request_with_sdp_content_type() {
        let mut c = ClientSession::new();
        let sdp = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=Test\r\n";
        let bytes = c.announce("rtsp://h/s", sdp).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("ANNOUNCE rtsp://h/s"));
        assert!(s.contains("Content-Type: application/sdp"));
        assert!(s.contains(sdp));
    }

    // Regression (audit run-09 W5): one unknown/duplicate CSeq in a batch of
    // inbound bytes must not discard events already decoded earlier in the
    // same `handle_data` call. Before the fix, `handle_data` returned `Err`
    // on the first unmatched CSeq, so a caller lost the CSeq-1 `Response`
    // event even though its bytes had already been drained from `inbound`.
    #[test]
    fn unknown_cseq_is_skipped_without_discarding_earlier_events() {
        fn wire(s: &str) -> Vec<u8> {
            s.replace('\n', "\r\n").into_bytes()
        }
        let mut c = ClientSession::new();
        c.options("rtsp://h/s").unwrap(); // CSeq 1
        c.describe("rtsp://h/s").unwrap(); // CSeq 2

        // A valid response (CSeq 1), then a duplicate/unmatched response
        // (CSeq 99, never sent), then another valid response (CSeq 2), all
        // in one `handle_data` call.
        let mut batch = wire("RTSP/1.0 200 OK\nCSeq: 1\n\n");
        batch.extend(wire("RTSP/1.0 200 OK\nCSeq: 99\n\n"));
        batch.extend(wire("RTSP/1.0 200 OK\nCSeq: 2\n\n"));

        let events = c
            .handle_data(&batch)
            .expect("an unmatched CSeq must not fail the whole batch");
        let cseqs: Vec<u32> = events
            .into_iter()
            .map(|e| match e {
                ClientEvent::Response { cseq, .. } => cseq,
                other => panic!("unexpected event: {other:?}"),
            })
            .collect();
        assert_eq!(
            cseqs,
            vec![1, 2],
            "expected both real responses despite the unmatched CSeq 99"
        );
    }

    // Regression (audit run-09 W6): an unterminated header (or a peer that
    // drips bytes forever without completing a message) must not grow
    // `inbound` without bound.
    #[test]
    fn inbound_buffer_is_capped_against_an_unterminated_header() {
        let mut c = ClientSession::new();
        c.options("rtsp://h/s").unwrap();
        let chunk = vec![b'A'; 64 * 1024];
        let mut last = Ok(Vec::new());
        for _ in 0..(MAX_INBOUND_BYTES / chunk.len() + 2) {
            last = c.handle_data(&chunk);
            if last.is_err() {
                break;
            }
        }
        assert!(
            last.is_err(),
            "expected the oversized inbound buffer to be rejected, not grown forever"
        );
    }

    #[test]
    fn record_fails_from_init_state() {
        let mut c = ClientSession::new();
        assert!(c.record("rtsp://h/s").is_err());
    }

    #[test]
    fn record_transitions_to_recording_state() {
        // RTSP lines are CRLF-terminated (RFC 2326 §1); convert "\n" -> CRLF.
        fn wire(s: &str) -> Vec<u8> {
            s.replace('\n', "\r\n").into_bytes()
        }
        let mut c = ClientSession::new();
        // Drive Init -> Ready via a successful SETUP round-trip.
        let t = Transport::single(crate::transport::TransportSpec::rtp_avp_tcp_interleaved(
            0, 1,
        ));
        c.setup("rtsp://h/s", &t).unwrap();
        c.handle_data(&wire(
            "RTSP/1.0 200 OK\nCSeq: 1\nSession: 42\nTransport: RTP/AVP/TCP;interleaved=0-1\n\n",
        ))
        .unwrap();
        assert_eq!(c.state(), SessionState::Ready);

        // RECORD is valid from Ready; the request alone does not move state.
        c.record("rtsp://h/s").unwrap();
        assert_eq!(c.state(), SessionState::Ready);
        // A 2xx response advances Ready -> Recording.
        c.handle_data(&wire("RTSP/1.0 200 OK\nCSeq: 2\nSession: 42\n\n"))
            .unwrap();
        assert_eq!(c.state(), SessionState::Recording);
    }

    // Regression (audit run-09 C1): a 401 auth retry must replay the ORIGINAL
    // request body and Content-Type, not an empty body. Before the fix, the
    // retried ANNOUNCE carried `Content-Length: 0` and no SDP, so an
    // authenticated ANNOUNCE could never reach a real server (RFC 2326 §10.3).
    #[test]
    fn auth_retry_replays_original_body_and_content_type() {
        fn wire(s: &str) -> Vec<u8> {
            s.replace('\n', "\r\n").into_bytes()
        }
        let mut c = ClientSession::new().with_credentials(Credentials::new("admin", "12345"));
        let sdp = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=Test\r\n";
        let first = c.announce("rtsp://h/s", sdp).unwrap();
        assert!(String::from_utf8_lossy(&first).contains("CSeq: 1"));

        let events = c
            .handle_data(&wire(
                "RTSP/1.0 401 Unauthorized\nCSeq: 1\nWWW-Authenticate: Digest realm=\"cam\",nonce=\"abc123\",qop=\"auth\"\n\n",
            ))
            .unwrap();
        let retry = events
            .into_iter()
            .find_map(|e| match e {
                ClientEvent::AuthRetry { request, .. } => Some(request),
                _ => None,
            })
            .expect("expected an AuthRetry event");
        let retry = String::from_utf8_lossy(&retry);

        assert!(retry.contains("ANNOUNCE rtsp://h/s"), "{retry}");
        assert!(retry.contains("Authorization: Digest "), "{retry}");
        assert!(
            retry.contains("Content-Type: application/sdp"),
            "Content-Type dropped on auth retry: {retry}"
        );
        assert!(
            retry.contains(sdp),
            "SDP body dropped on auth retry: {retry}"
        );
        let expected_len = format!("Content-Length: {}", sdp.len());
        assert!(
            retry.contains(&expected_len),
            "expected {expected_len:?} in: {retry}"
        );
    }

    // Regression (audit run-09 W4): a server that rotates its Digest nonce
    // without `stale=true` must still get a correctly-signed retry on the
    // very next request, not one signed with the stale nonce. Before the fix,
    // the authenticator was rebuilt only when it was `None`, `already`
    // retried, or `stale` was set — none of which hold for a fresh request's
    // first 401 when the client had already authenticated earlier in the
    // session — so the retry reused the old nonce, failed again, and the
    // engine gave up (`already && !stale`) instead of adapting.
    #[test]
    fn nonce_rotation_without_stale_is_picked_up_on_the_first_retry() {
        fn wire(s: &str) -> Vec<u8> {
            s.replace('\n', "\r\n").into_bytes()
        }
        let mut c = ClientSession::new().with_credentials(Credentials::new("admin", "12345"));

        // Establish the authenticator via an ordinary first challenge.
        c.options("rtsp://h/s").unwrap(); // CSeq 1
        let events = c
            .handle_data(&wire(
                "RTSP/1.0 401 Unauthorized\nCSeq: 1\nWWW-Authenticate: Digest realm=\"cam\",nonce=\"nonce-1\",qop=\"auth\"\n\n",
            ))
            .unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ClientEvent::AuthRetry { cseq: 2, .. })),
            "{events:?}"
        );
        c.handle_data(&wire("RTSP/1.0 200 OK\nCSeq: 2\n\n"))
            .unwrap();

        // A later, unrelated request (e.g. a GET_PARAMETER keepalive) hits a
        // rotated nonce with no `stale=true`.
        c.options("rtsp://h/s").unwrap(); // CSeq 3
        let events = c
            .handle_data(&wire(
                "RTSP/1.0 401 Unauthorized\nCSeq: 3\nWWW-Authenticate: Digest realm=\"cam\",nonce=\"nonce-2\",qop=\"auth\"\n\n",
            ))
            .unwrap();
        let retry = events
            .into_iter()
            .find_map(|e| match e {
                ClientEvent::AuthRetry {
                    cseq: 4, request, ..
                } => Some(request),
                _ => None,
            })
            .expect("expected a retry (CSeq 4), not a give-up");
        let retry = String::from_utf8_lossy(&retry);
        assert!(
            retry.contains("nonce=\"nonce-2\""),
            "retry signed with the stale nonce instead of the rotated one: {retry}"
        );
    }

    fn established(timeout: Option<u64>) -> ClientSession {
        let mut c = ClientSession::new();
        let _ = c
            .setup(
                "rtsp://h/s",
                &Transport::single(crate::transport::TransportSpec::rtp_avp_tcp_interleaved(
                    0, 1,
                )),
            )
            .unwrap();
        let sess = match timeout {
            Some(t) => format!("abc;timeout={t}"),
            None => "abc".into(),
        };
        let resp = format!(
            "RTSP/1.0 200 OK\r\nCSeq: 1\r\nSession: {sess}\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n"
        );
        c.handle_data(resp.as_bytes()).unwrap();
        c
    }

    #[test]
    fn the_keepalive_interval_is_floored_and_fires_at_most_once_per_interval() {
        // timeout=1 would give a 0.5 s half-timeout; the floor makes it 1 s.
        let t0 = Instant::now();
        let mut c = established(Some(1));
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), Some(t0 + MIN_KEEPALIVE_INTERVAL));
        assert!(
            c.handle_timeout(t0 + Duration::from_millis(500))
                .unwrap()
                .is_none()
        );
        // hammer one instant: exactly one keepalive per floor interval
        let mut sent = 0;
        let tick = t0 + MIN_KEEPALIVE_INTERVAL;
        for _ in 0..1000 {
            if c.handle_timeout(tick).unwrap().is_some() {
                sent += 1;
            }
        }
        assert_eq!(sent, 1, "keepalive must not busy-loop");
    }

    #[test]
    fn a_zero_timeout_is_a_warning_and_the_default_not_a_busy_loop() {
        let t0 = Instant::now();
        let mut c = ClientSession::new();
        let _ = c
            .setup(
                "rtsp://h/s",
                &Transport::single(crate::transport::TransportSpec::rtp_avp_tcp_interleaved(
                    0, 1,
                )),
            )
            .unwrap();
        c.handle_data(
            b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nSession: abc;timeout=0\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .unwrap();
        assert_eq!(c.session_warnings().len(), 1, "{:?}", c.session_warnings());
        assert!(c.session_warnings()[0].contains("timeout"));
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), Some(t0 + Duration::from_secs(30)));
    }

    /// Interop (review round 4): every received session id is stored and echoed back
    /// verbatim on the next request, as before the own-parser change.
    #[test]
    fn odd_received_session_ids_are_echoed_verbatim() {
        for (hdr, id) in [
            ("\"weird\"", "\"weird\""),
            ("a b;timeout=30", "a b"),
            ("abc,def", "abc,def"),
            ("ab\"c", "ab\"c"),
        ] {
            let mut c = ClientSession::new();
            let _ = c
                .setup(
                    "rtsp://h/s",
                    &Transport::single(crate::transport::TransportSpec::rtp_avp_tcp_interleaved(
                        0, 1,
                    )),
                )
                .unwrap();
            let resp = format!(
                "RTSP/1.0 200 OK\r\nCSeq: 1\r\nSession: {hdr}\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n"
            );
            c.handle_data(resp.as_bytes()).unwrap();
            assert_eq!(c.session_id(), Some(id), "{hdr}");
            let play = String::from_utf8(c.play("rtsp://h/s").unwrap()).unwrap();
            assert!(
                play.contains(&format!("Session: {id}\r\n")),
                "{hdr}: {play}"
            );
        }
    }

    #[test]
    fn session_warnings_are_capped_but_counted() {
        let mut c = ClientSession::new();
        for i in 1..=(MAX_SESSION_WARNINGS as u32 + 10) {
            let _ = c.options("rtsp://h/s").unwrap();
            c.handle_data(
                format!("RTSP/1.0 200 OK\r\nCSeq: {i}\r\nSession: abc;timeout=x\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        }
        assert_eq!(c.session_warnings().len(), MAX_SESSION_WARNINGS);
        assert_eq!(c.session_warning_count(), MAX_SESSION_WARNINGS + 10);
    }

    #[test]
    fn an_unusable_session_header_is_observable_not_silent() {
        let mut c = ClientSession::new();
        let _ = c.options("rtsp://h/s").unwrap();
        c.handle_data(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nSession: ;timeout=5\r\n\r\n")
            .unwrap();
        assert!(c.session_id().is_none());
        assert_eq!(c.session_warnings().len(), 1);
    }

    #[test]
    fn keepalive_is_due_at_half_the_session_timeout() {
        let t0 = Instant::now();
        let mut c = established(Some(20));
        assert_eq!(c.poll_timeout(), None, "no activity recorded yet");
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), Some(t0 + Duration::from_secs(10)));
        assert!(
            c.handle_timeout(t0 + Duration::from_secs(9))
                .unwrap()
                .is_none()
        );
        let req = c
            .handle_timeout(t0 + Duration::from_secs(10))
            .unwrap()
            .expect("keepalive due");
        let text = String::from_utf8(req).unwrap();
        assert!(
            text.starts_with("GET_PARAMETER rtsp://h/s RTSP/1.0"),
            "{text}"
        );
        assert!(text.contains("Session: abc"), "{text}");
        // re-armed from the moment it was sent
        assert_eq!(
            c.poll_timeout(),
            Some(t0 + Duration::from_secs(10) + Duration::from_secs(10))
        );
    }

    #[test]
    fn keepalive_uses_the_rfc_default_timeout_when_none_was_declared() {
        let t0 = Instant::now();
        let mut c = established(None);
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), Some(t0 + Duration::from_secs(30))); // 60 s / 2
    }

    #[test]
    fn no_keepalive_before_setup_or_after_teardown() {
        let t0 = Instant::now();
        let mut c = ClientSession::new();
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), None);
        assert!(
            c.handle_timeout(t0 + Duration::from_secs(3600))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn head_cap_holds_for_a_70_kib_run_of_a_and_for_9000_header_lines() {
        let mut c = ClientSession::new();
        assert!(matches!(
            c.handle_data(&vec![b'A'; 70 * 1024]),
            Err(Error::MessageParse(_))
        ));
        let mut c = ClientSession::new();
        let mut junk = b"RTSP/1.0 200 OK\r\n".to_vec();
        junk.extend(b"X-A: b\r\n".repeat(9000));
        assert!(matches!(c.handle_data(&junk), Err(Error::MessageParse(_))));
    }

    #[test]
    fn a_response_fed_one_byte_at_a_time_is_still_decoded() {
        let mut c = ClientSession::new();
        let _ = c.options("rtsp://h/s").unwrap();
        let resp = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: OPTIONS\r\n\r\n";
        let mut got = 0;
        for b in resp {
            got += c.handle_data(&[*b]).unwrap().len();
        }
        assert_eq!(got, 1);
    }

    #[test]
    fn an_unterminated_header_over_the_head_cap_is_rejected() {
        let mut c = ClientSession::new();
        let mut junk = b"RTSP/1.0 200 OK\r\nX-Pad: ".to_vec();
        junk.extend(std::iter::repeat_n(b'a', 70 * 1024)); // no terminator, ever
        let err = c
            .handle_data(&junk)
            .expect_err("header phase is capped at 64 KiB");
        assert!(matches!(err, Error::MessageParse(_)), "{err:?}");
    }

    #[test]
    fn stale_quoted_rechallenge_refreshes_nonce_after_a_prior_retry() {
        fn wire(s: &str) -> Vec<u8> {
            s.replace('\n', "\r\n").into_bytes()
        }
        let mut c = ClientSession::new().with_credentials(Credentials::new("admin", "12345"));
        c.options("rtsp://h/s").unwrap(); // CSeq 1

        // First 401 (no stale) -> normal retry at CSeq 2, marked retried.
        c.handle_data(&wire(
            "RTSP/1.0 401 Unauthorized\nCSeq: 1\nWWW-Authenticate: Digest realm=\"cam\",nonce=\"nonce-1\",qop=\"auth\"\n\n",
        )).unwrap();

        // Re-challenge with a quoted `stale="True"` and a fresh nonce: the
        // retried request must be re-signed with the NEW nonce, not given up.
        let events = c
            .handle_data(&wire(
                "RTSP/1.0 401 Unauthorized\nCSeq: 2\nWWW-Authenticate: Digest realm=\"cam\",nonce=\"nonce-2\",qop=\"auth\",stale=\"True\"\n\n",
            ))
            .unwrap();
        let retry = events
            .into_iter()
            .find_map(|e| match e {
                ClientEvent::AuthRetry {
                    cseq: 3, request, ..
                } => Some(request),
                _ => None,
            })
            .expect("a quoted stale=true re-challenge must trigger a retry");
        let retry = String::from_utf8_lossy(&retry);
        assert!(
            retry.contains("nonce=\"nonce-2\""),
            "quoted stale=\"True\" was not detected: {retry}"
        );
    }

    // Security-blocker regression (pre-release audit): `ClientSession`
    // derives `Debug` and embeds `Option<Credentials>` directly — it must
    // inherit `Credentials`'s redacting `Debug`, never the raw secret.
    #[test]
    fn client_session_debug_does_not_leak_embedded_credentials_secret() {
        let c = ClientSession::new()
            .with_credentials(Credentials::new("admin", "extremely-secret-password"));
        let debug = format!("{c:?}");
        assert!(
            !debug.contains("extremely-secret-password"),
            "leaked via ClientSession Debug: {debug}"
        );
        assert!(debug.contains("***"), "expected redaction marker: {debug}");
    }
}
