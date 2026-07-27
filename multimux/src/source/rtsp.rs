//! RTSP ingest source (ported onto the media-plane ingress traits at plan
//! step 5a) — DESCRIBE/SETUP/PLAY over interleaved TCP, depayloaded into
//! timed [`Sample`]s.
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
//! - **`rtsps://` (TLS) in [`run_rtsp`].** The session/`Stage` logic is
//!   completely transport-agnostic (it only ever sees bytes), but the
//!   driver loop below only wires a plain `TcpStream` for this story's time
//!   budget; wiring `tokio_rustls` back in is a small, mechanical addition
//!   to `run_rtsp` alone (the pre-5a `RtspClient` enum's `Plain`/`Tls` split
//!   is the template), not a design gap.

use std::collections::VecDeque;

use bytes::Bytes;
use rtsp_runtime::auth::Credentials;
use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::transport::{Transport, TransportSpec};
use rtsp_runtime::{Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    if channel % 2 != 0 {
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

/// Binds a TCP connection to `route` and drives an [`RtspIngestSession`]
/// through [`IngestDriver`] until the connection closes or fails — the new
/// drive loop, replacing the pre-5a `RtspSource::connect`/`RtspSession::next_samples`
/// pair. **Plain `rtsp://` only** in this port (see the module doc's "what
/// did not carry over").
pub async fn run_rtsp(
    route: &RtspRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
) -> MultimuxError {
    let mut dialer = RtspDialer::new(route.url.clone(), route.auth.clone());
    if matches!(dialer.is_tls(), Ok(true)) {
        return MultimuxError::Connect {
            reason: "rtsps:// (TLS) is not wired into run_rtsp in this port (step 5a scope cut)"
                .into(),
        };
    }
    let addr = match dialer.connect_addr() {
        Ok(a) => a,
        Err(e) => return e,
    };
    let connect_timeout = route.timeouts.connect;
    let stream = match tokio::time::timeout(connect_timeout, TcpStream::connect(&addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return MultimuxError::Connect {
                reason: format!("rtsp connect {addr}: {e}"),
            };
        }
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
    let mut driver = IngestDriver::new(session, trunk_config, handshake);
    let (mut rd, mut wr) = stream.into_split();
    let start = std::time::Instant::now();
    let mut buf = vec![0u8; 64 * 1024];
    let read_timeout = route.timeouts.read;

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
    }
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
    fn connect_addr_rejects_non_rtsp_scheme() {
        let base = Url::parse("http://cam.local/stream").unwrap();
        assert!(connect_addr(&base).is_err());
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
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
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
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
        let _describe = driver.poll_transmit().unwrap();
        let not_found = rtsp_response(1, "404 Not Found", "", "");
        driver.feed(&not_found, Timestamp::ZERO);
        assert!(
            matches!(driver.health(), HealthState::Failed(_)),
            "a non-success DESCRIBE must fail the session: {:?}",
            driver.health()
        );
    }
}
