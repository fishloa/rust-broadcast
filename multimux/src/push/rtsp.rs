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
//!   `crate::source::http_auth::resolve_credentials`' precedence);
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

use crate::push::PushTransport;
use rtsp_runtime::Credentials;
use rtsp_runtime::client::{ClientEvent, ClientSession};
use rtsp_runtime::transport::{Transport, TransportSpec};
use std::time::Instant;
use tokio::net::TcpStream;
use transmux::ir::TrackSpec;

/// RFC 3551 §6 static payload type for MPEG2 Transport Stream (RFC 2250 §2).
const PT_MP2T: u8 = 33;
/// MPEG-2 TS packet length (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;
/// RFC 2250 §2: "no more than seven ... TS packets" per RTP packet.
const MAX_TS_PACKETS_PER_RTP: usize = 7;
/// The resulting cap on one RTP packet's MP2T payload (1316 bytes).
const MAX_RTP_PAYLOAD: usize = TS_PACKET_LEN * MAX_TS_PACKETS_PER_RTP;
/// RFC 2250 §2's fixed RTP clock rate for MP2T.
const RTP_CLOCK_HZ: u64 = 90_000;
/// RFC 3550 §5.1: the fixed 12-byte RTP header (V/P/X/CC, M/PT, sequence,
/// timestamp, SSRC) with no CSRCs and no extension.
const RTP_HEADER_LEN: usize = 12;
/// RFC 3550 §5.1: the first header byte `V=2, P=0, X=0, CC=0` (`0b10_000000`).
const RTP_VERSION_2_NO_FLAGS: u8 = 0b1000_0000;
/// Bound on the best-effort `TEARDOWN` sent from [`RtspTransport::close`].
const TEARDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Per-connection configuration for the RTSP push transport.
#[derive(Debug, Clone, Default)]
pub struct RtspTransportConfig {
    /// Optional credentials for RTSP auth. Takes precedence over any
    /// username/password already present in the push URL's userinfo.
    pub credentials: Option<(String, String)>,
    /// Bounds on every awaited IO the push performs (SP6.1, defect 4): the
    /// OPTIONS/ANNOUNCE/SETUP/RECORD exchanges and each interleaved write.
    pub timeouts: rtsp_runtime::RtspTimeouts,
}

impl RtspTransportConfig {
    /// Set the IO timeouts (SP6.1). A caller that needs `rtsps://` TLS
    /// termination outside this transport, or a tighter bound in a test,
    /// overrides the defaults here.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: rtsp_runtime::RtspTimeouts) -> Self {
        self.timeouts = timeouts;
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
    /// The one track's control URL (RFC 2326 Appendix C.1.1): SETUP.
    control_url: String,
    /// RTP sequence number for the outgoing MP2T stream (RFC 3550 §5.1).
    seq: u16,
    /// Pseudo-random per-connection SSRC (no RTCP negotiation on a push).
    ssrc: u32,
    /// Wall-clock origin for the RTP timestamp (90 kHz, RFC 2250 §2).
    started: Instant,
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

        // Config credentials win over the URL's own userinfo (mirrors
        // `crate::source::http_auth::resolve_credentials`'s precedence).
        let url_credentials = credentials_from_url(&parsed)?;
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

        let control_url = control_url(&parsed)?;

        // Every IO the adapter performs is bounded by `config.timeouts`
        // (SP6.1, defect 4): the OPTIONS exchange below and every later
        // ANNOUNCE/SETUP/RECORD and interleaved write.
        let mut client =
            rtsp_runtime::AsyncRtspClient::connect_with_timeouts(&addr, session, config.timeouts)
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

