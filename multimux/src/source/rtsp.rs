//! RTSP ingest source — DESCRIBE/SETUP/PLAY over interleaved TCP, depayloaded
//! into timed [`Sample`]s.
//!
//! Drives [`rtsp_runtime::io::AsyncRtspClient`] (RFC 2326 Appendix A client
//! state machine, `rtsp://` (plain TCP) + `rtsps://` (RTSP-over-TLS, gated on
//! this crate's `tls` feature, default-on) + interleaved `$`-framed media per
//! §10.12) and feeds each received RTP packet into a [`RtpStreamDepacketizer`]
//! (RFC 6184 / RFC 3640) built from the DESCRIBE SDP
//! ([`crate::source::sdp::parse_sdp_tracks`]).
//!
//! `RtspSource` is the immutable "how to reach this stream" descriptor;
//! [`RtspSource::connect`] performs the DESCRIBE → SETUP (one per media,
//! interleaved) → PLAY handshake and returns a live [`RtspSession`] that
//! [`RtspSession::next_samples`] pulls one interleaved frame at a time.

use std::time::Duration;

use rtsp_runtime::auth::Credentials;
use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::io::AsyncRtspClient;
use rtsp_runtime::transport::{Transport, TransportSpec};
use tokio::net::TcpStream;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::resolve_credentials;
use crate::source::{IngestTimeouts, Source, TrackInit, sdp::parse_sdp_tracks};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{RtpStreamDepacketizer, RtpStreamTrack};

/// Interleaved channel offset from a media's RTP channel to its paired RTCP
/// channel (RFC 2326 §10.12: `interleaved=lo-hi` with `hi = lo + 1`).
const RTCP_CHANNEL_OFFSET: u8 = 1;

/// Default port for a bare `rtsp://host/...` URL with no explicit port
/// (RFC 2326 §1 / IANA `rtsp`), re-exported by `rtsp-runtime`.
const RTSP_DEFAULT_PORT: u16 = rtsp_runtime::RTSP_DEFAULT_PORT;

/// Default port for a bare `rtsps://host/...` URL with no explicit port
/// (IANA `rtsps`), re-exported by `rtsp-runtime`.
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

/// An RTSP stream to pull: a name (for logging/metrics) plus its source URL.
#[derive(Clone)]
pub struct RtspSource {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo
    /// — see `crate::source::http_auth::resolve_credentials`.
    auth: Option<Credentials>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// camera's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for RtspSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspSource")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl RtspSource {
    /// Build a source descriptor. `url` is the `rtsp://` (or `rtsps://`) URL
    /// used for DESCRIBE and as the base for relative `a=control` SETUP URLs.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        RtspSource {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`] — see
    /// `crate::origin::HttpLimits`/`AppState::with_limits` for the analogous
    /// pattern this mirrors. [`crate::origin::serve`] applies `Config`'s
    /// configured values; callers that only want the defaults (most tests)
    /// keep using [`Self::new`] unchanged.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo at
    /// [`Self::connect`] time — see
    /// `crate::source::http_auth::resolve_credentials`.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }

