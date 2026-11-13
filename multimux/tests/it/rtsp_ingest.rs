//! Deterministic loopback e2e test for the RTSP ingest path
//! (`multimux::source::rtsp`, issue #705 item 3), retargeted (plan step 5b)
//! from the pre-5a `RtspSource` onto `run_rtsp` and the shared session host (see
//! `multimux/src/source/rtsp.rs`'s module doc): a real `127.0.0.1:0` TCP socket, DESCRIBE -> SETUP -> PLAY, then
//! a few interleaved RTP packets — exercising the real socket -> SDP-parse ->
//! interleaved-depayload path with no live network and no flakiness (every
//! step is synchronized by the request/response handshake itself, and the
//! whole exchange is bounded by an outer timeout so a wiring bug fails fast
//! instead of hanging CI).
//!
//! # Why a hand-rolled server, not `rtsp_runtime::AsyncRtspServer`
//!
//! `ServerSession::handle_request` (rtsp-runtime/src/server.rs) always builds
//! a fixed, empty-body response — there is no way to hand it a custom DESCRIBE
//! body. This test needs a real, parseable SDP (carrying
//! `sprop-parameter-sets`) to build the AVC config, so a minimal raw-TCP
//! responder is used instead: it accepts the socket, reads each request,
//! and writes byte-correct `RTSP/1.0` responses (echoing `CSeq`) — DESCRIBE
//! gets a `Content-Type: application/sdp` body, SETUP negotiates interleaved
//! TCP on channel 0-1, PLAY is a bare 200. This still drives
//! `rtsp-runtime`'s real client parser (`ClientSession`) against real
//! response bytes, so the client-side wire path is exercised for real; only
//! the server side is hand-written (rtsp-runtime has no server-side
//! body-injection hook to reuse).
//!
//! The client under test is the production one: `run_rtsp`, which runs a relay
//! `broadcast_client::Session` through the shared session host. The resolved track set is read
//! back from the route's registry (`RouteHandle::track_specs`) and the samples from the
//! published `Trunk`.
//!
//! No feature gate is needed: `multimux::source::rtsp` is not behind any
//! Cargo feature (only `rtsps://` TLS support is, and this test uses plain
//! `rtsp://`).

use std::sync::Arc;
use std::time::Duration;
use transmux::TrackId;

use broadcast_auth::{AuthResult, RequestContext, Verifier};
use broadcast_common::Timestamp;
use media_plane::ingress::{HandshakePolicy, ProgramId};
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use multimux::source::IngestTimeouts;
use multimux::source::rtsp::{RtspRoute, run_rtsp};
use multimux::{MultimuxError, RouteHandle};
use rtsp_runtime::{Credentials, InterleavedFrame, TransportSpec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use transmux::TrackSpec;
use transmux::pipeline::{CodecConfig, Sample};

use crate::support::shared_it::nz;

/// A known-good H.264 `sprop-parameter-sets` (SPS + PPS) — the same vector
/// `avc_config_from_sprop`/`avc_config_from_fmtp` are unit-tested against
/// elsewhere in this workspace (transmux/src/rtp_sdp.rs,
/// multimux/src/source/{rtsp,sdp}.rs, multimux/tests/it/origin_llhls.rs).
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

/// The interleaved RTP channel SETUP negotiates for the sole video media
/// (RTCP rides the paired odd channel, unused by this test).
const RTP_CHANNEL: u8 = 0;

/// HANG GUARD (issue #807): bound on the whole DESCRIBE->SETUP->PLAY->depayload
/// exchange over real loopback TCP -- a wiring bug (e.g. a response the
/// client can't parse) must fail fast, not hang CI. Every step here is
/// synchronized by the handshake itself (no arbitrary sleeps to race), so
/// this normally completes in well under a second; raised from 5s to 60s for
/// load tolerance since it is not a timing claim.
const TEST_TIMEOUT: Duration = Duration::from_secs(60);

const RING: usize = 16;
const TIMED_RING: usize = 4096;

/// Minimal single-media H.264 SDP, parseable by
/// `multimux::source::sdp::parse_sdp_tracks`.
fn sdp_body() -> Vec<u8> {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         t=0 0\r\n\
         m=video 0 RTP/AVP 96\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 packetization-mode=1;sprop-parameter-sets={SPROP}\r\n\
         a=control:streamid=0\r\n\
         m=application 0 RTP/AVP 100\r\n\
         a=rtpmap:100 vnd.onvif.metadata/90000\r\n\
         a=control:streamid=meta\r\n"
    )
    .into_bytes()
}

