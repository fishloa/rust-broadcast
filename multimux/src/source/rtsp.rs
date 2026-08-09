//! RTSP ingest source (ported onto the media-plane ingress traits at plan
//! step 5a) — DESCRIBE/SETUP/PLAY over interleaved TCP, depayloaded into
//! timed [`transmux::Sample`]s.
//!
//! # The sans-IO reshape, for real RTSP — this module is the test of it
//!
//! [`RtspDialer::dial`] performs **no I/O**: it parses the URL, resolves
//! credentials, and builds exactly one thing — the DESCRIBE request bytes —
//! via [`rtsp_runtime::client::ClientSession`], the sans-IO RTSP engine
//! `rtsp-runtime` already ships (its request builders *return* the bytes to
//! send; `handle_data` consumes replies and returns typed events; the RFC
//! 2326 Appendix A.1 state machine lives inside it, never touching a
//! socket). [`RtspIngestSession`] wraps that engine directly — no
//! `AsyncRtspClient`, no owned `TcpStream` — so the *entire* DESCRIBE → SETUP
//! (× N tracks) → PLAY handshake completes through the ordinary
//! [`IngestSession::poll_transmit`]/[`Stage::feed`] pump: [`run_rtsp`] (the
//! multimux-side driver that owns the real socket) writes whatever
//! `poll_transmit` returns, reads the reply, and hands it to `feed`, which
//! either queues the next request or emits [`SessionEvent::Established`].
//!
//! This confirms `media_plane::ingress`'s central design bet: a "genuinely
//! multi-round-trip" protocol does **not** need a blocking `dial()` or an
//! executor-bridge thread, *provided* a sans-IO engine already exists to
//! delegate to. `rtsp-runtime` is that engine for RTSP. (Contrast
//! [`crate::source::ts_http`]/[`crate::source::srt`]'s caller mode, which
//! have no such sans-IO core and therefore *do* need the bridge — see their
//! own module docs.)
//!
//! # What did not carry over
//!
//! - **Keepalive / RTCP receiver reports.** The pre-5a session never sent
//!   either (it only ever read), so this port carries no regression, but
//!   [`IngestSession::poll_transmit`] is exactly the seam
//!   `media_plane::ingress`'s own docs name for wiring one in later
//!   (a periodic `OPTIONS`, driven off [`Stage::on_deadline`]) — not done
//!   here, since nothing pre-5a did it either.
//! # `rtsps://` (TLS)
//!
//! The session/`Stage` logic above is completely transport-agnostic (it only
//! ever sees bytes), so [`run_rtsp`] wires TLS in at the socket layer alone:
//! for an `rtsps://` route it TCP-connects then performs a `tokio_rustls`
//! handshake (trusting the public-CA `webpki-roots` bundle via
//! `rtsp_runtime::io::default_tls_client_config`, exactly like
//! [`rtsp_runtime::io::AsyncRtspClient::connect_tls_with`] does for
//! `rtsp-runtime`'s own client), producing the same `AsyncRead + AsyncWrite`
//! duplex stream a plain `rtsp://` connect would — the sans-IO driver loop
//! never has to know which one it got. Gated behind this crate's `tls`
//! feature (default-on); with it disabled, an `rtsps://` route fails fast
//! with a clear runtime error naming the missing feature, never falling back
//! to an unencrypted socket (issue #804).

use std::collections::VecDeque;

use bytes::Bytes;
use rtsp_runtime::auth::Credentials;
use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::transport::{Transport, TransportSpec};
use rtsp_runtime::{Method, StatusCode};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::resolve_credentials;
use crate::source::{IngestTimeouts, Source, TrackInit, sdp::parse_sdp_tracks};

/// Interleaved channel offset from a media's RTP channel to its paired RTCP
/// channel (RFC 2326 §10.12: `interleaved=lo-hi` with `hi = lo + 1`).
const RTCP_CHANNEL_OFFSET: u8 = 1;

/// Default port for a bare `rtsp://host/...` URL with no explicit port.
const RTSP_DEFAULT_PORT: u16 = rtsp_runtime::RTSP_DEFAULT_PORT;

/// Default port for a bare `rtsps://host/...` URL with no explicit port.
const RTSPS_DEFAULT_PORT: u16 = rtsp_runtime::RTSPS_DEFAULT_PORT;

/// Map an interleaved RTP channel to its track id (even channels only; RTCP
/// odd channels return `None`).
pub fn route_channel(channel: u8, tracks: &[TrackInit]) -> Option<u32> {
    if !channel.is_multiple_of(2) {
        return None;
    }
    tracks
        .iter()
        .find(|t| t.channel == channel)
        .map(|t| t.track_id)
}

/// An RTSP route: a name (for logging/metrics) plus its source URL. Replaces
/// the old (pre-5a) `RtspSource`; [`run_rtsp`] is the new drive loop.
#[derive(Clone)]
pub struct RtspRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    auth: Option<Credentials>,
}

impl std::fmt::Debug for RtspRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl RtspRoute {
    /// Build a route descriptor.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        RtspRoute {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }
}

impl Source for RtspRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Which handshake request [`RtspIngestSession`] is currently waiting a
/// response for — the state `poll_transmit`/`feed` walk through instead of
/// `RtspDialer::dial` blocking on it.
enum Phase {
    AwaitDescribe,
    AwaitSetup(usize),
    AwaitPlay,
    Live,
}

/// A live RTSP [`IngestSession`]: wraps `rtsp-runtime`'s sans-IO
/// [`ClientSession`] directly (no socket) — see the module doc.
pub struct RtspIngestSession {
    session: ClientSession,
    base_url: Url,
    request_uri: String,
    tracks: Vec<TrackInit>,
    depacketiser: Option<transmux::RtpStreamDepacketiser>,
    phase: Phase,
    outbound: VecDeque<Bytes>,
    pending: VecDeque<SessionEvent>,
}