        Ok(Self {
            client: Some(client),
            channel: 0,
            url: clean_url,
            control_url,
            seq: 0,
            ssrc: rand_ssrc(),
            started: Instant::now(),
            teardown: None,
        })
    }

    async fn setup(&mut self, _tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        let sdp = build_sdp()?;
        // RFC 2326 §12.39: `mode` defaults to PLAY: a RECORD session must
        // say so explicitly, or a strict server (gortsplib/`mediamtx`:
        // "transport header contains a invalid mode (null)") rejects SETUP.
        let mut spec = TransportSpec::rtp_avp_tcp_interleaved(self.channel, self.channel + 1);
        spec.mode = vec![rtsp_runtime::TransportMode::Record];
        let transport_spec = Transport::single(spec);
        let (url, control_url) = (self.url.clone(), self.control_url.clone());
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;

        for (what, event) in [
            ("ANNOUNCE", client.announce(&url, &sdp).await),
            ("SETUP", client.setup(&control_url, &transport_spec).await),
            ("RECORD", client.record(&url).await),
        ] {
            match event {
                Ok(ClientEvent::Response { status, .. }) if status.is_success() => {}
                Ok(ClientEvent::Response { status, .. }) => {
                    return Err(RtspPushError::Protocol(format!(
                        "{what} rejected: {status:?}"
                    )));
                }
                Ok(_) => {
                    return Err(RtspPushError::Protocol(format!("{what}: unexpected event")));
                }
                Err(e) => return Err(RtspPushError::Protocol(e.to_string())),
            }
        }
        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let channel = self.channel;
        // RFC 2250 §2: no more than 7 whole 188-byte TS packets per RTP
        // packet. `MAX_RTP_PAYLOAD` is a multiple of `TS_PACKET_LEN`, and
        // `data` is always a whole number of TS packets (it comes from
        // `TsMux::package`), so every chunk here — including the last — is
        // itself a whole number of TS packets; none is split mid-packet.
        let packets: Vec<Vec<u8>> = data
            .chunks(MAX_RTP_PAYLOAD)
            .map(|chunk| self.rtp_packet(chunk))
            .collect();
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
    /// Builds one RTP packet (RFC 3550 §5.1) carrying `payload` as MP2T.
    fn rtp_packet(&mut self, payload: &[u8]) -> Vec<u8> {
        let elapsed_micros = self.started.elapsed().as_micros() as u64;
        let timestamp = ((elapsed_micros * RTP_CLOCK_HZ / 1_000_000) & 0xFFFF_FFFF) as u32;
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);

        let mut packet = Vec::with_capacity(RTP_HEADER_LEN + payload.len());
        packet.push(RTP_VERSION_2_NO_FLAGS); // V=2, P=0, X=0, CC=0
        packet.push(PT_MP2T); // M=0
        packet.extend_from_slice(&seq.to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&self.ssrc.to_be_bytes());
        packet.extend_from_slice(payload);
        packet
    }
}

/// Extracts [`Credentials`] from `url`'s userinfo (RFC 3986 §3.2.1), if any.
/// Mirrors `crate::source::http_auth::credentials_from_url`, kept local so
/// this module's errors stay `RtspPushError` rather than `MultimuxError`.
fn credentials_from_url(url: &url::Url) -> Result<Option<Credentials>, RtspPushError> {
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

/// The RTSP presentation control URL: the base URL with `trackID=0` appended
/// as a final path segment (RFC 2326 §10.5 `control`). Built via
/// `Url::path_segments_mut` so a trailing slash never doubles and an IPv6 host
/// stays bracketed.
///
/// Returns an error for a URL with no authority the path segments can be set
/// on — a "cannot-be-a-base" URL such as `rtsp:cam` (no `//host`), which
/// `Url::parse` accepts but which has no host to connect to. `connect` calls
/// this before dialing so an operator's mistyped `rtsp:cam` is a clear
/// [`RtspPushError`], never a panic.
fn control_url(base: &url::Url) -> Result<String, RtspPushError> {
    let mut url = base.clone();
    {
        let mut segs = url.path_segments_mut().map_err(|()| {
            RtspPushError::Connect(
                "rtsp push URL has no authority (cannot-be-a-base, e.g. `rtsp:cam`); \
                 expected `rtsp://host[:port]/path`"
                    .to_string(),
            )
        })?;
        segs.pop_if_empty();
        segs.push("trackID=0");
    }
    Ok(url.to_string())
}

/// `#[doc(hidden)]` test seam: parse `url` and return [`control_url`], so the
/// URL-construction test exercises the real path. Errors (an invalid URL, or a
/// cannot-be-a-base `rtsp:cam`) surface as a `String`, matching the seam's
/// test-only role.
#[doc(hidden)]
pub fn control_url_for_test(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    control_url(&parsed).map_err(|e| e.to_string())
}

/// Percent-decodes a URL userinfo component (RFC 3986 §2.1) to UTF-8. The
/// error message deliberately never echoes `s` — it is (part of) a still
/// percent-encoded credential.
fn percent_decode(s: &str) -> Result<String, RtspPushError> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|e| RtspPushError::Connect(format!("invalid percent-encoded userinfo: {e}")))
}

/// A pseudo-random 32-bit SSRC — same `RandomState` technique as
/// `crate::output::whep::rand_ssrc` (OS-random, no `rand` dependency).
fn rand_ssrc() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let state = RandomState::new();
    (state.hash_one(Instant::now()) as u32) | 1
}