    /// Connects, DESCRIBEs, SETUPs every media (interleaved TCP, channels
    /// `(2i, 2i+1)` per the SDP's media order), and PLAYs — returning a live
    /// session ready for [`RtspSession::next_samples`].
    ///
    /// The whole handshake (TCP/TLS connect through PLAY) is bounded by
    /// [`IngestTimeouts::connect`] (issue #663 P5, audit-ingest #3): a
    /// stalled/half-open server that accepts the connection but never
    /// replies would otherwise hang this method forever, starving
    /// [`crate::origin::supervisor::supervise`]'s backoff of a chance to
    /// retry — a timed-out handshake instead surfaces as a
    /// [`MultimuxError::Connect`], exactly like any other connect failure.
    pub async fn connect(&self) -> Result<RtspSession> {
        let base_url = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad rtsp(s) URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        // Pull credentials from the URL's userinfo (RFC 3986 §3.2.1), if any,
        // then strip it: RTSP request URIs (DESCRIBE/SETUP/PLAY) must not
        // carry `user:pass@`, and stripping keeps secrets out of logs/request
        // lines. Every subsequent use of the URL — connect address, SNI name,
        // scheme check, and any error message that embeds it — uses
        // `request_url` (userinfo already stripped), never `base_url`.
        let credentials = resolve_credentials(self.auth.clone(), extract_credentials(&base_url)?);
        let request_url = strip_userinfo(&base_url)?;
        let request_uri = request_url.to_string();

        let is_tls = scheme_is_tls(&request_url)?;
        let addr = connect_addr(&request_url)?;
        let connect_timeout = self.timeouts.connect;

        let (tracks, client) = tokio::time::timeout(connect_timeout, async {
            let mut client = if is_tls {
                let server_name = sni_server_name(&request_url)?;
                connect_tls_client(&addr, &server_name, credentials).await?
            } else {
                connect_plain_client(&addr, credentials).await?
            };

            let describe = client
                .describe(&request_uri)
                .await
                .map_err(protocol_err("DESCRIBE"))?;
            let sdp = expect_ok_response(describe, "DESCRIBE")?;
            let mut tracks = parse_sdp_tracks(&sdp)?;

            for track in &mut tracks {
                let uri = resolve_control(&request_url, track.control.as_deref())?;
                let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(
                    track.channel,
                    track.channel.saturating_add(RTCP_CHANNEL_OFFSET),
                ));
                let setup = client
                    .setup(&uri, &transport)
                    .await
                    .map_err(protocol_err("SETUP"))?;
                expect_ok_response(setup, "SETUP")?;

                // The server must negotiate TCP-interleaved transport. Extract
                // the negotiated spec and verify it has both TCP
                // lower-transport and interleaved channels; reject any
                // non-interleaved response (e.g. a server that ignores TCP
                // and negotiates UDP).
                let spec = client
                    .session()
                    .negotiated_transport()
                    .and_then(Transport::first)
                    .ok_or_else(|| MultimuxError::Protocol {
                        phase: "SETUP",
                        reason: format!(
                            "track {}: server did not provide negotiated transport",
                            track.track_id
                        ),
                    })?;

                // Ensure the negotiated transport is TCP-interleaved.
                if let Some(channel) = interleaved_channel(spec) {
                    track.channel = channel;
                } else {
                    return Err(MultimuxError::Protocol {
                        phase: "SETUP",
                        reason: format!(
                            "track {}: server did not negotiate interleaved TCP transport",
                            track.track_id
                        ),
                    });
                }
            }

            let play = client
                .play(&request_uri)
                .await
                .map_err(protocol_err("PLAY"))?;
            expect_ok_response(play, "PLAY")?;

            Ok::<_, MultimuxError>((tracks, client))
        })
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!(
                "rtsp connect: no response within {connect_timeout:?} ({})",
                crate::redact::redact_url(&self.url)
            ),
        })??;

        let depacketizer = RtpStreamDepacketizer::new(
            tracks
                .iter()
                .map(|t| RtpStreamTrack::new(t.track_id, t.kind, t.config.clone(), t.clock_rate))
                .collect(),
        );

        Ok(RtspSession {
            tracks,
            client,
            depacketizer,
            read_timeout: self.timeouts.read,
        })
    }
}

impl Source for RtspSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live, connected RTSP session: PLAYing, pulling interleaved media frames
/// and depayloading them into timed [`Sample`]s.
pub struct RtspSession {
    /// Per-track init derived from the DESCRIBE SDP (channel assignments
    /// reflect whatever SETUP ultimately negotiated).
    pub tracks: Vec<TrackInit>,
    client: RtspClient,
    depacketizer: RtpStreamDepacketizer,
    /// Bound on each [`Self::next_samples`] read — see [`IngestTimeouts::read`].
    read_timeout: Duration,
}

