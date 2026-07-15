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

use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::io::AsyncRtspClient;
use rtsp_runtime::transport::{Transport, TransportSpec};
use tokio::net::TcpStream;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::{Source, TrackInit, sdp::parse_sdp_tracks};
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
#[derive(Debug, Clone)]
pub struct RtspSource {
    name: String,
    url: String,
}

impl RtspSource {
    /// Build a source descriptor. `url` is the `rtsp://` (or `rtsps://`) URL
    /// used for DESCRIBE and as the base for relative `a=control` SETUP URLs.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        RtspSource {
            name: name.into(),
            url: url.into(),
        }
    }

    /// Connects, DESCRIBEs, SETUPs every media (interleaved TCP, channels
    /// `(2i, 2i+1)` per the SDP's media order), and PLAYs — returning a live
    /// session ready for [`RtspSession::next_samples`].
    pub async fn connect(&self) -> Result<RtspSession> {
        let base_url = Url::parse(&self.url)
            .map_err(|e| MultimuxError::Source(format!("bad rtsp(s) URL {:?}: {e}", self.url)))?;
        let is_tls = scheme_is_tls(&base_url)?;
        let addr = connect_addr(&base_url)?;
        let mut client = if is_tls {
            let server_name = sni_server_name(&base_url)?;
            connect_tls_client(&addr, &server_name).await?
        } else {
            connect_plain_client(&addr).await?
        };

        let describe = client
            .describe(&self.url)
            .await
            .map_err(source_err("DESCRIBE"))?;
        let sdp = expect_ok_response(describe, "DESCRIBE")?;
        let mut tracks = parse_sdp_tracks(&sdp)?;

        for track in &mut tracks {
            let uri = resolve_control(&base_url, track.control.as_deref())?;
            let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(
                track.channel,
                track.channel.saturating_add(RTCP_CHANNEL_OFFSET),
            ));
            let setup = client
                .setup(&uri, &transport)
                .await
                .map_err(source_err("SETUP"))?;
            expect_ok_response(setup, "SETUP")?;

            // The server must negotiate TCP-interleaved transport. Extract the
            // negotiated spec and verify it has both TCP lower-transport and
            // interleaved channels; reject any non-interleaved response (e.g.
            // a server that ignores TCP and negotiates UDP).
            let spec = client
                .session()
                .negotiated_transport()
                .and_then(Transport::first)
                .ok_or_else(|| {
                    MultimuxError::Source(format!(
                        "SETUP {}: server did not provide negotiated transport",
                        track.track_id
                    ))
                })?;

            // Ensure the negotiated transport is TCP-interleaved.
            if let Some(channel) = interleaved_channel(spec) {
                track.channel = channel;
            } else {
                return Err(MultimuxError::Source(format!(
                    "SETUP {}: server did not negotiate interleaved TCP transport",
                    track.track_id
                )));
            }
        }

        let play = client.play(&self.url).await.map_err(source_err("PLAY"))?;
        expect_ok_response(play, "PLAY")?;

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
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let event = self
            .client
            .recv_interleaved()
            .await
            .map_err(source_err("recv"))?;
        let Some(event) = event else {
            return Ok(None);
        };
        let ClientEvent::MediaData { channel, data } = event else {
            return Ok(Some(Vec::new()));
        };
        let Some(track_id) = route_channel(channel, &self.tracks) else {
            return Ok(Some(Vec::new()));
        };
        let samples = self.depacketizer.push(track_id, &data)?;
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
            .map_err(|e| MultimuxError::Source(format!("bad a=control {c:?}: {e}"))),
    }
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
    let host = url
        .host_str()
        .ok_or_else(|| MultimuxError::Source(format!("rtsp(s) URL has no host: {url}")))?;
    let port = url.port().unwrap_or(default_port);
    Ok(format!("{host}:{port}"))
}

/// Unwraps a `ClientEvent::Response` with a success status into its body;
/// anything else (non-2xx status, or an unexpected event shape) becomes a
/// `MultimuxError::Source` naming which request failed.
fn expect_ok_response(event: ClientEvent, what: &'static str) -> Result<Vec<u8>> {
    match event {
        ClientEvent::Response { status, body, .. } if status.is_success() => Ok(body),
        ClientEvent::Response { status, .. } => Err(MultimuxError::Source(format!(
            "{what}: non-success status {status}"
        ))),
        other => Err(MultimuxError::Source(format!(
            "{what}: unexpected event {other:?}"
        ))),
    }
}

/// Maps an `rtsp-runtime` error into `MultimuxError::Source`, naming which
/// step failed.
fn source_err(what: &'static str) -> impl Fn(rtsp_runtime::error::Error) -> MultimuxError {
    move |e| MultimuxError::Source(format!("{what}: {e}"))
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
        other => Err(MultimuxError::Source(format!(
            "not an rtsp(s) URL scheme: {other}"
        ))),
    }
}

/// Derives the SNI server name for TLS handshake from a base `rtsp(s)://` URL,
/// stripping brackets from IPv6 literals. `Url::host_str()` returns IPv6
/// addresses in bracketed form (per RFC 3986 authority syntax, e.g.
/// `"[2001:db8::1]"`), but rustls `ServerName::try_from()` rejects the
/// brackets. This function extracts the host and strips leading `[` and
/// trailing `]` if present, leaving hostnames and IPv4 addresses unchanged.
fn sni_server_name(url: &Url) -> Result<String> {
    let host = url
        .host_str()
        .ok_or_else(|| MultimuxError::Source(format!("rtsp(s) URL has no host: {url}")))?;
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

/// Connects a plain `rtsp://` (TCP) client to `addr`.
async fn connect_plain_client(addr: &str) -> Result<RtspClient> {
    let client = AsyncRtspClient::<TcpStream>::connect(addr)
        .await
        .map_err(source_err("connect"))?;
    Ok(RtspClient::Plain(client))
}

/// Connects an `rtsps://` (RTSP-over-TLS) client to `addr`, presenting
/// `server_name` for SNI/certificate validation against the public-CA trust
/// store ([`rtsp_runtime::io::default_tls_client_config`]).
///
/// Only available when this crate's `tls` feature is enabled; otherwise
/// returns a `MultimuxError::Source` naming the missing feature (rather than
/// failing to compile), so callers get a clear runtime message if `tls` was
/// deliberately disabled.
#[cfg(feature = "tls")]
async fn connect_tls_client(addr: &str, server_name: &str) -> Result<RtspClient> {
    let config = rtsp_runtime::io::default_tls_client_config();
    let client = AsyncRtspClient::<tokio_rustls::client::TlsStream<TcpStream>>::connect_tls(
        addr,
        server_name,
        config,
    )
    .await
    .map_err(source_err("connect"))?;
    Ok(RtspClient::Tls(client))
}

#[cfg(not(feature = "tls"))]
async fn connect_tls_client(addr: &str, _server_name: &str) -> Result<RtspClient> {
    Err(MultimuxError::Source(format!(
        "rtsps:// (TLS) requires multimux's `tls` feature; cannot connect to {addr}"
    )))
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