/// Builds a minimal single-NAL-unit-mode H.264 RTP packet: the RFC 3550 §5.1
/// fixed 12-byte header (V=2, no padding/extension/CSRC) followed by one NAL
/// unit verbatim (RFC 6184 §5.1).
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

/// Reads off `sock` until one complete RTSP request (headers only — none of
/// DESCRIBE/SETUP/PLAY carry a request body from this client) is buffered,
/// returning its text and `CSeq`.
async fn read_request(sock: &mut TcpStream) -> (String, u32) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let text = String::from_utf8_lossy(&buf[..pos]).to_string();
            let cseq = text
                .lines()
                .find_map(|l| l.strip_prefix("CSeq:"))
                .map(|v| v.trim().parse().expect("valid CSeq"))
                .expect("request carries a CSeq header");
            return (text, cseq);
        }
        let n = sock.read(&mut chunk).await.expect("read request bytes");
        assert!(n > 0, "peer closed before a full request arrived");
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn write_all(sock: &mut TcpStream, bytes: &[u8]) {
    sock.write_all(bytes).await.expect("write response");
    sock.flush().await.expect("flush response");
}

/// Looks up a header's value by exact (case-sensitive) name in a headers-only
/// request/response text blob, as returned by `read_request`. `rtsp-types`
/// emits header names in the fixed casing `Authorization` /
/// `WWW-Authenticate` (RFC 2326 §12.5/§12.44), so an exact match is enough.
fn header_value<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
}

/// Base64-encodes `user:pass` (RFC 7617 §2 `basic-credentials`), for
/// comparing against the `Authorization` header value the client under test
/// actually sends.
fn basic_credentials(user: &str, pass: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
}

/// One running pull: the route's registry handle (resolved tracks, published `Trunk`), the cancel
/// token that stops it, and the task whose result is the error that ended it.
struct Pull {
    route_handle: Arc<RouteHandle>,
    cancel: CancellationToken,
    run: tokio::task::JoinHandle<MultimuxError>,
}

fn start_pull(url: String, auth: Option<Credentials>) -> Pull {
    let route = RtspRoute::new("rtsp-ingest", url)
        .with_auth(auth)
        .with_timeouts(IngestTimeouts {
            connect: TEST_TIMEOUT,
            read: TEST_TIMEOUT,
        });
    let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
    let config = TrunkConfig::new(nz(TIMED_RING), nz(RING), nz(RING), nz(RING), nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let route_handle = Arc::clone(&route_handle);
        let cancel = cancel.clone();
        async move { run_rtsp(&route, config, handshake, &route_handle, cancel).await }
    });
    Pull {
        route_handle,
        cancel,
        run,
    }
}

impl Pull {
    async fn samples(&self, want: usize) -> Vec<(TrackId, Sample)> {
        let trunk = self.route_handle.await_first_trunk().await;
        let mut cursor = trunk.subscribe_from_backlog();
        let mut samples = Vec::new();
        while samples.len() < want {
            match cursor.poll() {
                Some(SampleCursorItem::Timed { track_id, sample }) => {
                    samples.push((track_id, sample));
                }
                Some(_) => {}
                None => tokio::task::yield_now().await,
            }
        }
        samples
    }