impl RtspSession {
    /// The `TrackSpec`s (timescale = RTP clock rate) for init-segment
    /// construction.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.depacketizer.track_specs()
    }

    /// Pulls one interleaved frame and depayloads it. Returns the samples
    /// emitted (zero or more, paired with their track id), an empty `Vec` for
    /// non-media events or an unrouted (RTCP/unknown) channel, or `Ok(None)`
    /// when the peer has closed the connection.
    ///
    /// Bounded by [`IngestTimeouts::read`] (issue #663 P5, audit-ingest #3):
    /// a server that stops sending interleaved frames mid-session (wedged
    /// firmware, silently dropped link) would otherwise leave this `.await`
    /// pending forever — a read that times out surfaces as a
    /// [`MultimuxError::Protocol`], which [`crate::pipeline::run_pipeline`]
    /// propagates and [`crate::origin::supervisor::supervise`] reconnects on,
    /// same as any other read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let event = tokio::time::timeout(self.read_timeout, self.client.recv_interleaved())
            .await
            .map_err(|_| MultimuxError::Protocol {
                phase: "recv",
                reason: format!("no data within {:?}", self.read_timeout),
            })?
            .map_err(protocol_err("recv"))?;
        let Some(event) = event else {
            return Ok(None);
        };
        let ClientEvent::MediaData { channel, data } = event else {
            return Ok(Some(Vec::new()));
        };
        let Some(track_id) = route_channel(channel, &self.tracks) else {
            return Ok(Some(Vec::new()));
        };
        let samples =
            self.depacketizer
                .push(track_id, &data)
                .map_err(|e| MultimuxError::Depay {
                    reason: e.to_string(),
                })?;
        Ok(Some(samples.into_iter().map(|s| (track_id, s)).collect()))
    }
}

/// Resolves a per-media SETUP URI from the base RTSP URL and its `a=control`
/// value, per RFC 2326 §C.1.1: a missing control, or the aggregate-control
/// token `"*"` (RFC 2326 §C.1), falls back to the base (whole-presentation)
/// URL; any other value — absolute or relative — is resolved against the
/// base URL per RFC 3986 §5 (`Url::join` handles both: an absolute
/// `rtsp(s)://...` reference is returned as-is, a relative one like
/// `trackID=1` replaces the base's last path segment).
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

/// Extracts RTSP [`Credentials`] from a base URL's userinfo (RFC 3986
/// §3.2.1), percent-decoding both the username and password (userinfo
/// components may be percent-encoded, e.g. a literal `@` or `:` in a
/// password). Returns `Ok(None)` when the URL carries no username — the
/// common case, meaning "connect with no auth" exactly as before this URL
/// carried credentials.
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

/// Percent-decodes a URL userinfo component (RFC 3986 §2.1) to UTF-8.
///
/// The error message deliberately does **not** echo `s` — it is (part of) a
/// still percent-encoded credential, and even encoded it must never appear
/// in an error/log.
fn percent_decode(s: &str) -> Result<String> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|e| MultimuxError::Auth {
            reason: format!("invalid percent-encoded userinfo: {e}"),
        })
}

/// Returns a copy of `url` with its userinfo (username/password) removed, so
/// it is safe to use in RTSP request lines (DESCRIBE/SETUP/PLAY must not
/// carry `user:pass@`) and as the base for `a=control` resolution.
///
/// `url` here still carries the userinfo being stripped, so on the (very
/// rare — `url::Url::set_username`/`set_password` only fail for
/// cannot-be-a-base URLs, which an `rtsp(s)://` URL never is) error path the
/// message must redact it rather than `Display`ing `url` verbatim.
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

/// Derives the `host:port` connect address from the base `rtsp://`/`rtsps://`
/// URL, defaulting to [`RTSP_DEFAULT_PORT`] (`rtsp://`) or [`RTSPS_DEFAULT_PORT`]
/// (`rtsps://`) when no port is given (RFC 2326 §1 / IANA). `Url::host_str`
/// already renders IPv6 literals bracketed (the URL's authority component, per
/// RFC 3986 §3.2.2 `IP-literal`), so simply joining `host:port` yields a valid
/// socket-address string — e.g. `[::1]:8554` — for both IPv6 literals and
/// plain hostnames/IPv4 addresses.
fn connect_addr(url: &Url) -> Result<String> {
    let default_port = if scheme_is_tls(url)? {
        RTSPS_DEFAULT_PORT
    } else {
        RTSP_DEFAULT_PORT
    };
    // Safe to `Display` `url` directly here: every caller passes the
    // already userinfo-stripped `request_url` (see `RtspSource::connect`),
    // never the raw credentialed `base_url`.
    let host = url.host_str().ok_or_else(|| MultimuxError::Connect {
        reason: format!("rtsp(s) URL has no host: {url}"),
    })?;
    let port = url.port().unwrap_or(default_port);
    Ok(format!("{host}:{port}"))
}

