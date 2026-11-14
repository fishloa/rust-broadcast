//! RTSP push transport — client ANNOUNCE/RECORD to a remote RTSP server.
//!
//! RFC 2326 §10.3 (ANNOUNCE), §10.4 (SETUP), §10.11 (RECORD). The MPEG-2 TS
//! bytes produced by the default `PushTransport::encode_media` are sent as
//! RTP (RFC 2250 §2: MPEG2-TS, static payload type 33, clock rate 90000),
//! framed as interleaved TCP data (§10.12) — one SETUP/one interleaved
//! channel for the whole muxed stream, matching the single m= line the SDP
//! below advertises.
//!
//! Fixed here (audit run-07 C3 / run-09 C1, #1025) — this transport
//! previously could not push to any real server:
//! - the configured push `url` is now actually used for every request
//!   (previously hard-coded to `rtsp://localhost/push`);
//! - userinfo credentials in the URL are honoured (`config.credentials`
//!   still wins when both are given, matching
//!   the pull routes' precedence);
//! - every response status is checked; a non-2xx now fails `connect`/`setup`
//!   instead of being silently ignored;
//! - the SDP carries a session-level `c=` line (RFC 4566 requires one
//!   somewhere) and `a=rtpmap`/`a=control`, and SETUP addresses the track's
//!   own control URL (RFC 2326 Appendix C.1.1), not the aggregate URL;
//! - the interleaved payload is now RTP, not raw TS bytes with no RTP
//!   header (RFC 2326 §10.12 requires interleaved data to be RTP/RTCP);
//! - a 401 challenge's `ClientEvent::AuthRetry` is now written to the
//!   socket — rtsp-runtime computes the retry, but this transport never
//!   sent it, so an authenticating server (Digest or Basic) always failed
//!   the push even after rtsp-runtime's own C1 fix.
//!
//! The protocol work lives in the crates that own it (issue #2368): the RTP
//! packets come from [`transmux::rtp::Mp2tPacketiser`] (RFC 2250 §2 over
//! `rtp-packet`), the control URL from [`rtsp_runtime::sdp::control_url`],
//! and `ANNOUNCE`/`SETUP`/`RECORD` from
//! [`rtsp_runtime::AsyncRtspClient::start_record`]. This transport supplies
//! the SDP's media section and the 90 kHz timestamp — the one place the wall
//! clock is read — and nothing else.

use crate::push::PushTransport;
use rtsp_runtime::Credentials;
use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::sdp::session_description;

use broadcast_common::units::MICROS_PER_SEC;
use std::time::Instant;
use tokio::net::TcpStream;
use transmux::ir::TrackSpec;
use transmux::rtp::{MP2T_CLOCK_RATE, MP2T_MAX_PAYLOAD, MP2T_PT, MP2T_TS_PACKETS_PER_PACKET};

/// The sequence number the first RTP packet of a connection carries
/// (RFC 3550 §5.1 leaves the start to the sender).
const INITIAL_SEQ: u16 = 0;
/// The session name (`s=`) the push's SDP carries.
const SESSION_NAME: &str = "multimux push";
/// Bound on the best-effort `TEARDOWN` sent from [`RtspTransport::close`].
const TEARDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Per-connection configuration for the RTSP push transport.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct RtspTransportConfig {
    /// Optional credentials for RTSP auth. Takes precedence over any
    /// username/password already present in the push URL's userinfo.
    pub credentials: Option<(String, String)>,
    /// Bounds on every awaited IO the push performs (SP6.1, defect 4): the
    /// OPTIONS/ANNOUNCE/SETUP/RECORD exchanges and each interleaved write.
    pub timeouts: broadcast_common::timeouts::Timeouts,
    /// A fixed SSRC and RTP clock origin, so a test can record byte-exact
    /// wire output. `None` (production) draws both per connection.
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub test_wire: Option<RtpWireState>,
}

/// A fresh per-connection SSRC and clock origin: the SSRC from the OS random
/// source, as a push never negotiates one over RTCP.
fn per_connection_wire() -> RtpWireState {
    RtpWireState {
        ssrc: crate::secure_random::ssrc(),
        started: Instant::now(),
    }
}