impl RtspIngestSession {
    fn begin_setup(&mut self, index: usize) -> Result<()> {
        let track = &self.tracks[index];
        let uri = resolve_control(&self.base_url, track.control.as_deref())?;
        let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(
            track.channel,
            track.channel.saturating_add(RTCP_CHANNEL_OFFSET),
        ));
        let bytes = self
            .session
            .setup(&uri, &transport)
            .map_err(protocol_err("SETUP"))?;
        self.outbound.push_back(Bytes::from(bytes));
        self.phase = Phase::AwaitSetup(index);
        Ok(())
    }

    fn handle_event(&mut self, event: ClientEvent) -> Result<()> {
        match event {
            ClientEvent::AuthRetry { request, .. } => {
                self.outbound.push_back(Bytes::from(request));
            }
            ClientEvent::Response {
                status,
                body,
                method,
                ..
            } => {
                if !status.is_success() {
                    return Err(response_error(&method, status));
                }
                match (&self.phase, &method) {
                    (Phase::AwaitDescribe, m) if *m == Method::Describe => {
                        let tracks = parse_sdp_tracks(&body)?;
                        self.tracks = tracks;
                        if self.tracks.is_empty() {
                            return Err(MultimuxError::Sdp {
                                reason: "DESCRIBE: SDP declared no media".into(),
                            });
                        }
                        self.begin_setup(0)?;
                    }
                    (Phase::AwaitSetup(index), m) if *m == Method::Setup => {
                        let index = *index;
                        let spec = self
                            .session
                            .negotiated_transport()
                            .and_then(Transport::first)
                            .ok_or_else(|| MultimuxError::Protocol {
                                phase: "SETUP",
                                reason: format!(
                                    "track {}: server did not provide negotiated transport",
                                    self.tracks[index].track_id
                                ),
                            })?;
                        let channel =
                            interleaved_channel(spec).ok_or_else(|| MultimuxError::Protocol {
                                phase: "SETUP",
                                reason: format!(
                                    "track {}: server did not negotiate interleaved TCP transport",
                                    self.tracks[index].track_id
                                ),
                            })?;
                        self.tracks[index].channel = channel;
                        let next = index + 1;
                        if next < self.tracks.len() {
                            self.begin_setup(next)?;
                        } else {
                            self.depacketiser = Some(transmux::RtpStreamDepacketiser::new(
                                self.tracks
                                    .iter()
                                    .map(|t| {
                                        transmux::RtpStreamTrack::new(
                                            t.track_id,
                                            t.kind,
                                            t.config.clone(),
                                            t.clock_rate,
                                        )
                                    })
                                    .collect(),
                            ));
                            let bytes = self
                                .session
                                .play(&self.request_uri)
                                .map_err(protocol_err("PLAY"))?;
                            self.outbound.push_back(Bytes::from(bytes));
                            self.phase = Phase::AwaitPlay;
                        }
                    }
                    (Phase::AwaitPlay, m) if *m == Method::Play => {
                        self.pending.push_back(SessionEvent::Established);
                        let specs = self
                            .depacketiser
                            .as_ref()
                            .expect("depacketiser built before PLAY is sent")
                            .track_specs();
                        self.pending.push_back(SessionEvent::NewProgram {
                            program: ProgramId(0),
                            tracks: specs,
                        });
                        self.phase = Phase::Live;
                    }
                    _ => {
                        // A response for a request phase we've already moved
                        // past (e.g. a stray retransmit) — ignore rather than
                        // error, mirroring how `handle_data` already
                        // correlates by CSeq.
                    }
                }
            }
            ClientEvent::MediaData { channel, data } => {
                if !matches!(self.phase, Phase::Live) {
                    return Ok(());
                }
                let Some(track_id) = route_channel(channel, &self.tracks) else {
                    return Ok(());
                };
                let Some(depa) = self.depacketiser.as_mut() else {
                    return Ok(());
                };
                let samples = depa
                    .push(track_id, &data)
                    .map_err(|e| MultimuxError::Depay {
                        reason: e.to_string(),
                    })?;
                for sample in samples {
                    self.pending.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
            // `ClientEvent` is `#[non_exhaustive]`: a future variant this
            // session has no reaction to yet is ignored, not a hard error.
            _ => {}
        }
        Ok(())
    }
}

impl Stage for RtspIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<()> {
        let events = self
            .session
            .handle_data(input)
            .map_err(protocol_err("recv"))?;
        for event in events {
            self.handle_event(event)?;
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(4096)
    }
}

impl IngestSession for RtspIngestSession {
    type Request = Bytes;

    fn poll_transmit(&mut self) -> Option<Bytes> {
        self.outbound.pop_front()
    }
}

/// Constructs an [`RtspIngestSession`] and queues its first handshake
/// request (DESCRIBE) — **no I/O**: see the module doc.
pub struct RtspDialer {
    url: String,
    auth: Option<Credentials>,
}

impl RtspDialer {
    /// Build a dialer for `url` (`rtsp://`/`rtsps://`), with optional
    /// config-supplied credentials taking precedence over URL userinfo.
    pub fn new(url: impl Into<String>, auth: Option<Credentials>) -> Self {
        RtspDialer {
            url: url.into(),
            auth,
        }
    }

    /// The `host:port` [`run_rtsp`] must TCP-connect to — computed here
    /// (pure, no I/O) so the driver never needs to re-parse the URL.
    pub fn connect_addr(&self) -> Result<String> {
        let base = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad rtsp(s) URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let clean = strip_userinfo(&base)?;
        connect_addr(&clean)
    }

    /// Whether `run_rtsp` needs a TLS-wrapped connection (`rtsps://`).
    pub fn is_tls(&self) -> Result<bool> {
        let base = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad rtsp(s) URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        scheme_is_tls(&strip_userinfo(&base)?)
    }

    /// The SNI server name `run_rtsp` presents during an `rtsps://` TLS
    /// handshake: this URL's host with IPv6 brackets stripped
    /// (`[2001:db8::1]` -> `2001:db8::1`, which is what rustls'
    /// `ServerName::try_from` accepts), hostnames and IPv4 literals
    /// unchanged. Userinfo is stripped first, like its `connect_addr`/
    /// `is_tls` siblings.
    pub fn sni_server_name(&self) -> Result<String> {
        let base = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad rtsp(s) URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        sni_server_name(&strip_userinfo(&base)?)
    }
}

impl Dialer for RtspDialer {
    type Session = RtspIngestSession;
    type Error = MultimuxError;

    fn dial(&mut self) -> Result<RtspIngestSession> {
        let base_url = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad rtsp(s) URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let credentials = resolve_credentials(self.auth.clone(), extract_credentials(&base_url)?);
        let request_url = strip_userinfo(&base_url)?;
        let request_uri = request_url.to_string();

        let mut session = session_with_credentials(credentials);
        let describe_bytes = session
            .describe(&request_uri)
            .map_err(protocol_err("DESCRIBE"))?;

        Ok(RtspIngestSession {
            session,
            base_url: request_url,
            request_uri,
            tracks: Vec::new(),
            depacketiser: None,
            phase: Phase::AwaitDescribe,
            outbound: VecDeque::from(vec![Bytes::from(describe_bytes)]),
            pending: VecDeque::new(),
        })
    }
}

/// Builds a fresh [`ClientSession`], attaching `credentials` if present.
fn session_with_credentials(credentials: Option<Credentials>) -> ClientSession {
    match credentials {
        Some(creds) => ClientSession::new().with_credentials(creds),
        None => ClientSession::new(),
    }
}

/// A non-success response status becomes the concrete error the driver
/// reports (via `Stage::feed`'s `Err`, i.e. `HealthState::Failed`) — a `401`/
/// `403` maps to [`MultimuxError::Auth`] (matchable), anything else to
/// [`MultimuxError::Protocol`].
fn response_error(method: &Method, status: StatusCode) -> MultimuxError {
    let phase: &'static str = match *method {
        Method::Describe => "DESCRIBE",
        Method::Setup => "SETUP",
        Method::Play => "PLAY",
        _ => "recv",
    };
    if matches!(status, StatusCode::Unauthorized | StatusCode::Forbidden) {
        MultimuxError::Auth {
            reason: format!("{phase}: {status}"),
        }
    } else {
        MultimuxError::Protocol {
            phase,
            reason: format!("non-success status {status}"),
        }
    }
}

/// Resolves a per-media SETUP URI from the base RTSP URL and its `a=control`
/// value, per RFC 2326 §C.1.1.
fn resolve_control(base_url: &Url, control: Option<&str>) -> Result<String> {
    match control {
        None | Some("*") => Ok(base_url.to_string()),
        Some(c) => base_url
            .join(c)
            .map(|u| u.to_string())
            .map_err(|e| MultimuxError::Sdp {
                reason: format!("bad a=control {c:?}: {e}"),
            }),
    }
}

/// Extracts RTSP [`Credentials`] from a base URL's userinfo, percent-decoding
/// both components.
fn extract_credentials(url: &Url) -> Result<Option<Credentials>> {
    if url.username().is_empty() {
        return Ok(None);
    }
    let username = percent_decode(url.username())?;
    let password = match url.password() {
        Some(p) => percent_decode(p)?,
        None => String::new(),
    };
    Ok(Some(Credentials::new(username, password)))
}