/// Unwraps a `ClientEvent::Response` with a success status into its body;
/// anything else (non-2xx status, or an unexpected event shape) becomes an
/// error naming which request failed. A `401`/`403` status maps to
/// [`MultimuxError::Auth`] (a distinct, matchable kind from a generic
/// protocol failure); any other non-success status or unexpected event shape
/// maps to [`MultimuxError::Protocol`].
fn expect_ok_response(event: ClientEvent, what: &'static str) -> Result<Vec<u8>> {
    use rtsp_runtime::StatusCode;
    match event {
        ClientEvent::Response { status, body, .. } if status.is_success() => Ok(body),
        ClientEvent::Response {
            status: status @ (StatusCode::Unauthorized | StatusCode::Forbidden),
            ..
        } => Err(MultimuxError::Auth {
            reason: format!("{what}: {status}"),
        }),
        ClientEvent::Response { status, .. } => Err(MultimuxError::Protocol {
            phase: what,
            reason: format!("non-success status {status}"),
        }),
        other => Err(MultimuxError::Protocol {
            phase: what,
            reason: format!("unexpected event {other:?}"),
        }),
    }
}

/// Maps an `rtsp-runtime` error from an RTSP request/response phase
/// (DESCRIBE/SETUP/PLAY/recv) into [`MultimuxError::Protocol`], naming which
/// phase failed.
fn protocol_err(phase: &'static str) -> impl Fn(rtsp_runtime::error::Error) -> MultimuxError {
    move |e| MultimuxError::Protocol {
        phase,
        reason: e.to_string(),
    }
}

/// Maps an `rtsp-runtime` error from the transport-connect step (TCP/TLS)
/// into [`MultimuxError::Connect`].
fn connect_err(e: rtsp_runtime::error::Error) -> MultimuxError {
    MultimuxError::Connect {
        reason: e.to_string(),
    }
}

/// Extracts the RTP channel from a transport spec if it is TCP-interleaved,
/// returns `None` otherwise (e.g. UDP or missing interleaved parameter).
fn interleaved_channel(spec: &TransportSpec) -> Option<u8> {
    use rtsp_runtime::transport::LowerTransport;
    if spec.lower_transport == Some(LowerTransport::Tcp) {
        spec.interleaved.map(|(lo, _hi)| lo)
    } else {
        None
    }
}

/// Decides which connected-client kind a base RTSP URL needs: `Ok(false)` for
/// plain `rtsp://` (TCP), `Ok(true)` for `rtsps://` (RTSP-over-TLS per the
/// `tls` feature), `Err` for any other scheme.
fn scheme_is_tls(url: &Url) -> Result<bool> {
    match url.scheme() {
        "rtsp" => Ok(false),
        "rtsps" => Ok(true),
        other => Err(MultimuxError::Connect {
            reason: format!("not an rtsp(s) URL scheme: {other}"),
        }),
    }
}

/// Derives the SNI server name for TLS handshake from a base `rtsp(s)://` URL,
/// stripping brackets from IPv6 literals. `Url::host_str()` returns IPv6
/// addresses in bracketed form (per RFC 3986 authority syntax, e.g.
/// `"[2001:db8::1]"`), but rustls `ServerName::try_from()` rejects the
/// brackets. This function extracts the host and strips leading `[` and
/// trailing `]` if present, leaving hostnames and IPv4 addresses unchanged.
fn sni_server_name(url: &Url) -> Result<String> {
    // Safe to `Display` `url` directly: called only with the already
    // userinfo-stripped `request_url` (see `RtspSource::connect`).
    let host = url.host_str().ok_or_else(|| MultimuxError::Connect {
        reason: format!("rtsp(s) URL has no host: {url}"),
    })?;
    // Strip brackets from IPv6 literals: "[2001:db8::1]" -> "2001:db8::1".
    // Hostnames and IPv4 are unchanged.
    let sni = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    Ok(sni.to_string())
}