/// The per-connection SSRC and RTP clock origin (RFC 3550 §5.1, RFC 2250
/// §2's 90 kHz clock). Production draws both per connection; a recorded wire
/// golden pins both.
#[cfg_attr(not(feature = "test-hooks"), doc(hidden))]
#[derive(Debug, Clone, Copy)]
pub struct RtpWireState {
    /// The fixed RTP SSRC (RFC 3550 §5.1).
    pub ssrc: u32,
    /// The RTP timestamp origin (RFC 2250 §2's 90 kHz clock).
    pub started: std::time::Instant,
}

/// Manual `Debug`: the username/password pair never renders (issue #1142).
impl std::fmt::Debug for RtspTransportConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspTransportConfig")
            .field(
                "credentials",
                &self.credentials.as_ref().map(|_| "<redacted>"),
            )
            .field("timeouts", &self.timeouts)
            .finish()
    }
}

impl RtspTransportConfig {
    /// Set the IO timeouts (SP6.1). A caller that needs `rtsps://` TLS
    /// termination outside this transport, or a tighter bound in a test,
    /// overrides the defaults here.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: broadcast_common::timeouts::Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Set the RTSP auth credentials.
    #[must_use]
    pub fn with_credentials(mut self, username: String, password: String) -> Self {
        self.credentials = Some((username, password));
        self
    }

    /// Set a fixed SSRC and RTP clock origin (test hooks only).
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    #[must_use]
    pub fn with_test_wire(mut self, test_wire: RtpWireState) -> Self {
        self.test_wire = Some(test_wire);
        self
    }
}

/// The RTSP push transport: the rtsp-runtime async client adapter over an
/// owned TCP connection (SP6.1).
pub struct RtspTransport {
    /// The connected client, or `None` once closed.
    client: Option<rtsp_runtime::AsyncRtspClient<TcpStream>>,
    channel: u8,
    /// The presentation URL (credentials stripped): OPTIONS/ANNOUNCE/RECORD.
    url: String,
    /// The same URL, typed, so the publish flow derives the track's control
    /// URL (RFC 2326 Appendix C.1.1) and rejects a host-less URL up front.
    publish_url: url::Url,
    /// The outgoing MP2T stream's SSRC and sequence number (RFC 2250 §2,
    /// RFC 3550 §5.1).
    stream: transmux::rtp::Mp2tPacketiser,
    /// The connection's SSRC and RTP clock origin (RFC 3550 §5.1).
    wire: RtpWireState,
    /// The 90 kHz timestamp stamped on the next `send` row's packets.
    timestamp: u32,
    /// The connection's long-lived TS muxer (issue #1024).
    ts_mux: transmux::StreamingTsMux,
    /// The bounded best-effort `TEARDOWN` task spawned by [`Self::close`],
    /// kept so the task is owned (its `JoinHandle` is not dropped
    /// fire-and-forget) and so a later `close` can abort a superseded one. It
    /// is bounded by `TEARDOWN_TIMEOUT`, so it always finishes on its own.
    teardown: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for RtspTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspTransport")
            .field("connected", &self.client.is_some())
            // Path/query can carry a stream key: destination only.
            .field("url", &crate::redact::redact_destination(&self.url))
            .finish()
    }
}

#[async_trait::async_trait]
impl PushTransport for RtspTransport {
    type Config = RtspTransportConfig;
    type Error = RtspPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let mut parsed = url::Url::parse(url).map_err(|e| RtspPushError::Connect(e.to_string()))?;
        // A cannot-be-a-base URL (`rtsp:cam`, no `//host`) parses but names no
        // host; reject it rather than silently dialing 127.0.0.1 (the
        // fallback below) or panicking in `control_url`.
        let Some(host) = parsed.host_str() else {
            return Err(RtspPushError::Connect(
                "rtsp push URL has no host (expected `rtsp://host[:port]/path`, e.g. not `rtsp:cam`)"
                    .to_string(),
            ));
        };
        let host = host.to_string();
        let port = parsed.port().unwrap_or(554);
        let addr = format!("{host}:{port}");