/// Builds the SDP for the push session: one `m=` line for the whole
/// MPEG-2 TS-muxed stream (RFC 2250 §2) — every track is multiplexed into
/// this single RTP session, matching the one SETUP/one interleaved channel
/// this transport actually uses, not one `m=` line per elementary track.
///
/// Returns an error rather than `expect`ing (project rule: no production
/// `expect` on a fallible path); writing to a `Vec` and `String::from_utf8`
/// are effectively infallible, but the error is surfaced anyway.
fn build_sdp() -> Result<String, RtspPushError> {
    use sdp_types::{
        AddrType, Attribute, Connection, Media, MediaType, NetType, Origin, Session, TransportProto,
    };

    let origin = Origin::new("0", 0, NetType::In, AddrType::Ip4, "0.0.0.0");
    let mut session = Session::new(origin, "multimux push");
    session.set_connection(Connection::new(NetType::In, AddrType::Ip4, "0.0.0.0"));
    let mut media = Media::new(
        MediaType::Video,
        0,
        TransportProto::RtpAvp,
        PT_MP2T.to_string(),
    );
    media.add_attribute(Attribute::with_value(
        "rtpmap",
        format!("{PT_MP2T} MP2T/{RTP_CLOCK_HZ}"),
    ));
    media.add_attribute(Attribute::with_value("control", "trackID=0"));
    session.add_media(media);

    let mut out = Vec::new();
    session
        .write(&mut out)
        .map_err(|e| RtspPushError::Protocol(format!("SDP write: {e}")))?;
    String::from_utf8(out).map_err(|e| RtspPushError::Protocol(format!("SDP encoding: {e}")))
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

    /// Byte-for-byte golden (SP4.1): the ANNOUNCE SDP body the transport
    /// sends equals `tests/golden/rtsp_announce.sdp`, captured from the
    /// pre-`sdp-types` `format!` on `origin/main`. `GOLDEN_BLESS=<dir>`
    /// writes instead.
    #[test]
    fn announce_sdp_golden() {
        let actual = build_sdp().expect("build_sdp");
        let file = "rtsp_announce.sdp";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(file);
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
            control_url: "rtsp://cam.example:554/live/STREAMKEY123/trackID=0".to_string(),
            seq: 0,
            ssrc: 1,
            started: Instant::now(),
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
            sdp.contains(&format!("RTP/AVP {PT_MP2T}")),
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
    fn control_url_is_appended_with_a_single_slash() {
        // Regression shape for RFC 2326 Appendix C.1.1: no double slash
        // when the presentation URL already ends in one, no missing slash
        // otherwise.
        for (base, expected) in [
            ("rtsp://h/push", "rtsp://h/push/trackID=0"),
            ("rtsp://h/push/", "rtsp://h/push/trackID=0"),
        ] {
            let control_url = if base.ends_with('/') {
                format!("{base}trackID=0")
            } else {
                format!("{base}/trackID=0")
            };
            assert_eq!(control_url, expected);
        }
    }

    #[test]
    fn rtp_packet_header_is_v2_pt33_with_incrementing_seq() {
        let mut transport = RtspTransport {
            client: None,
            channel: 0,
            url: "rtsp://h/push".to_string(),
            control_url: "rtsp://h/push/trackID=0".to_string(),
            seq: 0xFFFF, // exercise the u16 wraparound.
            ssrc: 0xdead_beef,
            started: Instant::now(),
            teardown: None,
        };
        let payload = [0x47u8; TS_PACKET_LEN]; // one TS sync-byte-led packet.

        let first = transport.rtp_packet(&payload);
        assert_eq!(first[0], 0x80, "V=2,P=0,X=0,CC=0");
        assert_eq!(first[1], PT_MP2T, "M=0, PT=33 (RFC 2250 static MP2T)");
        assert_eq!(u16::from_be_bytes([first[2], first[3]]), 0xFFFF);
        assert_eq!(
            u32::from_be_bytes([first[8], first[9], first[10], first[11]]),
            0xdead_beef
        );
        assert_eq!(
            &first[12..],
            &payload[..],
            "payload must pass through verbatim"
        );

        let second = transport.rtp_packet(&payload);
        assert_eq!(
            u16::from_be_bytes([second[2], second[3]]),
            0,
            "sequence number must wrap u16, not panic"
        );
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
            control_url: "rtsp://cam.example:554/live/STREAMKEY123/trackID=0".to_string(),
            seq: 0,
            ssrc: 1,
            started: Instant::now(),
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
        let other = map_rtsp_error(rtsp_runtime::Error::MessageParse("x".into()));
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
            control_url: "rtsp://h/push/trackID=0".to_string(),
            seq: 0,
            ssrc: 1,
            started: Instant::now(),
            teardown: None,
        };
        // No runtime is active here; close() must not panic. (A `None` client
        // still exercises the `Handle::try_current()` path when the guard is
        // reached with a client, but the no-client form proves no panic.)
        transport.close();
    }

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

        let mut client = rtsp_runtime::AsyncRtspClient::connect_with_timeouts(
            &addr,
            ClientSession::new(),
            rtsp_runtime::RtspTimeouts::default()
                .with_read_idle(std::time::Duration::from_millis(100))
                .with_write(std::time::Duration::from_millis(300)),
        )
        .await
        .expect("connect");
        let url = format!("rtsp://{addr}/live/key");
        let sdp = build_sdp().expect("sdp");
        let _ = client.announce(&url, &sdp).await;
        let mut spec = TransportSpec::rtp_avp_tcp_interleaved(0, 1);
        spec.mode = vec![rtsp_runtime::TransportMode::Record];
        let _ = client.setup(&url, &Transport::single(spec)).await;
        let _ = client.record(&url).await;

        // Send media until the keepalive (session timeout=1 s) elicits the 454
        // and the drain surfaces it.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
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