/// A connected RTSP client: either plain TCP (`rtsp://`) or RTSP-over-TLS
/// (`rtsps://`, gated on the `tls` feature). Plain and TLS clients are
/// different concrete `AsyncRtspClient<S>` instantiations (the socket types
/// don't unify), so `RtspSession` holds this enum and forwards the handful of
/// calls it needs to whichever inner client is live.
enum RtspClient {
    Plain(AsyncRtspClient<TcpStream>),
    #[cfg(feature = "tls")]
    Tls(AsyncRtspClient<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl RtspClient {
    async fn describe(&mut self, uri: &str) -> rtsp_runtime::error::Result<ClientEvent> {
        match self {
            RtspClient::Plain(c) => c.describe(uri).await,
            #[cfg(feature = "tls")]
            RtspClient::Tls(c) => c.describe(uri).await,
        }
    }

    async fn setup(
        &mut self,
        uri: &str,
        transport: &Transport,
    ) -> rtsp_runtime::error::Result<ClientEvent> {
        match self {
            RtspClient::Plain(c) => c.setup(uri, transport).await,
            #[cfg(feature = "tls")]
            RtspClient::Tls(c) => c.setup(uri, transport).await,
        }
    }

    async fn play(&mut self, uri: &str) -> rtsp_runtime::error::Result<ClientEvent> {
        match self {
            RtspClient::Plain(c) => c.play(uri).await,
            #[cfg(feature = "tls")]
            RtspClient::Tls(c) => c.play(uri).await,
        }
    }

    async fn recv_interleaved(&mut self) -> rtsp_runtime::error::Result<Option<ClientEvent>> {
        match self {
            RtspClient::Plain(c) => c.recv_interleaved().await,
            #[cfg(feature = "tls")]
            RtspClient::Tls(c) => c.recv_interleaved().await,
        }
    }

    fn session(&self) -> &ClientSession {
        match self {
            RtspClient::Plain(c) => c.session(),
            #[cfg(feature = "tls")]
            RtspClient::Tls(c) => c.session(),
        }
    }
}

/// Connects a plain `rtsp://` (TCP) client to `addr`, attaching `credentials`
/// (if any) so the engine can answer a `401` challenge on DESCRIBE (RFC 2326
/// §14).
async fn connect_plain_client(addr: &str, credentials: Option<Credentials>) -> Result<RtspClient> {
    let session = session_with_credentials(credentials);
    let client = AsyncRtspClient::<TcpStream>::connect_with(addr, session)
        .await
        .map_err(connect_err)?;
    Ok(RtspClient::Plain(client))
}

/// Builds a fresh [`ClientSession`], attaching `credentials` via
/// [`ClientSession::with_credentials`] when present.
fn session_with_credentials(credentials: Option<Credentials>) -> ClientSession {
    match credentials {
        Some(creds) => ClientSession::new().with_credentials(creds),
        None => ClientSession::new(),
    }
}

/// Connects an `rtsps://` (RTSP-over-TLS) client to `addr`, presenting
/// `server_name` for SNI/certificate validation against the public-CA trust
/// store ([`rtsp_runtime::io::default_tls_client_config`]).
///
/// Only available when this crate's `tls` feature is enabled; otherwise
/// returns a [`MultimuxError::Connect`] naming the missing feature (rather
/// than failing to compile), so callers get a clear runtime message if `tls`
/// was deliberately disabled.
#[cfg(feature = "tls")]
async fn connect_tls_client(
    addr: &str,
    server_name: &str,
    credentials: Option<Credentials>,
) -> Result<RtspClient> {
    let config = rtsp_runtime::io::default_tls_client_config();
    let session = session_with_credentials(credentials);
    let client = AsyncRtspClient::<tokio_rustls::client::TlsStream<TcpStream>>::connect_tls_with(
        addr,
        server_name,
        config,
        session,
    )
    .await
    .map_err(connect_err)?;
    Ok(RtspClient::Tls(client))
}

#[cfg(not(feature = "tls"))]
async fn connect_tls_client(
    addr: &str,
    _server_name: &str,
    _credentials: Option<Credentials>,
) -> Result<RtspClient> {
    Err(MultimuxError::Connect {
        reason: format!(
            "rtsps:// (TLS) requires multimux's `tls` feature; cannot connect to {addr}"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::avc_config_from_sprop;
    use transmux::pipeline::CodecConfig;
    use transmux::rtp::RtpMediaKind;

    fn video_track(channel: u8) -> TrackInit {
        // reuse a known-good sprop; any valid avcC works here.
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
    fn sni_server_name_strips_ipv6_brackets() {
        let url = Url::parse("rtsps://[2001:db8::1]:8554/stream").unwrap();
        assert_eq!(sni_server_name(&url).unwrap(), "2001:db8::1");
    }

    #[test]
    fn sni_server_name_hostname_unchanged() {
        let url = Url::parse("rtsps://cam.local/stream").unwrap();
        assert_eq!(sni_server_name(&url).unwrap(), "cam.local");
    }

    #[test]
    fn sni_server_name_ipv4_unchanged() {
        let url = Url::parse("rtsps://192.0.2.4:8554/stream").unwrap();
        assert_eq!(sni_server_name(&url).unwrap(), "192.0.2.4");
    }

    #[test]
    fn routes_even_channel_to_track_ignores_rtcp() {
        let tracks = vec![video_track(0)];
        assert_eq!(route_channel(0, &tracks), Some(1)); // RTP -> track 1
        assert_eq!(route_channel(1, &tracks), None); // RTCP -> ignored
        assert_eq!(route_channel(4, &tracks), None); // unknown
    }

    #[test]
    fn routes_second_media_even_channel() {
        let tracks = vec![video_track(0), video_track(2)];
        assert_eq!(route_channel(2, &tracks), Some(1));
        assert_eq!(route_channel(3, &tracks), None);
    }

    #[test]
    fn setup_uri_absolute_control() {
        let base = Url::parse("rtsp://h/media.sdp").unwrap();
        // Absolute control is an RFC-3986 absolute URI reference, so
        // `Url::join` returns it as-is rather than resolving against base.
        assert_eq!(
            resolve_control(&base, Some("rtsp://h/media.sdp/trackID=2")).unwrap(),
            "rtsp://h/media.sdp/trackID=2"
        );
    }

    #[test]
    fn setup_uri_resolves_relative_control() {
        let base = Url::parse("rtsp://h/media.sdp").unwrap();
        // RFC-3986 resolution replaces the base's last path segment, unlike a
        // naive concat (which would wrongly produce ".../media.sdp/trackID=1").
        assert_eq!(
            resolve_control(&base, Some("trackID=1")).unwrap(),
            base.join("trackID=1").unwrap().to_string()
        );
        assert_eq!(
            resolve_control(&base, Some("trackID=1")).unwrap(),
            "rtsp://h/trackID=1"
        );
    }

    #[test]
    fn setup_uri_missing_or_aggregate_control_falls_back_to_base() {
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
        assert_eq!(connect_addr(&base).unwrap(), "cam.local:554");
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
    fn connect_addr_rejects_non_rtsp_scheme() {
        let base = Url::parse("http://cam.local/stream").unwrap();
        assert!(connect_addr(&base).is_err());
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

    #[test]
    fn rtsp_source_new_and_stream_name() {
        let src = RtspSource::new("cam1", "rtsp://cam.local/stream");
        assert_eq!(src.stream_name(), "cam1");
    }

    /// Biting test: an `RtspSource`'s credential must never appear in its
    /// `Debug` output. Fails immediately if the manual `Debug` impl is
    /// reverted to `#[derive(Debug)]`, which would render `url` (and its
    /// `user:pass@`) verbatim.
    #[test]
    fn rtsp_source_debug_redacts_credentials() {
        let src = RtspSource::new("cam1", "rtsp://user:secretpass@host/s");
        let debug = format!("{src:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");
    }

    /// Biting test: the connect-time error path for a URL that fails
    /// `Url::parse` must not leak the raw credentialed string — this drives
    /// `RtspSource::connect`'s very first fallible step (before any real I/O)
    /// with a URL malformed enough to fail parsing while still carrying
    /// `user:pass@`.
    #[tokio::test]
    async fn connect_bad_url_error_redacts_credentials() {
        // A userinfo-bearing URL with an invalid (space-containing, thus
        // unparsable) host — fails `Url::parse` before any network I/O.
        let src = RtspSource::new("cam1", "rtsp://user:secretpass@bad host/s");
        // Not `.expect_err()`/`.unwrap_err()`: both require `RtspSession`
        // (the `Ok` type) to implement `Debug`, which it doesn't.
        let msg = match src.connect().await {
            Ok(_) => panic!("bad host must fail to parse"),
            Err(e) => e.to_string(),
        };
        assert!(!msg.contains("user"), "error leaked username: {msg}");
        assert!(!msg.contains("secretpass"), "error leaked password: {msg}");
    }

    /// Biting test (issue #663 P5, audit-ingest #3): a server that accepts
    /// the TCP connection but never replies (a wedged/half-open RTSP
    /// server — the exact failure mode a "no timeout anywhere" `RtspSource`
    /// used to hang on forever) must fail `connect()` within the configured
    /// [`IngestTimeouts::connect`], not hang. A short timeout keeps the test
    /// itself fast; the outer `tokio::time::timeout` is a generous test-only
    /// backstop that must never actually fire.
    #[tokio::test]
    async fn connect_times_out_against_a_stalled_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        // Accept the connection and then do nothing at all — never read,
        // never write, never close. Held for the test's duration so the
        // socket doesn't get RST before `connect()` gets a chance to hang on
        // it.
        let _accepted = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            std::future::pending::<()>().await;
            drop(stream);
        });

        let source = RtspSource::new("stalled", format!("rtsp://{addr}/stream")).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_millis(100),
                read: Duration::from_secs(30),
            },
        );

        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect(
                "connect() must return on its own via IngestTimeouts::connect, \
                 not hang until this test's own backstop timeout",
            );
        assert!(
            result.is_err(),
            "a server that never responds must fail connect(), not hang forever"
        );
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

    /// Live smoke test against a real RTSP source: connects, pulls a few
    /// samples, and confirms the depayload pipeline runs end-to-end. Skipped
    /// unless `MULTIMUX_TEST_RTSP` is set (no CI fixture server), and reviewed
    /// by inspection.
    #[ignore]
    #[tokio::test]
    async fn live_rtsp_smoke() {
        let url = std::env::var("MULTIMUX_TEST_RTSP")
            .expect("set MULTIMUX_TEST_RTSP to a live rtsp:// URL to run this test");
        let source = RtspSource::new("live", url);
        let mut session = source.connect().await.expect("connect");
        assert!(!session.track_specs().is_empty());
        for _ in 0..10 {
            match session.next_samples().await.expect("next_samples") {
                Some(samples) if !samples.is_empty() => return,
                Some(_) => continue,
                None => panic!("stream ended before any sample was emitted"),
            }
        }
        panic!("no samples emitted within 10 frames");
    }

    /// Live smoke test against a real `rtsps://` (TLS) source: connects,
    /// pulls a few samples, and confirms the depayload pipeline runs
    /// end-to-end over TLS. Skipped unless `MULTIMUX_TEST_RTSPS` is set (no
    /// CI fixture TLS server), and reviewed by inspection — mirrors
    /// `live_rtsp_smoke` above for the plain-TCP path.
    #[ignore]
    #[tokio::test]
    async fn live_rtsps_smoke() {
        let url = std::env::var("MULTIMUX_TEST_RTSPS")
            .expect("set MULTIMUX_TEST_RTSPS to a live rtsps:// URL to run this test");
        let source = RtspSource::new("live-tls", url);
        let mut session = source.connect().await.expect("connect");
        assert!(!session.track_specs().is_empty());
        for _ in 0..10 {
            match session.next_samples().await.expect("next_samples") {
                Some(samples) if !samples.is_empty() => return,
                Some(_) => continue,
                None => panic!("stream ended before any sample was emitted"),
            }
        }
        panic!("no samples emitted within 10 frames");
    }
}