    async fn tracks(&self) -> Vec<TrackSpec> {
        self.route_handle.await_first_trunk().await;
        self.route_handle.track_specs(ProgramId(0))
    }

    async fn stop(self) {
        self.cancel.cancel();
        let _ = self.run.await;
    }

    async fn failure(self) -> MultimuxError {
        self.run.await.expect("the pull task must not panic")
    }
}

/// Runs the hand-rolled RTSP server side: DESCRIBE (SDP body) -> SETUP
/// (interleaved TCP transport) -> PLAY -> three interleaved RTP access units
/// (IDR, then two non-IDR, at distinct 90 kHz timestamps with the marker bit
/// set on each so the streaming depayloader can close every access unit).
async fn serve_one_session(sock: TcpStream) {
    serve_session_with_frames(sock, false).await;
}

/// [`serve_one_session`], optionally with a corrupt interleaved frame (a valid
/// `$` frame whose RTP payload is 5 bytes, shorter than any RTP header) in the
/// middle of the media, and a fourth access unit after it.
async fn serve_session_with_frames(mut sock: TcpStream, corrupt_middle: bool) {
    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("DESCRIBE"), "expected DESCRIBE, got: {req}");
    let sdp = sdp_body();
    let describe_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
        sdp.len()
    );
    write_all(&mut sock, describe_resp.as_bytes()).await;
    write_all(&mut sock, &sdp).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("SETUP"), "expected SETUP, got: {req}");
    let transport = TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1)
        .to_header_value()
        .unwrap();
    let setup_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000001\r\nTransport: {transport}\r\n\r\n"
    );
    write_all(&mut sock, setup_resp.as_bytes()).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("PLAY"), "expected PLAY, got: {req}");
    let play_resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000001\r\n\r\n");
    write_all(&mut sock, play_resp.as_bytes()).await;

    // AU0 @1000 (IDR), AU1 @4000 (non-IDR), AU2 @7000 (non-IDR): 3000-tick
    // spacing at the SDP's 90 kHz clock rate. The streaming depayloader only
    // knows a sample's duration once the *next* AU's timestamp has arrived,
    // so 3 AUs yield exactly 2 completed samples (AU2 stays pending until a
    // flush, which this test never triggers).
    let idr = [0x65u8, 0xAA, 0xBB]; // nal_ref_idc=3, type=5 (IDR slice)
    let non1 = [0x41u8, 0xAA, 0xBB]; // nal_ref_idc=2, type=1 (non-IDR slice)
    let non2 = [0x41u8, 0xCC, 0xDD];
    let non3 = [0x41u8, 0xEE, 0xFF];
    let aus: [(u32, &[u8]); 4] = [(1000, &idr), (4000, &non1), (7000, &non2), (10_000, &non3)];
    let n_aus = if corrupt_middle { 4 } else { 3 };
    for (i, (ts, nal)) in aus.into_iter().take(n_aus).enumerate() {
        if corrupt_middle && i == 2 {
            let junk = InterleavedFrame::new(RTP_CHANNEL, vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01])
                .to_bytes()
                .expect("frame fits the u16 length field");
            write_all(&mut sock, &junk).await;
        }
        let pkt = rtp_packet(1 + i as u16, ts, true, nal);
        let frame = InterleavedFrame::new(RTP_CHANNEL, pkt)
            .to_bytes()
            .expect("frame fits the u16 length field");
        write_all(&mut sock, &frame).await;
    }

    // Keep the connection open until the client has drained every frame; the
    // client side runs concurrently and its assertions are what actually gate
    // the test (bounded by the outer `TEST_TIMEOUT`), so this is just cleanup,
    // not a synchronization sleep.
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn rtsp_ingest_loopback_describe_setup_play_depayload() {
    tokio::time::timeout(TEST_TIMEOUT, run())
        .await
        .expect("rtsp ingest e2e timed out — client/server wiring did not complete");
}

