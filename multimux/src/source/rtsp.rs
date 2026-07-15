//! RTSP ingest source — DESCRIBE/SETUP/PLAY over interleaved TCP, depayloaded
//! into timed [`Sample`]s.
//!
//! Drives [`rtsp_runtime::io::AsyncRtspClient`] (RFC 2326 Appendix A client
//! state machine, plain `rtsp://` + interleaved `$`-framed media per §10.12)
//! and feeds each received RTP packet into a [`RtpStreamDepacketizer`]
//! (RFC 6184 / RFC 3640) built from the DESCRIBE SDP
//! ([`crate::source::sdp::parse_sdp_tracks`]).
//!
//! `RtspSource` is the immutable "how to reach this stream" descriptor;
//! [`RtspSource::connect`] performs the DESCRIBE → SETUP (one per media,
//! interleaved) → PLAY handshake and returns a live [`RtspSession`] that
//! [`RtspSession::next_samples`] pulls one interleaved frame at a time.

use rtsp_runtime::client::ClientEvent;
use rtsp_runtime::io::AsyncRtspClient;
use rtsp_runtime::transport::{Transport, TransportSpec};
use tokio::net::TcpStream;

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
        let (host, port) = host_port(&self.url)?;
        let mut client = AsyncRtspClient::<TcpStream>::connect((host.as_str(), port))
            .await
            .map_err(source_err("connect"))?;

        let describe = client
            .describe(&self.url)
            .await
            .map_err(source_err("DESCRIBE"))?;
        let sdp = expect_ok_response(describe, "DESCRIBE")?;
        let mut tracks = parse_sdp_tracks(&sdp)?;

        for track in &mut tracks {
            let uri = setup_uri(&self.url, track.control.as_deref());
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
    client: AsyncRtspClient<TcpStream>,
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

/// Builds a per-media SETUP URI from the base RTSP URL and its `a=control`
/// value: an absolute `rtsp(s)://` control is used verbatim, a relative one is
/// joined onto the base URL, and a missing control falls back to the base URL
/// itself (RFC 2326 §C.1.1).
fn setup_uri(base_url: &str, control: Option<&str>) -> String {
    match control {
        None => base_url.to_string(),
        Some(c) if c.starts_with("rtsp://") || c.starts_with("rtsps://") => c.to_string(),
        Some(c) if base_url.ends_with('/') => format!("{base_url}{c}"),
        Some(c) => format!("{base_url}/{c}"),
    }
}

/// Extracts `(host, port)` from a `rtsp://`/`rtsps://` URL for the initial TCP
/// connect, defaulting to [`RTSP_DEFAULT_PORT`] when no port is given.
///
/// This is a minimal authority parser (host, optional `:port`) sufficient for
/// the plain hostnames/IPv4 addresses real cameras and media servers publish;
/// it does not handle bracketed IPv6 literals or userinfo.
fn host_port(url: &str) -> Result<(String, u16)> {
    let rest = url
        .strip_prefix("rtsp://")
        .or_else(|| url.strip_prefix("rtsps://"))
        .ok_or_else(|| MultimuxError::Source(format!("not an rtsp(s) URL: {url:?}")))?;
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((host, port_str)) => {
            let port = port_str
                .parse::<u16>()
                .map_err(|e| MultimuxError::Source(format!("bad port in {url:?}: {e}")))?;
            Ok((host.to_string(), port))
        }
        None => Ok((authority.to_string(), RTSP_DEFAULT_PORT)),
    }
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
    fn setup_uri_absolute_control_used_verbatim() {
        assert_eq!(
            setup_uri("rtsp://cam/base", Some("rtsp://other/track1")),
            "rtsp://other/track1"
        );
    }

    #[test]
    fn setup_uri_relative_control_joined() {
        assert_eq!(
            setup_uri("rtsp://cam/base", Some("streamid=0")),
            "rtsp://cam/base/streamid=0"
        );
        assert_eq!(
            setup_uri("rtsp://cam/base/", Some("streamid=0")),
            "rtsp://cam/base/streamid=0"
        );
    }

    #[test]
    fn setup_uri_missing_control_falls_back_to_base() {
        assert_eq!(setup_uri("rtsp://cam/base", None), "rtsp://cam/base");
    }

    #[test]
    fn host_port_defaults_to_rtsp_port() {
        assert_eq!(
            host_port("rtsp://cam.local/stream").unwrap(),
            ("cam.local".to_string(), RTSP_DEFAULT_PORT)
        );
    }

    #[test]
    fn host_port_parses_explicit_port() {
        assert_eq!(
            host_port("rtsp://cam.local:8554/stream").unwrap(),
            ("cam.local".to_string(), 8554)
        );
    }

    #[test]
    fn host_port_rejects_non_rtsp_scheme() {
        assert!(host_port("http://cam.local/stream").is_err());
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
}