/// Percent-decodes a URL userinfo component to UTF-8. Never echoes `s`: it is
/// (part of) a still percent-encoded credential.
fn percent_decode(s: &str) -> Result<String> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|e| MultimuxError::Auth {
            reason: format!("invalid percent-encoded userinfo: {e}"),
        })
}

/// Returns a copy of `url` with its userinfo removed.
fn strip_userinfo(url: &Url) -> Result<Url> {
    let mut clean = url.clone();
    clean
        .set_username("")
        .map_err(|()| MultimuxError::Connect {
            reason: format!(
                "failed to strip username from rtsp(s) URL {}",
                crate::redact::redact_url(url.as_str())
            ),
        })?;
    clean
        .set_password(None)
        .map_err(|()| MultimuxError::Connect {
            reason: format!(
                "failed to strip password from rtsp(s) URL {}",
                crate::redact::redact_url(url.as_str())
            ),
        })?;
    Ok(clean)
}

/// Derives the `host:port` connect address from the base URL, defaulting to
/// [`RTSP_DEFAULT_PORT`]/[`RTSPS_DEFAULT_PORT`].
fn connect_addr(url: &Url) -> Result<String> {
    let default_port = if scheme_is_tls(url)? {
        RTSPS_DEFAULT_PORT
    } else {
        RTSP_DEFAULT_PORT
    };
    let host = url.host_str().ok_or_else(|| MultimuxError::Connect {
        reason: format!("rtsp(s) URL has no host: {url}"),
    })?;
    let port = url.port().unwrap_or(default_port);
    Ok(format!("{host}:{port}"))
}

/// Maps an `rtsp-runtime` error from a request/response phase into
/// [`MultimuxError::Protocol`].
fn protocol_err(phase: &'static str) -> impl Fn(rtsp_runtime::error::Error) -> MultimuxError {
    move |e| MultimuxError::Protocol {
        phase,
        reason: e.to_string(),
    }
}

/// Extracts the RTP channel from a transport spec if it is TCP-interleaved.
fn interleaved_channel(spec: &TransportSpec) -> Option<u8> {
    use rtsp_runtime::transport::LowerTransport;
    if spec.lower_transport == Some(LowerTransport::Tcp) {
        spec.interleaved.map(|(lo, _hi)| lo)
    } else {
        None
    }
}

/// Decides which connected-client kind a base RTSP URL needs.
fn scheme_is_tls(url: &Url) -> Result<bool> {
    match url.scheme() {
        "rtsp" => Ok(false),
        "rtsps" => Ok(true),
        other => Err(MultimuxError::Connect {
            reason: format!("not an rtsp(s) URL scheme: {other}"),
        }),
    }
}

/// Derives the SNI server name for an `rtsps://` TLS handshake from a base
/// `rtsp(s)://` URL, stripping brackets from IPv6 literals. `Url::host_str()`
/// returns IPv6 addresses in bracketed form (per RFC 3986 authority syntax,
/// e.g. `"[2001:db8::1]"`), but rustls `ServerName::try_from()` rejects the
/// brackets. This function extracts the host and strips a leading `[` and
/// trailing `]` if present, leaving hostnames and IPv4 addresses unchanged.
fn sni_server_name(url: &Url) -> Result<String> {
    // Safe to `Display` `url` directly: every caller passes the already
    // userinfo-stripped URL (see `RtspDialer::sni_server_name`).
    let host = url.host_str().ok_or_else(|| MultimuxError::Connect {
        reason: format!("rtsp(s) URL has no host: {url}"),
    })?;
    let sni = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    Ok(sni.to_string())
}

/// A connected transport's read/write halves, boxed so [`run_rtsp`]'s drive
/// loop can be written once and fed either a plain [`TcpStream`]'s halves or
/// a TLS stream's — see the module doc's "`rtsps://` (TLS)" section.
type BoxedRead = Box<dyn AsyncRead + Send + Unpin>;
type BoxedWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// Connects a plain `rtsp://` (TCP) transport to `addr`.
async fn connect_plain(addr: &str) -> Result<(BoxedRead, BoxedWrite)> {
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("rtsp connect {addr}: {e}"),
        })?;
    let (rd, wr) = stream.into_split();
    Ok((Box::new(rd), Box::new(wr)))
}

/// Connects an `rtsps://` (RTSP-over-TLS) transport to `addr`, verifying the
/// server against `config` and presenting `server_name` for SNI/certificate
/// validation — the same TCP-connect-then-`tokio_rustls`-handshake sequence
/// [`rtsp_runtime::io::AsyncRtspClient::connect_tls_with`] performs, minus
/// the `AsyncRtspClient` wrapper this crate's sans-IO driver loop (see the
/// module doc) has no use for: `run_rtsp` only ever needs the connected
/// duplex stream, split for its own `poll_transmit`/`feed` pump. Split via
/// `tokio::io::split` (a `TlsStream` has no `into_split`, unlike
/// [`TcpStream`]).
#[cfg(feature = "tls")]
async fn connect_tls_with_config(
    addr: &str,
    server_name: &str,
    config: rustls::ClientConfig,
) -> Result<(BoxedRead, BoxedWrite)> {
    let tcp = TcpStream::connect(addr)
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("rtsp connect {addr}: {e}"),
        })?;
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let dns = rustls::pki_types::ServerName::try_from(server_name.to_string()).map_err(|e| {
        MultimuxError::Connect {
            reason: format!("invalid TLS server name {server_name:?}: {e}"),
        }
    })?;
    let stream = connector
        .connect(dns, tcp)
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("rtsp TLS handshake {addr}: {e}"),
        })?;
    let (rd, wr) = tokio::io::split(stream);
    Ok((Box::new(rd), Box::new(wr)))
}

/// Connects an `rtsps://` transport to `addr`, trusting the public-CA
/// `webpki-roots` bundle ([`rtsp_runtime::io::default_tls_client_config`]) —
/// the config every real (non-test) caller uses; see
/// [`connect_tls_with_config`] for a caller-supplied trust store (this
/// module's own TLS loopback tests use one trusting a self-signed fixture
/// cert, since no public CA ever signs a `127.0.0.1` loopback cert).
#[cfg(feature = "tls")]
async fn connect_tls(addr: &str, server_name: &str) -> Result<(BoxedRead, BoxedWrite)> {
    connect_tls_with_config(
        addr,
        server_name,
        rtsp_runtime::io::default_tls_client_config(),
    )
    .await
}

/// Without this crate's `tls` feature, an `rtsps://` route fails fast with a
/// clear runtime error naming the missing feature — it never falls back to
/// an unencrypted socket (issue #804).
#[cfg(not(feature = "tls"))]
async fn connect_tls(addr: &str, _server_name: &str) -> Result<(BoxedRead, BoxedWrite)> {
    Err(MultimuxError::Connect {
        reason: format!(
            "rtsps:// (TLS) requires multimux's `tls` feature; cannot connect to {addr}"
        ),
    })
}