async fn run() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.expect("accept");
        serve_one_session(sock).await;
    });

    let pull = start_pull(format!("rtsp://{addr}/test"), None);
    let samples = pull.samples(2).await;
    let tracks = pull.tracks().await;

    // The DESCRIBE SDP was really parsed and a real avcC built (not an empty
    // placeholder): one video track at the RTP clock rate, carrying the
    // sprop's SPS + PPS.
    assert_eq!(tracks.len(), 1, "one video track from the SDP");
    // NOTE: this does not by itself prove the `a=rtpmap` clock rate was parsed
    // (rather than defaulted): H.264's RTP clock is fixed at 90 kHz per RFC 6184
    // §8.2.1, which coincides with the crate's `DEFAULT_CLOCK_RATE_HZ`, so a
    // video-only conformant fixture can't distinguish the two. `rtpmap_clock_rate`
    // parsing is bitten separately in transmux's `rtp_sdp` unit tests.
    assert_eq!(
        tracks[0].timescale, 90_000,
        "RTP clock rate becomes the IR timescale"
    );
    match &tracks[0].config {
        CodecConfig::Avc { config, .. } => {
            assert!(
                !config.config.sps.is_empty(),
                "avcC must carry the sprop's SPS, not an empty placeholder"
            );
            assert!(
                !config.config.pps.is_empty(),
                "avcC must carry the sprop's PPS, not an empty placeholder"
            );
        }
        other => panic!("expected CodecConfig::Avc from the H.264 SDP, got {other:?}"),
    }

    assert_eq!(
        samples.len(),
        2,
        "3 access units over interleaved RTP yield exactly 2 completed samples"
    );
    let (track_id0, sample0) = &samples[0];
    assert_eq!(
        *track_id0,
        TrackId::new(1),
        "the sole video track routes to track id 1"
    );
    // The same `transmux::Sample` a `Trunk`'s `SampleCursorItem::Timed` would
    // hand back — see this module's doc for why this test reads it here
    // instead.
    assert!(sample0.flags.is_sync, "the first access unit was the IDR");
    let (track_id1, sample1) = &samples[1];
    assert_eq!(*track_id1, TrackId::new(1));
    assert!(!sample1.flags.is_sync, "the second access unit was non-IDR");

    pull.stop().await;
    server.await.expect("server task");
}

#[tokio::test]
async fn corrupt_interleaved_rtp_frame_is_skipped_and_the_session_continues() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            serve_session_with_frames(sock, true).await;
        });
        let pull = start_pull(format!("rtsp://{addr}/test"), None);

        // 4 access units + 1 corrupt frame between AU1 and AU2: the three
        // completed samples are the same as an undamaged 4-AU stream yields.
        let samples = pull.samples(3).await;
        let sync: Vec<bool> = samples.iter().map(|(_, s)| s.flags.is_sync).collect();
        assert_eq!(sync, vec![true, false, false], "IDR then two non-IDR");
        pull.stop().await;
        server.await.expect("server task");
    })
    .await
    .expect("timed out");
}