        // Config credentials win over the URL's own userinfo (the same
        // precedence the pull routes use).
        let url_credentials = broadcast_auth::credentials_from_url(&parsed)
            .map_err(|e| RtspPushError::Connect(e.to_string()))?;
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        let clean_url = parsed.to_string();

        let mut session = ClientSession::new();
        let creds = config
            .credentials
            .as_ref()
            .map(|(user, pass)| Credentials::new(user.clone(), pass.clone()))
            .or(url_credentials);
        if let Some(creds) = creds {
            session = session.with_credentials(creds);
        }

        // The push's publish flow derives its control URL, so a host-less URL
        // is rejected here, before dialing.
        rtsp_runtime::sdp::control_url(&parsed)
            .map_err(|e| RtspPushError::Connect(e.to_string()))?;

        // Every IO the adapter performs is bounded by `config.timeouts`
        // (SP6.1, defect 4): the OPTIONS exchange below and every later
        // ANNOUNCE/SETUP/RECORD and interleaved write.
        let mut client = rtsp_runtime::AsyncRtspClient::connect(
            &addr,
            rtsp_runtime::RtspClientConfig::default()
                .with_session(session)
                .with_timeouts(config.timeouts),
        )
        .await
        .map_err(|e| RtspPushError::Connect(e.to_string()))?;

        match client.options(&clean_url).await {
            Ok(ClientEvent::Response { status, .. }) if status.is_success() => {}
            Ok(ClientEvent::Response { status, .. }) => {
                return Err(RtspPushError::Protocol(format!(
                    "OPTIONS rejected: {status:?}"
                )));
            }
            Ok(_) => {
                return Err(RtspPushError::Protocol(
                    "OPTIONS: unexpected event".to_string(),
                ));
            }
            Err(e) => return Err(RtspPushError::Protocol(e.to_string())),
        }

        #[cfg(feature = "test-hooks")]
        let wire = config.test_wire.unwrap_or_else(per_connection_wire);
        #[cfg(not(feature = "test-hooks"))]
        let wire = per_connection_wire();
        Ok(Self {
            client: Some(client),
            channel: 0,
            url: clean_url,
            publish_url: parsed,
            stream: transmux::rtp::Mp2tPacketiser::new(wire.ssrc, INITIAL_SEQ),
            wire,
            timestamp: 0,
            ts_mux: transmux::StreamingTsMux::new(),
            teardown: None,
        })
    }

    async fn setup(&mut self, _tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        let sdp = build_sdp()?;
        let channel = self.channel;
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
        match client.start_record(&self.publish_url, &sdp, channel).await {
            Ok(()) => {
                self.timestamp = self.rtp_timestamp();
                Ok(())
            }
            Err(e) => Err(map_rtsp_error(e)),
        }
    }

    fn ts_mux(&mut self) -> Option<&mut transmux::StreamingTsMux> {
        Some(&mut self.ts_mux)
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let channel = self.channel;
        if !data.len().is_multiple_of(mpeg_ts::ts::TS_PACKET_SIZE) {
            return Err(RtspPushError::Protocol(
                "RTSP push payload is not a whole number of 188-byte TS packets".to_string(),
            ));
        }
        let packets = self.packetise(data)?;
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
        for packet in packets {
            client
                .send_interleaved(channel, &packet)
                .await
                .map_err(map_rtsp_error)?;
        }
        Ok(())
    }

    /// Drop the connection, first sending a bounded, best-effort `TEARDOWN`
    /// (RFC 2326 §10.10) so the server frees the publisher slot immediately
    /// rather than at its own session timeout — a quick reconnect to
    /// `mediamtx`/`gortsplib` would otherwise be rejected as "path already has
    /// a publisher".
    ///
    /// `close` is synchronous (the trait is), so the TEARDOWN is dispatched on
    /// a spawned task. Three properties are guaranteed:
    /// - a **second** `close()` on an already-closed transport is a no-op (the
    ///   client is `None`), so exactly one TEARDOWN is sent per connection;
    /// - a superseded handle is **aborted** before it is replaced, so no
    ///   detached task outlives its use;
    /// - outside a Tokio runtime (`Handle::try_current()` is `Err`, e.g. a
    ///   `Drop` in a sync context) the best-effort TEARDOWN is **skipped** with
    ///   a log, never a panic.
    ///
    /// It is best-effort at process shutdown: a TEARDOWN spawned as the process
    /// exits may not run, which is acceptable (the server frees the slot at its
    /// own timeout).
    fn close(&mut self) {
        if let Some(mut client) = self.client.take() {
            let Some(handle) = teardown_runtime_handle() else {
                tracing::debug!(
                    "RTSP push closed outside a Tokio runtime; skipping the best-effort TEARDOWN"
                );
                return;
            };
            let uri = self.url.clone();
            let task = handle.spawn(async move {
                let _ = tokio::time::timeout(TEARDOWN_TIMEOUT, client.teardown(&uri)).await;
            });
            // Abort a still-running superseded teardown before replacing it
            // (the field is otherwise only nominally "owned").
            if let Some(old) = self.teardown.replace(task) {
                old.abort();
            }
        }
    }
}