/// Binds a connection (TCP for `rtsp://`, TLS-over-TCP for `rtsps://`) to
/// `route` and drives an [`RtspIngestSession`] through [`IngestDriver`] until
/// the connection closes or fails — the new drive loop, replacing the pre-5a
/// `RtspSource::connect`/`RtspSession::next_samples` pair.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2:
/// after every [`IngestDriver::feed`], `crate::source::report_driver_progress`
/// flips `route_handle` to [`crate::route::HealthState::Live`] the first time
/// this session establishes, and publishes each newly-announced program's
/// `Trunk` into `route_handle`'s registry.
pub async fn run_rtsp(
    route: &RtspRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> MultimuxError {
    let mut dialer = RtspDialer::new(route.url.clone(), route.auth.clone());
    let addr = match dialer.connect_addr() {
        Ok(a) => a,
        Err(e) => return e,
    };
    let is_tls = match dialer.is_tls() {
        Ok(b) => b,
        Err(e) => return e,
    };
    let connect_timeout = route.timeouts.connect;
    let connected = if is_tls {
        let server_name = match dialer.sni_server_name() {
            Ok(s) => s,
            Err(e) => return e,
        };
        tokio::time::timeout(connect_timeout, connect_tls(&addr, &server_name)).await
    } else {
        tokio::time::timeout(connect_timeout, connect_plain(&addr)).await
    };
    let (mut rd, mut wr) = match connected {
        Ok(Ok(streams)) => streams,
        Ok(Err(e)) => return e,
        Err(_) => {
            return MultimuxError::Connect {
                reason: format!("rtsp connect {addr}: no response within {connect_timeout:?}"),
            };
        }
    };
    let session = match dialer.dial() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut driver = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        // Bound the Trunks one session may mint (media-plane #803). Routes here
        // carry a single programme today; the default ceiling covers an MPTS.
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let start = std::time::Instant::now();
    let mut buf = vec![0u8; 64 * 1024];
    let read_timeout = route.timeouts.read;
    let mut progress = crate::source::DriverProgress::new();

    loop {
        while let Some(bytes) = driver.poll_transmit() {
            if let Err(e) = wr.write_all(&bytes).await {
                return MultimuxError::Connect {
                    reason: format!("rtsp write: {e}"),
                };
            }
        }
        if !driver.health().is_running() {
            break;
        }
        let n = match tokio::time::timeout(read_timeout, rd.read(&mut buf)).await {
            Ok(Ok(0)) => {
                driver.finish();
                break;
            }
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                return MultimuxError::Protocol {
                    phase: "recv",
                    reason: e.to_string(),
                };
            }
            Err(_) => {
                return MultimuxError::Protocol {
                    phase: "recv",
                    reason: format!("no data within {read_timeout:?}"),
                };
            }
        };
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        driver.feed(&buf[..n], now);
        crate::source::advance_route(&driver, route_handle, &mut progress);
    }
    // Health is already terminal on every path that broke out of the loop
    // above (handshake timeout, clean socket EOF via `driver.finish()`) —
    // this call's internal terminal-health check flushes every program's
    // trailing buffered partial segment.
    crate::source::advance_route(&driver, route_handle, &mut progress);
    MultimuxError::Connect {
        reason: format!("rtsp: session ended: {:?}", driver.health()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::ingress::HealthState;
    use std::num::NonZeroUsize;
    use transmux::avc_config_from_sprop;
    use transmux::pipeline::CodecConfig;
    use transmux::rtp::RtpMediaKind;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    fn video_track(channel: u8) -> TrackInit {
        let avc = avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        TrackInit {
            track_id: 1,
            kind: RtpMediaKind::H264,
            config: CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
            clock_rate: 90_000,
            control: None,
            channel,
            payload_type: 96,
        }
    }

    #[test]
    fn routes_even_channel_to_track_ignores_rtcp() {
        let tracks = vec![video_track(0)];
        assert_eq!(route_channel(0, &tracks), Some(1));
        assert_eq!(route_channel(1, &tracks), None);
        assert_eq!(route_channel(4, &tracks), None);
    }

    #[test]
    fn routes_second_media_even_channel() {
        let tracks = vec![video_track(0), video_track(2)];
        assert_eq!(route_channel(2, &tracks), Some(1));
        assert_eq!(route_channel(3, &tracks), None);
    }

    #[test]
    fn resolve_control_falls_back_to_base_on_aggregate_or_missing() {
        let base = Url::parse("rtsp://cam/base").unwrap();
        assert_eq!(resolve_control(&base, None).unwrap(), base.to_string());
        assert_eq!(resolve_control(&base, Some("*")).unwrap(), base.to_string());
    }

    #[test]
    fn connect_addr_defaults_port_554() {
        let base = Url::parse("rtsp://cam.local/stream").unwrap();
        assert_eq!(
            connect_addr(&base).unwrap(),
            format!("cam.local:{RTSP_DEFAULT_PORT}")
        );
    }

    #[test]
    fn connect_addr_parses_explicit_port() {
        let base = Url::parse("rtsp://cam.local:8554/stream").unwrap();
        assert_eq!(connect_addr(&base).unwrap(), "cam.local:8554");
    }

    #[test]
    fn connect_addr_handles_ipv6_userinfo_port() {
        let base = Url::parse("rtsp://user:pass@[2001:db8::1]:8554/stream").unwrap();
        assert_eq!(connect_addr(&base).unwrap(), "[2001:db8::1]:8554");
    }

    #[test]
    fn connect_addr_defaults_rtsps_port_322() {
        let base = Url::parse("rtsps://cam.local/stream").unwrap();
        assert_eq!(
            connect_addr(&base).unwrap(),
            format!("cam.local:{RTSPS_DEFAULT_PORT}")
        );
        assert_eq!(connect_addr(&base).unwrap(), "cam.local:322");
    }

    #[test]
    fn connect_addr_rejects_non_rtsp_scheme() {
        let base = Url::parse("http://cam.local/stream").unwrap();
        assert!(connect_addr(&base).is_err());
    }

    #[test]
    fn scheme_is_tls_false_for_rtsp() {
        let base = Url::parse("rtsp://cam.local/stream").unwrap();
        assert!(!scheme_is_tls(&base).unwrap());
    }

    #[test]
    fn scheme_is_tls_true_for_rtsps() {
        let base = Url::parse("rtsps://cam.local/stream").unwrap();
        assert!(scheme_is_tls(&base).unwrap());
    }

    #[test]
    fn scheme_is_tls_rejects_other_scheme() {
        let base = Url::parse("http://cam.local/stream").unwrap();
        assert!(scheme_is_tls(&base).is_err());
    }

    /// MUTATION VERIFIED: changing the `strip_prefix('[')`/`strip_suffix(']')`
    /// pair to only `strip_prefix('[')` (i.e. leaving the trailing `]`) makes
    /// this fail with `assertion `left == right` failed / left:
    /// "2001:db8::1]" / right: "2001:db8::1"` — the exact IPv6-bracket case
    /// this function exists for (see its own doc: rustls `ServerName::try_from`
    /// rejects the brackets).
    #[test]
    fn sni_server_name_strips_ipv6_brackets() {
        let url = Url::parse("rtsps://[2001:db8::1]:8554/stream").unwrap();
        assert_eq!(sni_server_name(&url).unwrap(), "2001:db8::1");
    }

    /// MUTATION VERIFIED: replacing the function body with
    /// `Ok(format!("[{host}]"))` (always bracketing) makes this fail with
    /// `assertion `left == right` failed / left: "[cam.local]" / right:
    /// "cam.local"` — a plain hostname must pass through unchanged.
    #[test]
    fn sni_server_name_hostname_unchanged() {
        let url = Url::parse("rtsps://cam.local/stream").unwrap();
        assert_eq!(sni_server_name(&url).unwrap(), "cam.local");
    }

    /// MUTATION VERIFIED: same mutation as above (always bracketing) makes
    /// this fail with `assertion `left == right` failed / left:
    /// "[192.0.2.4]" / right: "192.0.2.4"` — an IPv4 literal must also pass
    /// through unchanged (only the bracketed-IPv6 form is stripped).
    #[test]
    fn sni_server_name_ipv4_unchanged() {
        let url = Url::parse("rtsps://192.0.2.4:8554/stream").unwrap();
        assert_eq!(sni_server_name(&url).unwrap(), "192.0.2.4");
    }

    /// The `RtspDialer::sni_server_name` method wrapper strips userinfo like
    /// its `connect_addr`/`is_tls` siblings — proven by the fact userinfo
    /// (`user:pass@`) does not appear in, and does not break, the derived
    /// name.
    ///
    /// MUTATION VERIFIED: replacing the method body with
    /// `Ok(base.host_str().unwrap_or_default().to_string())` (returning the
    /// raw host, skipping the free-function `sni_server_name` call this
    /// method exists to delegate to) makes this fail with `assertion `left
    /// == right` failed / left: "[2001:db8::1]" / right: "2001:db8::1"` —
    /// proving the method really does route through the bracket-stripping
    /// logic, not just re-derive the host itself.
    #[test]
    fn rtsp_dialer_sni_server_name_strips_userinfo_and_brackets() {
        let dialer = RtspDialer::new("rtsps://user:pass@[2001:db8::1]:8554/stream", None);
        assert_eq!(dialer.sni_server_name().unwrap(), "2001:db8::1");
    }

    #[test]
    fn interleaved_channel_accepts_tcp_with_channels() {
        use rtsp_runtime::transport::LowerTransport;
        let spec = TransportSpec {
            lower_transport: Some(LowerTransport::Tcp),
            interleaved: Some((0, 1)),
            ..Default::default()
        };
        assert_eq!(interleaved_channel(&spec), Some(0));
    }

    #[test]
    fn interleaved_channel_rejects_udp() {
        use rtsp_runtime::transport::LowerTransport;
        let spec = TransportSpec {
            lower_transport: Some(LowerTransport::Udp),
            interleaved: Some((0, 1)),
            ..Default::default()
        };
        assert_eq!(interleaved_channel(&spec), None);
    }

    #[test]
    fn interleaved_channel_rejects_missing_interleaved() {
        use rtsp_runtime::transport::LowerTransport;
        let spec = TransportSpec {
            lower_transport: Some(LowerTransport::Tcp),
            interleaved: None,
            ..Default::default()
        };
        assert_eq!(interleaved_channel(&spec), None);
    }

    #[test]
    fn rtsp_dialer_debug_redacts_credentials_via_route() {
        let route = RtspRoute::new("cam1", "rtsp://user:secretpass@host/s");
        let debug = format!("{route:?}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");
    }

    /// Biting test: the connect-time error path for a URL that fails
    /// `Url::parse` must not leak the raw credentialed string — this drives
    /// `RtspDialer::connect_addr`'s very first fallible step (before any real
    /// I/O) with a URL malformed enough to fail parsing while still carrying
    /// `user:pass@`. Ported from the pre-5a `RtspSource::connect` version of
    /// this test: the redaction happens in `RtspDialer::connect_addr`/`dial`
    /// now, but it is exactly the same security-relevant property (raw
    /// credentials must never appear in an error string) on the same "bad
    /// URL" input, so it stays a plain sync `#[test]` rather than needing
    /// `#[tokio::test]` — `connect_addr` performs no I/O at all.
    #[test]
    fn connect_bad_url_error_redacts_credentials() {
        // A userinfo-bearing URL with an invalid (space-containing, thus
        // unparsable) host — fails `Url::parse` before any network I/O.
        let dialer = RtspDialer::new("rtsp://user:secretpass@bad host/s", None);
        let msg = match dialer.connect_addr() {
            Ok(_) => panic!("bad host must fail to parse"),
            Err(e) => e.to_string(),
        };
        assert!(!msg.contains("user"), "error leaked username: {msg}");
        assert!(!msg.contains("secretpass"), "error leaked password: {msg}");
    }

    // --- The central test: real RTSP DESCRIBE/SETUP/PLAY, entirely through
    // poll_transmit()/feed() — no socket, no executor bridge. -------------

    /// A minimal, valid single-track SDP body (RFC 4566), the same fixture
    /// the pre-5a RTP/RTSP tests used.
    fn sdp_body() -> String {
        let sprop = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 127.0.0.1\r\n\
             s=-\r\n\
             t=0 0\r\n\
             m=video 0 RTP/AVP 96\r\n\
             a=control:trackID=1\r\n\
             a=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 packetization-mode=1;sprop-parameter-sets={sprop}\r\n"
        )
    }

    /// Builds a real RTSP/1.0 response (RFC 2326 §6): status line + headers
    /// (+ body, with a matching `Content-Length`) — hand-written wire text,
    /// but to the real grammar `rtsp_types::Message::parse` (driven via
    /// `ClientSession::handle_data`) actually parses, not a shortcut.
    fn rtsp_response(cseq: u32, status: &str, extra_headers: &str, body: &str) -> Vec<u8> {
        let content_length = if body.is_empty() {
            String::new()
        } else {
            format!("Content-Length: {}\r\n", body.len())
        };
        format!("RTSP/1.0 {status}\r\nCSeq: {cseq}\r\n{extra_headers}{content_length}\r\n{body}")
            .into_bytes()
    }

    /// The centrepiece test: dial() only constructs (asserted via the exact
    /// DESCRIBE bytes it queues); the whole DESCRIBE → SETUP → PLAY
    /// handshake then completes purely through `poll_transmit`/`feed`, one
    /// request in flight at a time, ending in `SessionEvent::Established` +
    /// a `NewProgram` trunk, and a subsequent interleaved RTP frame produces
    /// a real depayloaded `Sample` in that `Trunk`.
    ///
    /// MUTATION-CHECKED (by inspection of the assertions, each independently
    /// falsifiable):
    /// - swallowing `Established`/`NewProgram` in `handle_event`'s
    ///   `AwaitPlay` arm would fail `driver.health()` staying `Establishing`.
    /// - severing `poll_transmit` (returning `None` always) would fail the
    ///   very first assertion (no DESCRIBE bytes to send) — the no-hidden-IO
    ///   proof `media_plane::ingress`'s own suite uses for the same purpose.
    /// - collapsing SETUP's per-track channel assignment (e.g. hardcoding
    ///   channel 0) would still pass here (single track), but the interleaved
    ///   frame is sent on the *server-negotiated* channel from the SETUP
    ///   response below (channel 4, not the client's proposed 0), so a
    ///   session that ignored the negotiated transport and routed by its own
    ///   proposed channel would fail to route the media at all — proving
    ///   `route_channel` uses what the server actually returned.
    #[tokio::test]
    async fn multi_round_trip_rtsp_handshake_completes_through_feed_and_poll_transmit_only() {
        let mut dialer = RtspDialer::new("rtsp://cam.local/stream", None);
        let session = dialer
            .dial()
            .expect("dial: local construction only, no I/O");
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        assert!(matches!(driver.health(), HealthState::Establishing));

        // 1. DESCRIBE went out; nothing else queued.
        let describe_req = driver
            .poll_transmit()
            .expect("dial() must have queued the DESCRIBE request");
        assert!(
            driver.poll_transmit().is_none(),
            "only one request in flight at a time"
        );
        let describe_text = String::from_utf8(describe_req.to_vec()).unwrap();
        assert!(describe_text.starts_with("DESCRIBE rtsp://cam.local/stream RTSP/1.0\r\n"));
        assert!(describe_text.contains("CSeq: 1\r\n"));

        // 2. Feed a real DESCRIBE response (SDP body) -> SETUP goes out.
        let describe_resp = rtsp_response(
            1,
            "200 OK",
            "Content-Type: application/sdp\r\n",
            &sdp_body(),
        );
        driver.feed(&describe_resp, Timestamp::ZERO);
        assert!(matches!(driver.health(), HealthState::Establishing));
        let setup_req = driver
            .poll_transmit()
            .expect("DESCRIBE response must queue a SETUP request");
        let setup_text = String::from_utf8(setup_req.to_vec()).unwrap();
        assert!(setup_text.starts_with("SETUP rtsp://cam.local/trackID=1 RTSP/1.0\r\n"));
        assert!(setup_text.contains("CSeq: 2\r\n"));
        assert!(setup_text.contains("Transport:"));

        // 3. Feed a real SETUP response, negotiating interleaved channel 4/5
        // (deliberately NOT the client's proposed 0/1) -> PLAY goes out.
        let setup_resp = rtsp_response(
            2,
            "200 OK",
            "Transport: RTP/AVP/TCP;interleaved=4-5\r\nSession: ABC123;timeout=60\r\n",
            "",
        );
        driver.feed(&setup_resp, Timestamp::from_nanos(1));
        assert!(matches!(driver.health(), HealthState::Establishing));
        let play_req = driver
            .poll_transmit()
            .expect("SETUP response must queue a PLAY request");
        let play_text = String::from_utf8(play_req.to_vec()).unwrap();
        assert!(play_text.starts_with("PLAY rtsp://cam.local/stream RTSP/1.0\r\n"));
        assert!(play_text.contains("Session: ABC123"));

        // 4. Feed a real PLAY response -> Established + NewProgram.
        let play_resp = rtsp_response(3, "200 OK", "", "");
        driver.feed(&play_resp, Timestamp::from_nanos(2));
        assert!(
            matches!(driver.health(), HealthState::Live),
            "after the final handshake reply the session must be Live: {:?}",
            driver.health()
        );
        let trunk = driver
            .trunk(ProgramId(0))
            .cloned()
            .expect("PLAY response must announce NewProgram(0)");
        let mut cursor = trunk.subscribe();

        // 5. A real interleaved RTP frame, on the *server-negotiated*
        // channel (4), produces a depayloaded Sample in the Trunk.
        let nal = [0x65u8, 0xAA, 0xBB];
        let mut rtp_pkt = Vec::new();
        rtp_pkt.push(0x80); // V=2
        rtp_pkt.push(0x80 | 96); // marker + PT 96
        rtp_pkt.extend_from_slice(&1u16.to_be_bytes()); // seq
        rtp_pkt.extend_from_slice(&1000u32.to_be_bytes()); // timestamp
        rtp_pkt.extend_from_slice(&0xCAFEBABEu32.to_be_bytes()); // SSRC
        rtp_pkt.extend_from_slice(&nal);
        let frame = rtsp_runtime::interleaved::InterleavedFrame::new(4, rtp_pkt)
            .to_bytes()
            .expect("serialize interleaved frame");
        // A second frame with a later timestamp so the depacketiser closes
        // out the first access unit.
        let nal2 = [0x41u8, 0xCC];
        let mut rtp_pkt2 = Vec::new();
        rtp_pkt2.push(0x80);
        rtp_pkt2.push(96);
        rtp_pkt2.extend_from_slice(&2u16.to_be_bytes());
        rtp_pkt2.extend_from_slice(&4000u32.to_be_bytes());
        rtp_pkt2.extend_from_slice(&0xCAFEBABEu32.to_be_bytes());
        rtp_pkt2.extend_from_slice(&nal2);
        let frame2 = rtsp_runtime::interleaved::InterleavedFrame::new(4, rtp_pkt2)
            .to_bytes()
            .expect("serialize interleaved frame 2");
        // A third frame: the depacketiser needs the *next* AU's timestamp to
        // finalise the previous one's duration (mirrors the pre-5a
        // `rtp_udp`/RTSP tests' 3-packet-for-2-samples shape), so the second
        // AU (nal2) only closes once this one arrives.
        let nal3 = [0x41u8, 0xDD];
        let mut rtp_pkt3 = Vec::new();
        rtp_pkt3.push(0x80);
        rtp_pkt3.push(96);
        rtp_pkt3.extend_from_slice(&3u16.to_be_bytes());
        rtp_pkt3.extend_from_slice(&7000u32.to_be_bytes());
        rtp_pkt3.extend_from_slice(&0xCAFEBABEu32.to_be_bytes());
        rtp_pkt3.extend_from_slice(&nal3);
        let frame3 = rtsp_runtime::interleaved::InterleavedFrame::new(4, rtp_pkt3)
            .to_bytes()
            .expect("serialize interleaved frame 3");

        driver.feed(&frame, Timestamp::from_nanos(3));
        driver.feed(&frame2, Timestamp::from_nanos(4));
        driver.feed(&frame3, Timestamp::from_nanos(5));

        let item = cursor
            .poll()
            .expect("a depayloaded sample must reach the Trunk");
        match item {
            media_plane::trunk::SampleCursorItem::Timed { track_id, sample } => {
                assert_eq!(track_id, 1);
                assert!(sample.flags.is_sync, "the IDR NAL must be marked sync");
            }
            other => panic!("expected Timed, got {other:?}"),
        }
    }

    /// Severing `poll_transmit` (the no-hidden-I/O proof `media_plane::ingress`'s
    /// own suite uses): if `IngestSession::poll_transmit` were never drained,
    /// the DESCRIBE bytes would simply sit unread — this asserts the
    /// dialer's queued request is *only* observable through `poll_transmit`,
    /// never as a side effect of `dial()` or `feed()` alone.
    #[test]
    fn dial_alone_performs_no_io_the_request_only_appears_via_poll_transmit() {
        let mut dialer = RtspDialer::new("rtsp://cam.local/stream", None);
        let mut session = dialer.dial().expect("local construction only");
        // Before poll_transmit is ever called, the session has produced no
        // SessionEvent (no Established, nothing) — dial() truly did nothing
        // beyond construct + queue.
        assert!(session.poll().is_none());
        assert!(session.poll_transmit().is_some());
    }

    /// A non-success DESCRIBE response (e.g. a stalled/misconfigured camera
    /// returning 404) must fail the session (`Stage::feed`'s `Err`), driving
    /// `HealthState::Failed` — not silently hang in `Establishing` forever.
    #[test]
    fn non_success_describe_response_fails_the_session() {
        let mut dialer = RtspDialer::new("rtsp://cam.local/stream", None);
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let _describe = driver.poll_transmit().unwrap();
        let not_found = rtsp_response(1, "404 Not Found", "", "");
        driver.feed(&not_found, Timestamp::ZERO);
        assert!(
            matches!(driver.health(), HealthState::Failed(_)),
            "a non-success DESCRIBE must fail the session: {:?}",
            driver.health()
        );
    }

    // --- issue #804: rtsps:// (TLS) wiring ---------------------------------

    /// Fail-safe property: an `rtsps://` route must never fall back to an
    /// unencrypted socket, even though establishing TLS against this
    /// deliberately-plain listener will fail. Points a real `rtsps://` route
    /// at a plain (non-TLS) loopback listener, through the real
    /// [`run_rtsp`] entry point (not a lower-level helper), and asserts on
    /// what the listener actually received, rather than trusting the code
    /// path: a genuine `rustls` `ClientHello` is opaque binary starting with
    /// TLS record type `0x16`, never the ASCII text of an RTSP request line —
    /// confirmed against the actual bytes captured below, not merely a
    /// keyword search.
    ///
    /// MUTATION VERIFIED: changing `run_rtsp`'s dispatch (`if is_tls {
    /// connect_tls(...) } else { connect_plain(...) }`) to call
    /// `connect_plain(&addr)` unconditionally (reintroducing a scheme-blind
    /// fallback) makes this fail at `assert_eq!(received[0], 0x16, ...)` with
    /// `left: 0x44, right: 0x16` — `0x44` is ASCII `'D'`, the first byte of
    /// the plaintext `DESCRIBE ...` request line the listener then receives
    /// instead of a TLS `ClientHello`.
    #[tokio::test]
    async fn rtsps_route_never_falls_back_to_plain_socket() {
        const TLS_HANDSHAKE_RECORD_TYPE: u8 = 0x16;
        const TLS_MAJOR_VERSION: u8 = 0x03;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 256];
            // HANG GUARD (issue #826): best-effort read to capture what the
            // client sent before giving up. The test's semantic assertions
            // (TLS record type and version bytes) are on the content of
            // whatever was written, not on how fast it arrives; 3s was
            // generous for loopback but under load even a 1-byte write can
            // be delayed by local scheduling. Raised to 60s since this is
            // purely a "did the client write anything at all" check, not a
            // timing claim.
            let n = tokio::time::timeout(std::time::Duration::from_secs(60), sock.read(&mut buf))
                .await
                .unwrap_or(Ok(0))
                .unwrap_or(0);
            buf[..n].to_vec()
        });

        // A short connect timeout on this *route instance* (via the existing
        // public `with_timeouts` builder) so the test doesn't wait out the
        // production `DEFAULT_CONNECT_TIMEOUT` default — this changes no
        // production timeout, only this one test route's own value.
        let route = RtspRoute::new("fail-safe", format!("rtsps://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: std::time::Duration::from_millis(500),
                read: std::time::Duration::from_millis(500),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));

        // HANG GUARD (issue #826): the test's semantic assertion is that
        // the rtsps:// client sends a real TLS ClientHello on the wire
        // (the TLS record-type and version-byte checks below). The TLS
        // handshake against a plaintext listener fails instantly — the
        // client sends its ClientHello, the plain TCP server reads it and
        // closes the connection, and `connect_tls` returns an error
        // instantly on the EOF. Verified: with connect timeout inflated
        // to 30s, the test still passes in <100ms (TLS handshake failure
        // is immediate, not timeout-driven). Raised to 60s for load
        // tolerance since this is not a timing claim — only job is to
        // fail a deadlock rather than hang CI.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            run_rtsp(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("run_rtsp must not hang against a non-TLS listener");
        assert!(
            matches!(result, MultimuxError::Connect { .. }),
            "an rtsps:// connect against a plaintext listener must fail cleanly: {result:?}"
        );

        let received = server.await.expect("server task");
        assert!(
            received.len() >= 2,
            "the client must have sent real TLS handshake bytes before giving up: {received:?}"
        );
        assert_eq!(
            received[0], TLS_HANDSHAKE_RECORD_TYPE,
            "the first byte on the wire must be a TLS handshake record (0x16) — a genuine \
             ClientHello — never the ASCII first byte of a plaintext RTSP request line: \
             {received:?}"
        );
        assert_eq!(
            received[1], TLS_MAJOR_VERSION,
            "the TLS record's major version byte must be 0x03 (TLS 1.x): {received:?}"
        );
    }

    #[cfg(feature = "tls")]
    mod tls_tests {
        //! A genuine `tokio_rustls` loopback TLS server + client, proving
        //! [`super::connect_tls_with_config`] (the exact helper `run_rtsp`'s
        //! `rtsps://` path calls, via [`super::connect_tls`]) performs a real
        //! TLS handshake and carries a real DESCRIBE/SETUP/PLAY/depayload
        //! exchange — the TLS analogue of this file's plain-`rtsp://`
        //! centrepiece test
        //! (`multi_round_trip_rtsp_handshake_completes_through_feed_and_poll_transmit_only`),
        //! and mirroring `multimux/tests/rtsp_ingest.rs`'s real-socket
        //! loopback pattern.
        //!
        //! Uses a caller-supplied trust config (rather than
        //! `default_tls_client_config`'s public-CA `webpki-roots` bundle)
        //! because no public CA ever signs a `127.0.0.1` loopback
        //! certificate — this is exactly the flexibility
        //! `connect_tls_with_config` exists to provide underneath the
        //! fixed-trust-store `connect_tls` wrapper `run_rtsp` actually calls.

        use super::*;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::net::TcpListener;

        /// Self-signed `CN=localhost` cert/key fixture, shared byte-for-byte
        /// with `rtsp-runtime/tests/fixtures/localhost-{cert,key}.der` (see
        /// `multimux/tests/fixtures/PROVENANCE.md`) — the same pair
        /// `rtsp-runtime`'s own `tests/io_loopback.rs::tls_full_session_over_loopback`
        /// uses to run a real TLS handshake over `127.0.0.1` loopback.
        const CERT_DER: &[u8] = include_bytes!("../../tests/fixtures/localhost-cert.der");
        const KEY_DER: &[u8] = include_bytes!("../../tests/fixtures/localhost-key.der");

        /// A hang guard, not a speed assertion (issue #807): every
        /// real-socket step below is wrapped in this so a wiring bug fails
        /// in seconds instead of hanging CI forever. Raised from 10s to 60s
        /// for the same load-tolerance reason (issue #826).
        const HANG_GUARD: Duration = Duration::from_secs(60);

        fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
            // Explicit provider, not the process-global default: another
            // crate in the same build (e.g. `reqwest`'s `aws-lc-rs`) can
            // already have installed one, and the plain `::builder()` then
            // has no unambiguous default and panics — mirrors
            // `rtsp_runtime::io::default_tls_client_config`'s own doc.
            Arc::new(rustls::crypto::aws_lc_rs::default_provider())
        }

        fn server_config() -> rustls::ServerConfig {
            use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
            let certs = vec![CertificateDer::from(CERT_DER.to_vec())];
            let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY_DER.to_vec()));
            rustls::ServerConfig::builder_with_provider(tls_provider())
                .with_safe_default_protocol_versions()
                .expect("aws-lc-rs provider supports the safe default protocol versions")
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .expect("server config")
        }

        /// A client trust store containing *only* the fixture's self-signed
        /// cert — the loopback-test analogue of `default_tls_client_config`'s
        /// public-CA bundle.
        fn client_config_trusting_fixture() -> rustls::ClientConfig {
            use rustls::pki_types::CertificateDer;
            let mut roots = rustls::RootCertStore::empty();
            roots
                .add(CertificateDer::from(CERT_DER.to_vec()))
                .expect("add self-signed root");
            rustls::ClientConfig::builder_with_provider(tls_provider())
                .with_safe_default_protocol_versions()
                .expect("aws-lc-rs provider supports the safe default protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth()
        }

        /// Reads off `sock` until one complete RTSP request (headers only)
        /// is buffered — a synchronization boundary only; this test never
        /// needs to parse the request's contents (the responses below use
        /// fixed `CSeq`s, matching a fresh `ClientSession`'s own monotonic
        /// counter, exactly like this file's centrepiece hand-fed test).
        async fn read_request_boundary<S: AsyncRead + Unpin>(sock: &mut S) {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    return;
                }
                let n = sock.read(&mut chunk).await.expect("read request bytes");
                assert!(n > 0, "peer closed before a full request arrived");
                buf.extend_from_slice(&chunk[..n]);
            }
        }

        async fn write_response<S: AsyncWrite + Unpin>(sock: &mut S, bytes: &[u8]) {
            sock.write_all(bytes).await.expect("write response");
            sock.flush().await.expect("flush response");
        }

        /// Builds a minimal RFC 3550 §5.1 RTP packet carrying one NAL unit
        /// verbatim (RFC 6184 §5.1) — same shape as this file's centrepiece
        /// test's inline packets and `multimux/tests/rtsp_ingest.rs`'s
        /// `rtp_packet` helper.
        fn rtp_packet(seq: u16, timestamp: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
            const PT_H264_DYNAMIC: u8 = 96;
            const SSRC: u32 = 0xCAFE_BABE;
            let mut pkt = Vec::with_capacity(12 + nal.len());
            pkt.push(0x80); // V=2, P=0, X=0, CC=0
            pkt.push(if marker {
                0x80 | PT_H264_DYNAMIC
            } else {
                PT_H264_DYNAMIC
            });
            pkt.extend_from_slice(&seq.to_be_bytes());
            pkt.extend_from_slice(&timestamp.to_be_bytes());
            pkt.extend_from_slice(&SSRC.to_be_bytes());
            pkt.extend_from_slice(nal);
            pkt
        }

        /// Real self-signed-cert loopback TLS server: DESCRIBE (SDP) ->
        /// SETUP (interleaved channel 0-1) -> PLAY -> three interleaved RTP
        /// access units — mirrors `multimux/tests/rtsp_ingest.rs`'s
        /// `serve_one_session`, just over `tokio_rustls` instead of plain
        /// TCP.
        async fn serve_one_tls_session(tcp: TcpStream) {
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config()));
            let mut tls = acceptor.accept(tcp).await.expect("TLS handshake (server)");

            read_request_boundary(&mut tls).await;
            let sdp = sdp_body();
            let describe_resp = format!(
                "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
                sdp.len()
            );
            write_response(&mut tls, describe_resp.as_bytes()).await;

            read_request_boundary(&mut tls).await;
            let setup_resp = "RTSP/1.0 200 OK\r\nCSeq: 2\r\nSession: TLSTEST\r\n\
                               Transport: RTP/AVP/TCP;interleaved=0-1\r\n\r\n";
            write_response(&mut tls, setup_resp.as_bytes()).await;

            read_request_boundary(&mut tls).await;
            let play_resp = "RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: TLSTEST\r\n\r\n";
            write_response(&mut tls, play_resp.as_bytes()).await;

            // AU0 @1000 (IDR), AU1 @4000, AU2 @7000: the depacketiser only
            // knows a sample's duration once the *next* AU's timestamp
            // arrives, so 3 AUs yield exactly 1 completed (IDR) sample —
            // exactly enough for this test's assertion.
            let idr = [0x65u8, 0xAA, 0xBB];
            let non1 = [0x41u8, 0xAA, 0xBB];
            let non2 = [0x41u8, 0xCC, 0xDD];
            let aus: [(u32, &[u8]); 3] = [(1000, &idr), (4000, &non1), (7000, &non2)];
            for (i, (ts, nal)) in aus.into_iter().enumerate() {
                let pkt = rtp_packet(1 + i as u16, ts, true, nal);
                let frame = rtsp_runtime::interleaved::InterleavedFrame::new(0, pkt)
                    .to_bytes()
                    .expect("serialize interleaved frame");
                write_response(&mut tls, &frame).await;
            }
            // Keep the TLS connection open until the client has drained
            // every frame — cleanup, not a synchronization sleep (the
            // client's own assertions, bounded by `HANG_GUARD`, are what
            // actually gate the test).
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        /// The TLS analogue of this module's plain-`rtsp://` centrepiece
        /// test: a genuine `tokio_rustls` handshake against a real
        /// self-signed-cert loopback server, then a real DESCRIBE -> SETUP
        /// -> PLAY -> depayload exchange over the encrypted socket, using
        /// the exact `connect_tls_with_config` helper `run_rtsp`'s
        /// `rtsps://` path calls (via `connect_tls`) — proving TLS is
        /// genuinely negotiated (a plaintext byte stream could never
        /// complete this handshake against a TLS-only server) and that real
        /// media streams over it, not merely that the URL is accepted.
        ///
        /// MUTATION VERIFIED: replacing this test's `connect_tls_with_config`
        /// call with `connect_plain(&addr.to_string())` (skipping the TLS
        /// handshake entirely) does *not* fail at the client's own
        /// `.expect("client TLS handshake")` — a bare TCP connect succeeds
        /// fine, since `connect_plain` never attempts TLS. The failure
        /// surfaces one step later, exactly where a real "no TLS" bug would
        /// actually be caught: `serve_one_tls_session`'s `TlsAcceptor::accept`
        /// panics trying to parse the client's plaintext DESCRIBE bytes as a
        /// TLS record (`Custom { kind: InvalidData, error:
        /// InvalidMessage(InvalidContentType) }`), the server task's
        /// connection drops, and the test's own read loop then fails at
        /// `assert!(n > 0, "peer closed before a sample arrived")`.
        #[tokio::test]
        async fn rtsps_dialer_negotiates_real_tls_and_streams() {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind loopback");
            let addr = listener.local_addr().expect("local addr");

            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.expect("accept");
                serve_one_tls_session(tcp).await;
            });

            let mut dialer = RtspDialer::new(format!("rtsps://{addr}/stream"), None);
            let session = dialer
                .dial()
                .expect("dial: local construction only, no I/O");
            let mut driver = IngestDriver::new(
                session,
                trunk_config(),
                handshake(),
                media_plane::DEFAULT_MAX_PROGRAMS,
            );

            let (mut rd, mut wr) = tokio::time::timeout(
                HANG_GUARD,
                connect_tls_with_config(
                    &addr.to_string(),
                    "localhost",
                    client_config_trusting_fixture(),
                ),
            )
            .await
            .expect("client TLS connect must not hang")
            .expect("client TLS handshake");

            let start = std::time::Instant::now();
            let mut buf = vec![0u8; 64 * 1024];
            let mut cursor = None;
            let mut sample_track: Option<(u32, bool)> = None;

            for _ in 0..32 {
                while let Some(bytes) = driver.poll_transmit() {
                    wr.write_all(&bytes).await.expect("write request over TLS");
                }
                if cursor.is_none()
                    && let Some(trunk) = driver.trunk(ProgramId(0))
                {
                    cursor = Some(trunk.subscribe());
                }
                if let Some(c) = cursor.as_mut() {
                    while let Some(item) = c.poll() {
                        if let media_plane::trunk::SampleCursorItem::Timed { track_id, sample } =
                            item
                        {
                            sample_track = Some((track_id, sample.flags.is_sync));
                        }
                    }
                }
                if sample_track.is_some() {
                    break;
                }
                let n = tokio::time::timeout(HANG_GUARD, rd.read(&mut buf))
                    .await
                    .expect("read over TLS must not hang")
                    .expect("read over TLS");
                assert!(n > 0, "peer closed before a sample arrived");
                let now = Timestamp::from_instant(start, std::time::Instant::now());
                driver.feed(&buf[..n], now);
            }

            assert!(
                matches!(driver.health(), HealthState::Live),
                "DESCRIBE/SETUP/PLAY must complete over the TLS transport: {:?}",
                driver.health()
            );
            let (track_id, is_sync) =
                sample_track.expect("a depayloaded sample must reach the Trunk over TLS");
            assert_eq!(track_id, 1, "the sole video track routes to track id 1");
            assert!(is_sync, "the first access unit was the IDR");

            server.await.expect("server task");
        }
    }
}