/// Runs a hand-rolled RTSP server that requires Basic auth on DESCRIBE (RFC
/// 2326 §14 / §16, sharing HTTP's schemes verbatim; RFC 7617 for Basic): a
/// DESCRIBE without an `Authorization` header gets `401 Unauthorized` +
/// `WWW-Authenticate: Basic realm="mock"`; the client's transparently-retried
/// DESCRIBE is checked for a Basic `Authorization` header matching
/// `expect_user`/`expect_pass` before the SDP/SETUP/PLAY handshake completes
/// exactly like `serve_one_session` above (minus the RTP frames — this test
/// only checks that the handshake succeeds, not depayloading).
async fn serve_one_session_requiring_auth(
    mut sock: TcpStream,
    expect_user: &str,
    expect_pass: &str,
) {
    let (req, cseq) = read_request(&mut sock).await;
    assert!(
        req.starts_with("DESCRIBE"),
        "expected first (unauthenticated) DESCRIBE, got: {req}"
    );
    assert!(
        header_value(&req, "Authorization").is_none(),
        "the first DESCRIBE must not carry an Authorization header: {req}"
    );
    let challenge_resp = format!(
        "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: Basic realm=\"mock\"\r\n\r\n"
    );
    write_all(&mut sock, challenge_resp.as_bytes()).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(
        req.starts_with("DESCRIBE"),
        "expected the retried DESCRIBE, got: {req}"
    );
    let auth = header_value(&req, "Authorization")
        .expect("the retried DESCRIBE must carry an Authorization header");
    let expected = format!("Basic {}", basic_credentials(expect_user, expect_pass));
    assert_eq!(
        auth, expected,
        "Authorization must encode the URL userinfo's username/password"
    );

    let sdp = sdp_body();
    let describe_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
        sdp.len()
    );
    write_all(&mut sock, describe_resp.as_bytes()).await;
    write_all(&mut sock, &sdp).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("SETUP"), "expected SETUP, got: {req}");
    let transport = TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1)
        .to_header_value()
        .unwrap();
    let setup_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000002\r\nTransport: {transport}\r\n\r\n"
    );
    write_all(&mut sock, setup_resp.as_bytes()).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("PLAY"), "expected PLAY, got: {req}");
    let play_resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000002\r\n\r\n");
    write_all(&mut sock, play_resp.as_bytes()).await;
}

/// Credentials embedded in the `rtsp://user:pass@host/...` URL's userinfo
/// (RFC 3986 §3.2.1) must flow through to a Basic `Authorization` header
/// answering the server's `401` challenge, and the request line sent on the
/// wire must not carry `user:pass@` (verified indirectly: the hand-rolled
/// server only ever sees `DESCRIBE`/`SETUP`/`PLAY` request lines built from
/// the route, and a URL still carrying userinfo would fail
/// `rtsp_types::Url::parse` inside `ClientSession::assemble`, so a request
/// ever reaching the server at all already proves the stripped form was
/// used).
///
/// Reverting the `rtsp.rs` fix (no credentials attached to `ClientSession`)
/// makes this test fail: the client would never answer the 401, `feed()`
/// would surface it as an error, and the handshake wait's `.expect(...)`
/// would panic.
#[tokio::test]
async fn rtsp_ingest_basic_auth_from_url_userinfo_succeeds() {
    tokio::time::timeout(TEST_TIMEOUT, run_auth_success())
        .await
        .expect("rtsp basic-auth e2e timed out — auth wiring did not complete");
}

async fn run_auth_success() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.expect("accept");
        serve_one_session_requiring_auth(sock, "camuser", "camsecret").await;
    });

    let pull = start_pull(format!("rtsp://camuser:camsecret@{addr}/test"), None);
    let tracks = tokio::time::timeout(TEST_TIMEOUT, pull.tracks())
        .await
        .expect("handshake must complete");
    assert_eq!(
        tracks.len(),
        1,
        "the SDP was parsed after the authenticated DESCRIBE succeeded"
    );
    pull.stop().await;

    server.await.expect("server task");
}

/// The same server (requiring Basic auth) against the same URL *minus* its
/// userinfo: with no credentials configured, the client cannot answer the
/// `401` challenge, so the handshake must fail rather than silently proceed.
///
/// This is the counterpart that proves the auth-success test above is really
/// exercising credential flow and not e.g. a server that accepts anything:
/// the exact same server rejects an unauthenticated client.
#[tokio::test]
async fn rtsp_ingest_missing_url_credentials_fails_with_401() {
    tokio::time::timeout(TEST_TIMEOUT, run_no_credentials_fails())
        .await
        .expect("rtsp no-auth e2e timed out — server never got a request");
}