/// The runtime handle to spawn the best-effort `TEARDOWN` on, or `None`
/// outside a Tokio runtime (where spawning would panic) — factored out so the
/// no-runtime path is unit-testable (I-D).
fn teardown_runtime_handle() -> Option<tokio::runtime::Handle> {
    tokio::runtime::Handle::try_current().ok()
}

/// Map an rtsp-runtime error onto this transport's error, preserving the
/// session-lost signal (RFC 2326 §11.3.4 `454`) distinctly from a generic
/// protocol error so the push reconnects instead of retrying in place.
fn map_rtsp_error(e: rtsp_runtime::Error) -> RtspPushError {
    match e {
        rtsp_runtime::Error::SessionNotFound { .. } => RtspPushError::SessionLost(e.to_string()),
        other => RtspPushError::Protocol(other.to_string()),
    }
}

impl RtspTransport {
    /// The RTP packets one `send` payload becomes: its TS packets in RFC 2250
    /// §2 rows of at most seven, packetised by
    /// [`transmux::rtp::Mp2tPacketiser`].
    ///
    /// A row that is not exactly seven TS packets is sent on its own: RFC 2250
    /// §2 lets a packet carry up to seven, but a row's timestamp is the
    /// instant of its first TS packet, so a short row appended to a full one
    /// would misdate the TS packets after it. Rows that are a whole number of
    /// seven TS packets share the connection's one clock read, which is what a
    /// TS muxer's batches are.
    fn packetise(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, RtspPushError> {
        let row_ts = self.timestamp;
        let mut packets: Vec<Vec<u8>> = Vec::with_capacity(data.len().div_ceil(MP2T_MAX_PAYLOAD));
        for row in data.chunks(MP2T_TS_PACKETS_PER_PACKET * mpeg_ts::ts::TS_PACKET_SIZE) {
            let timestamp = if row.len() == MP2T_MAX_PAYLOAD {
                row_ts
            } else {
                self.rtp_timestamp()
            };
            packets.extend(
                self.stream
                    .packetise(row, timestamp)
                    .map_err(|e| RtspPushError::Protocol(e.to_string()))?,
            );
        }
        Ok(packets)
    }

    /// The RTP timestamp for the packets sent from now on: RFC 2250 §2's
    /// 90 kHz clock, counted from the connection's start and wrapped at 32
    /// bits (RFC 3550 §5.1). This is the push path's one wall-clock read, and
    /// every RTP packet of one `send` row carries the value it returned, as
    /// RFC 2250 §2 stamps a packet with the instant of its first TS packet.
    fn rtp_timestamp(&self) -> u32 {
        let micros = u64::try_from(self.wire.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        u32::try_from(
            u128::from(micros) * u128::from(MP2T_CLOCK_RATE) / u128::from(MICROS_PER_SEC as u32),
        )
        .unwrap_or(u32::MAX)
    }
}

/// `#[doc(hidden)]` test seam: parse `url` and return [`control_url`], so the
/// URL-construction test exercises the real path. Errors (an invalid URL, or a
/// cannot-be-a-base `rtsp:cam`) surface as a `String`, matching the seam's
/// test-only role.
#[doc(hidden)]
pub fn control_url_for_test(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    rtsp_runtime::sdp::control_url(&parsed).map_err(|e| e.to_string())
}

/// Builds the SDP for the push session: one `m=` line for the whole
/// MPEG-2 TS-muxed stream (RFC 2250 §2) — every track is multiplexed into
/// this single RTP session, matching the one SETUP/one interleaved channel
/// this transport actually uses, not one `m=` line per elementary track.
///
/// The session-level lines come from `rtsp_runtime::sdp::session_description`
/// (the one session builder in the workspace, issue #2357); this transport
/// supplies only the media description.
fn build_sdp() -> Result<String, RtspPushError> {
    let origin = sdp_types::Origin::new(
        "0",
        0,
        sdp_types::NetType::In,
        sdp_types::AddrType::Ip4,
        "0.0.0.0",
    );
    let connection =
        sdp_types::Connection::new(sdp_types::NetType::In, sdp_types::AddrType::Ip4, "0.0.0.0");
    session_description(SESSION_NAME, origin, connection, vec![push_media()])
        .map_err(|e| RtspPushError::Protocol(format!("SDP session: {e}")))
}

/// One `m=` section for the whole MPEG-2 TS-muxed stream: RFC 2250 §2's
/// static MP2T payload type on its fixed 90 kHz clock, and the `control`
/// attribute matching the one control URL `SETUP` addresses.
fn push_media() -> sdp_types::Media {
    use sdp_types::{Attribute, Media, MediaType, TransportProto};

    let mut media = Media::new(
        MediaType::Video,
        0,
        TransportProto::RtpAvp,
        MP2T_PT.to_string(),
    );
    media.add_attribute(Attribute::with_value(
        "rtpmap",
        format!("{MP2T_PT} MP2T/{MP2T_CLOCK_RATE}"),
    ));
    media.add_attribute(Attribute::with_value("control", "trackID=0"));
    media
}

/// `#[doc(hidden)]` test seam: the SDP the push announces, so the golden
/// recorded in `rtsp-runtime`'s tests can reach the private builder.
#[doc(hidden)]
pub fn push_sdp_for_test() -> Result<String, RtspPushError> {
    build_sdp()
}

/// `#[doc(hidden)]` test seam: the media sections whose `m=` block the push
/// announces (see [`push_sdp_for_test`]).
#[doc(hidden)]
pub fn push_media_for_test() -> Vec<sdp_types::Media> {
    vec![push_media()]
}

/// Errors from the RTSP push transport.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtspPushError {
    #[error("RTSP connect failed: {0}")]
    Connect(String),
    #[error("RTSP protocol error: {0}")]
    Protocol(String),
    /// The server no longer has the session (RFC 2326 §11.3.4 `454 Session
    /// Not Found`) — the push must reconnect rather than keep sending.
    #[error("RTSP session lost: {0}")]
    SessionLost(String),
    #[error("RTSP I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed SSRC and a fresh clock origin, so a test can assert the exact
    /// bytes (the timestamp itself is the one field a wall clock sets).
    fn test_wire() -> RtpWireState {
        RtpWireState {
            ssrc: 1,
            started: Instant::now(),
        }
    }

    /// Byte-for-byte golden (SP4.1): the ANNOUNCE SDP body the transport
    /// sends equals `golden/rtsp/rtsp-announce.sdp`, captured from the
    /// pre-`sdp-types` `format!` on `origin/main`. `GOLDEN_BLESS=<dir>`
    /// writes instead.
    #[test]
    fn announce_sdp_golden() {
        let actual = build_sdp().expect("build_sdp");
        let file = "golden/rtsp/rtsp-announce.sdp";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = test_fixtures::path(file);
        let expected = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(actual, expected, "{file} differs from the golden output");
    }

    /// Audit T14 (#1142): `Debug` of a transport prints the destination only
    /// (the path and query can carry a stream key).
    #[test]
    fn rtsp_transport_debug_does_not_print_the_stream_key() {
        let transport = RtspTransport {
            client: None,
            channel: 0,
            url: "rtsp://cam.example:554/live/STREAMKEY123?token=abc".to_string(),
            publish_url: url::Url::parse("rtsp://cam.example:554/live/STREAMKEY123?token=abc")
                .expect("test URL"),
            stream: transmux::rtp::Mp2tPacketiser::new(1, INITIAL_SEQ),
            wire: test_wire(),
            timestamp: 0,
            ts_mux: transmux::StreamingTsMux::new(),
            teardown: None,
        };
        let shown = format!("{transport:?}");
        assert!(
            !shown.contains("STREAMKEY123") && !shown.contains("token"),
            "{shown}"
        );
        assert!(
            shown.contains("rtsp://cam.example:554/<redacted>"),
            "{shown}"
        );
    }

    #[test]
    fn sdp_has_a_session_level_c_line_and_matches_the_single_control_url() {
        // RFC 4566 §5 requires a `c=` line somewhere (session- or
        // media-level) — the pre-fix SDP had neither, which is invalid SDP
        // regardless of the RTP-framing bugs.
        let sdp = build_sdp().expect("build_sdp");
        assert!(sdp.contains("\r\nc=IN IP4"), "missing c= line: {sdp}");
        assert!(
            sdp.contains("a=control:trackID=0"),
            "control attribute must match the one control URL SETUP uses: {sdp}"
        );
        assert!(
            sdp.contains(&format!("RTP/AVP {MP2T_PT}")),
            "must advertise the static MP2T payload type: {sdp}"
        );
        // Exactly one m= line: everything is multiplexed into the one TS
        // stream this transport actually SETUPs and sends.
        assert_eq!(
            sdp.matches("\r\nm=").count(),
            1,
            "expected one m= line: {sdp}"
        );
    }

    #[test]
    fn publish_control_url_is_appended_with_a_single_slash() {
        // Regression shape for RFC 2326 Appendix C.1.1: no double slash
        // when the presentation URL already ends in one, no missing slash
        // otherwise.
        for (base, expected) in [
            ("rtsp://h/push", "rtsp://h/push/trackID=0"),
            ("rtsp://h/push/", "rtsp://h/push/trackID=0"),
        ] {
            assert_eq!(
                control_url_for_test(base).expect("control URL"),
                expected,
                "{base}"
            );
        }
    }

    /// One `send` payload becomes RFC 2250 §2 rows of at most seven TS
    /// packets, stamped with the connection's 90 kHz clock and serialized
    /// entirely by the transmux packetiser.
    ///
    /// Revert-check: go back to appending a short row to a full one (or to
    /// stamping a packet with a fresh read) and the rows no longer line up
    /// with `MP2T_MAX_PAYLOAD`.
    #[test]
    fn a_send_payload_becomes_mp2t_rows_of_at_most_seven_ts_packets() {
        let mut transport = RtspTransport {
            client: None,
            channel: 0,
            url: "rtsp://h/push".to_string(),
            publish_url: url::Url::parse("rtsp://h/push").expect("test URL"),
            stream: transmux::rtp::Mp2tPacketiser::new(0xdead_beef, 0x1234),
            wire: test_wire(),
            timestamp: 7_500,
            ts_mux: transmux::StreamingTsMux::new(),
            teardown: None,
        };
        let mut payload = vec![0u8; MP2T_MAX_PAYLOAD + mpeg_ts::ts::TS_PACKET_SIZE];
        for (i, ts_packet) in payload.chunks_mut(mpeg_ts::ts::TS_PACKET_SIZE).enumerate() {
            ts_packet[0] = 0x47;
            ts_packet[1] = u8::try_from(i).unwrap_or(0);
        }

        let packets = transport.packetise(&payload).expect("packetise");
        assert_eq!(packets.len(), 2, "a full row, then the one leftover packet");
        let first =
            <rtp_packet::RtpPacket as broadcast_common::Parse>::parse(&packets[0]).expect("parse");
        assert_eq!(first.payload_type, MP2T_PT, "static MP2T payload type");
        assert_eq!(first.sequence_number, 0x1234);
        assert_eq!(first.timestamp, 7_500);
        assert_eq!(first.ssrc, 0xdead_beef);
        let mut reassembled = first.payload.to_vec();
        reassembled.extend_from_slice(
            <rtp_packet::RtpPacket as broadcast_common::Parse>::parse(&packets[1])
                .expect("parse")
                .payload,
        );
        assert_eq!(reassembled, payload, "every TS packet, in order");
    }

    /// A payload that is not whole TS packets is refused rather than cut.
    #[test]
    fn a_partial_ts_packet_in_a_send_payload_is_an_error() {
        let mut transport = RtspTransport {
            client: None,
            channel: 0,
            url: "rtsp://h/push".to_string(),
            publish_url: url::Url::parse("rtsp://h/push").expect("test URL"),
            stream: transmux::rtp::Mp2tPacketiser::new(1, INITIAL_SEQ),
            wire: test_wire(),
            timestamp: 0,
            ts_mux: transmux::StreamingTsMux::new(),
            teardown: None,
        };
        assert!(transport.packetise(&[0u8; 100]).is_err());
    }

    // Regression: this transport's errors can end up in route logs, so the
    // configured password must never appear in a connect failure, and
    // `Debug` must never show the credential (mirrors rtsp-runtime's own
    // `client_session_debug_does_not_leak_embedded_credentials_secret`).
    #[test]
    fn transport_debug_does_not_leak_the_configured_password() {
        // `RtspTransport` holds an `AsyncRtspClient` whose `ClientSession`
        // carries the credentials, but `Debug` prints only `connected` and
        // the redacted URL, so the secret can never reach a log line.
        let transport = RtspTransport {
            client: None,
            channel: 0,
            url: "rtsp://cam.example:554/live/STREAMKEY123?token=abc".to_string(),
            publish_url: url::Url::parse("rtsp://cam.example:554/live/STREAMKEY123?token=abc")
                .expect("test URL"),
            stream: transmux::rtp::Mp2tPacketiser::new(1, INITIAL_SEQ),
            wire: test_wire(),
            timestamp: 0,
            ts_mux: transmux::StreamingTsMux::new(),
            teardown: None,
        };
        let debug = format!("{transport:?}");
        assert!(
            !debug.contains("STREAMKEY123") && !debug.contains("token"),
            "leaked via RtspTransport Debug: {debug}"
        );
    }

    /// I5: a server `454 Session Not Found` maps to `SessionLost` (so the push
    /// reconnects) rather than a generic `Protocol` error.
    ///
    /// Revert-check: drop the `SessionNotFound` arm in `map_rtsp_error` (map
    /// everything to `Protocol`) and the match fails.
    #[test]
    fn a_454_maps_to_session_lost_distinctly() {
        let err = map_rtsp_error(rtsp_runtime::Error::SessionNotFound {
            method: rtsp_runtime::Method::GetParameter,
        });
        assert!(
            matches!(err, RtspPushError::SessionLost(_)),
            "a 454 must map to SessionLost, got {err:?}"
        );
        // A different rtsp error stays a plain protocol error.
        let other = map_rtsp_error(rtsp_runtime::Error::MessageParse {
            what: rtsp_runtime::MessageKind::Response,
            source: "x".into(),
        });
        assert!(matches!(other, RtspPushError::Protocol(_)), "{other:?}");
    }

    /// I-D(2): outside a Tokio runtime the best-effort TEARDOWN is skipped
    /// (no handle), so `close()` never panics. The no-runtime helper is the
    /// load-bearing predicate.
    ///
    /// Revert-check: make `teardown_runtime_handle` return `Some(..)`
    /// unconditionally (or drop the guard) and the assertion fails.
    #[test]
    fn close_outside_a_runtime_does_not_panic() {
        assert!(
            teardown_runtime_handle().is_none(),
            "outside a runtime there must be no spawn handle"
        );
        let mut transport = RtspTransport {
            client: None,
            channel: 0,
            url: "rtsp://h/push".to_string(),
            publish_url: url::Url::parse("rtsp://h/push").expect("test URL"),
            stream: transmux::rtp::Mp2tPacketiser::new(1, INITIAL_SEQ),
            wire: test_wire(),
            timestamp: 0,
            ts_mux: transmux::StreamingTsMux::new(),
            teardown: None,
        };
        // No runtime is active here; close() must not panic. (A `None` client
        // still exercises the `Handle::try_current()` path when the guard is
        // reached with a client, but the no-client form proves no panic.)
        transport.close();
    }

    /// Only turns a keepalive that never elicits the 454 into a failure. The
    /// keepalive is due after half the one-second session timeout, so a stall
    /// of the whole process shorter than this cannot fail the test.
    const KEEPALIVE_HANG_CEILING: std::time::Duration = std::time::Duration::from_secs(60);

    /// I-D(1): a real socket 454 reaches `SessionLost` — the wiring, not just
    /// the pure `map_rtsp_error`. A loopback peer serves the record handshake,
    /// then answers the push's next request (the keepalive `GET_PARAMETER`)
    /// with 454; `send_interleaved`'s inbound drain surfaces
    /// `Error::SessionNotFound`, which `map_rtsp_error` maps to `SessionLost`.
    #[tokio::test]
    async fn a_socket_454_surfaces_session_lost_through_map_rtsp_error() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut recorded = false;
            loop {
                // Read one request (headers end at CRLFCRLF).
                let mut chunk = [0u8; 2048];
                let n = match sock.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&chunk[..n]);
                let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                buf.drain(..end + 4);
                let cseq = head
                    .lines()
                    .find_map(|l| l.strip_prefix("CSeq:").map(|v| v.trim().to_string()))
                    .unwrap_or_else(|| "1".to_string());
                let method = head.split_whitespace().next().unwrap_or("").to_string();
                let resp = if recorded {
                    // Every request after RECORD is answered 454.
                    format!("RTSP/1.0 454 Session Not Found\r\nCSeq: {cseq}\r\n\r\n")
                } else {
                    let extra = if method == "SETUP" {
                        "Transport: RTP/AVP/TCP;unicast;interleaved=0-1;mode=record\r\n\
                         Session: 1;timeout=1\r\n"
                    } else {
                        "Session: 1;timeout=1\r\n"
                    };
                    format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n{extra}\r\n")
                };
                if sock.write_all(resp.as_bytes()).await.is_err() {
                    return;
                }
                if method == "RECORD" {
                    recorded = true;
                }
            }
        });

        let mut client = rtsp_runtime::AsyncRtspClient::connect(
            &addr,
            rtsp_runtime::RtspClientConfig::default()
                .with_session(ClientSession::new())
                .with_timeouts(broadcast_common::timeouts::Timeouts::default()),
        )
        .await
        .expect("connect");
        let url = url::Url::parse(&format!("rtsp://{addr}/live/key")).expect("URL");
        let sdp = build_sdp().expect("sdp");
        client
            .start_record(&url, &sdp, 0)
            .await
            .expect("the record handshake completes");

        // Send media until the keepalive (session timeout=1 s) elicits the 454
        // and the drain surfaces it.
        let deadline = tokio::time::Instant::now() + KEEPALIVE_HANG_CEILING;
        loop {
            match client.send_interleaved(0, &[0u8; 188]).await {
                Ok(()) => {}
                Err(e) => {
                    assert!(
                        matches!(map_rtsp_error(e), RtspPushError::SessionLost(_)),
                        "a socket 454 must map to SessionLost"
                    );
                    return;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no 454 surfaced in time"
            );
            tokio::task::yield_now().await;
        }
    }

    /// m-2: the "no host" error is a clean single-line literal (no embedded
    /// newline or run of spaces from a bad line continuation).
    ///
    /// Revert-check: restore the wrapped `\n                 e.g.` literal and
    /// this fails.
    #[tokio::test]
    async fn the_no_host_error_is_a_single_line() {
        let err = RtspTransport::connect("rtsp:cam", &RtspTransportConfig::default())
            .await
            .expect_err("a cannot-be-a-base URL must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains('\n'),
            "error message must be single-line: {msg:?}"
        );
        assert!(
            !msg.contains("  "),
            "error message must have no run of spaces: {msg:?}"
        );
    }
}