async fn run_no_credentials_fails() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let (req, cseq) = read_request(&mut sock).await;
        assert!(req.starts_with("DESCRIBE"), "expected DESCRIBE, got: {req}");
        assert!(
            header_value(&req, "Authorization").is_none(),
            "an unauthenticated client must not send an Authorization header: {req}"
        );
        let challenge_resp = format!(
            "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: Basic realm=\"mock\"\r\n\r\n"
        );
        write_all(&mut sock, challenge_resp.as_bytes()).await;
        // No credentials configured => the client does not retry; this is the
        // whole exchange.
    });

    let pull = start_pull(format!("rtsp://{addr}/test"), None);
    let err = tokio::time::timeout(TEST_TIMEOUT, pull.failure())
        .await
        .expect("the handshake must fail, not hang");
    assert!(
        matches!(&err, MultimuxError::Auth { .. }),
        "a refused DESCRIBE credential must surface as the typed Auth error, got: {err:?}"
    );

    server.await.expect("server task");
}

// ---------------------------------------------------------------------------
// Config-supplied (`RtspRoute::with_auth`) Digest auth — the gap
// flagged in the client-auth story: config-supplied credentials (as opposed
// to URL userinfo, which `rtsp_ingest_basic_auth_from_url_userinfo_succeeds`
// above already covers) needed a test driving them against a real server.
// Mirrors `rtsp-runtime/tests/it/io_loopback.rs::digest_auth_over_loopback`, but
// for multimux's own RTSP ingest and through the full DESCRIBE->SETUP->PLAY
// handshake, not just a single DESCRIBE round trip. The mock server's Digest
// verification is the real, production `broadcast_auth::Verifier` (issue
// #663 "shared output auth" promoted this out of a hand-rolled test double
// in `multimux::testutil`), so a wrong password genuinely fails here rather
// than passing a literal-string check.
// ---------------------------------------------------------------------------

/// Extracts the request-URI (the second, space-delimited token of the
/// request line) from a request's raw text, as captured by [`read_request`]
/// — needed to verify the client's Digest `uri=` field against the actual
/// request-URI it sent (RFC 7616 §3.4.1).
fn request_uri(request_text: &str) -> &str {
    request_text
        .lines()
        .next()
        .expect("non-empty request")
        .split_whitespace()
        .nth(1)
        .expect("request line carries a request-URI")
}

/// Runs a hand-rolled RTSP server requiring Digest auth (RFC 2326 §14 / RFC
/// 7616) on DESCRIBE, verified by the real `verifier` — see this section's
/// module-level doc comment for why a real `Verifier` rather than a literal
/// comparison. Completes the SDP/SETUP/PLAY handshake once a correctly
/// authenticated DESCRIBE arrives; re-challenges (and stops) on a failed
/// retry, since the client under test never retries a second time.
async fn serve_one_session_requiring_digest_auth(mut sock: TcpStream, verifier: &Verifier) {
    let (req, cseq) = read_request(&mut sock).await;
    assert!(
        req.starts_with("DESCRIBE"),
        "expected first (unauthenticated) DESCRIBE, got: {req}"
    );
    assert!(
        header_value(&req, "Authorization").is_none(),
        "the first DESCRIBE must not carry an Authorization header: {req}"
    );
    let challenge_resp = format!(
        "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: {}\r\n\r\n",
        verifier.challenge()
    );
    write_all(&mut sock, challenge_resp.as_bytes()).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(
        req.starts_with("DESCRIBE"),
        "expected the retried DESCRIBE, got: {req}"
    );
    let auth = header_value(&req, "Authorization")
        .expect("the retried DESCRIBE must carry an Authorization header");
    assert!(
        auth.starts_with("Digest "),
        "retry must carry a Digest Authorization: {auth}"
    );
    let uri = request_uri(&req);
    let auth_header: &[(&str, &str)] = &[("Authorization", auth)];
    let ctx = RequestContext::new("DESCRIBE", uri).with_headers(auth_header);
    let verified = verifier.verify(&ctx);

    if verified != AuthResult::Ok {
        // Wrong credentials: re-challenge (matching a real Digest origin's
        // behaviour on a failed retry) and stop — the client under test
        // does not retry a second time, so this is the whole exchange.
        let reject_resp = format!(
            "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: {}\r\n\r\n",
            verifier.challenge()
        );
        write_all(&mut sock, reject_resp.as_bytes()).await;
        return;
    }

    let sdp = sdp_body();
    let describe_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
        sdp.len()
    );
    write_all(&mut sock, describe_resp.as_bytes()).await;
    write_all(&mut sock, &sdp).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("SETUP"), "expected SETUP, got: {req}");
    let transport = TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1)
        .to_header_value()
        .unwrap();
    let setup_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000003\r\nTransport: {transport}\r\n\r\n"
    );
    write_all(&mut sock, setup_resp.as_bytes()).await;

    let (req, cseq) = read_request(&mut sock).await;
    assert!(req.starts_with("PLAY"), "expected PLAY, got: {req}");
    let play_resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000003\r\n\r\n");
    write_all(&mut sock, play_resp.as_bytes()).await;
}

/// The config-supplied Digest path, driven against a real server: a URL
/// carrying **no** userinfo, plus credentials passed straight to
/// `RtspRoute::with_auth`, must authenticate and succeed — proving the config path
/// independently of the URL-userinfo path already covered above. Reverting
/// the config-credentials -> `ClientSession::with_credentials` wiring in
/// `source::rtsp::session_with_credentials` makes this fail (the client
/// would never answer the 401, and the handshake would surface it as an
/// error).
#[tokio::test]
async fn rtsp_ingest_config_digest_auth_succeeds() {
    tokio::time::timeout(TEST_TIMEOUT, run_config_digest_auth_success())
        .await
        .expect("rtsp config digest-auth e2e timed out — auth wiring did not complete");
}

async fn run_config_digest_auth_success() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let verifier = Verifier::new(
        Credentials::Digest {
            username: "camuser".into(),
            password: "camsecret".into(),
        },
        "IP Camera",
    );
    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.expect("accept");
        serve_one_session_requiring_digest_auth(sock, &verifier).await;
    });

    // No userinfo in the URL at all — credentials come ONLY from the dialer's
    // `auth` parameter.
    let pull = start_pull(
        format!("rtsp://{addr}/test"),
        Some(Credentials::new("camuser", "camsecret")),
    );
    let tracks = tokio::time::timeout(TEST_TIMEOUT, pull.tracks())
        .await
        .expect("handshake must complete");
    assert_eq!(
        tracks.len(),
        1,
        "the SDP was parsed after the authenticated DESCRIBE succeeded"
    );
    pull.stop().await;

    server.await.expect("server task");
}

/// The same config-supplied Digest path, but with the wrong password: the
/// real `Verifier` must genuinely reject it, and the handshake must fail
/// (never falling back to proceeding unauthenticated).
#[tokio::test]
async fn rtsp_ingest_config_digest_auth_wrong_password_fails() {
    tokio::time::timeout(TEST_TIMEOUT, run_config_digest_auth_wrong_password())
        .await
        .expect("rtsp config digest-auth (wrong password) e2e timed out");
}

async fn run_config_digest_auth_wrong_password() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let verifier = Verifier::new(
        Credentials::Digest {
            username: "camuser".into(),
            password: "camsecret".into(),
        },
        "IP Camera",
    );
    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.expect("accept");
        serve_one_session_requiring_digest_auth(sock, &verifier).await;
    });

    let pull = start_pull(
        format!("rtsp://{addr}/test"),
        Some(Credentials::new("camuser", "WRONGPASSWORD")),
    );
    let err = tokio::time::timeout(TEST_TIMEOUT, pull.failure())
        .await
        .expect("the handshake must fail, not hang");
    assert!(
        matches!(&err, MultimuxError::Auth { .. }),
        "a refused DESCRIBE credential must surface as the typed Auth error, got: {err:?}"
    );

    server.await.expect("server task");
}
