//! Deterministic loopback e2e test for the RTSP ingest path
//! (`multimux::source::rtsp`, issue #705 item 3), retargeted (plan step 5b)
//! from the pre-5a `RtspSource` onto `RtspDialer`/`RtspIngestSession` (step
//! 5a — see `multimux/src/source/rtsp.rs`'s module doc for the sans-IO
//! reshape): a real `127.0.0.1:0` TCP socket, DESCRIBE -> SETUP -> PLAY, then
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
//! # Why this drives `RtspIngestSession` directly rather than through
//! `IngestDriver`
//!
//! `media_plane::ingress::IngestDriver` (the production wrapper
//! `crate::source::rtsp::run_rtsp` uses) deliberately does not retain a
//! `SessionEvent::NewProgram`'s track list once it has decided whether to
//! admit the program (`IngestDriver::drain` matches `NewProgram { program,
//! .. }` — the tracks are read only to gate `max_programs`, then dropped);
//! there is no accessor on `IngestDriver`/`Trunk` that recovers them
//! afterwards. This test's whole point (per the pre-5a doc above) is
//! asserting on the *resolved track set* — a real avcC built from the SDP's
//! `sprop-parameter-sets`, and the RTP clock rate becoming the track
//! timescale — so it drives `RtspIngestSession`'s own `Stage`/`IngestSession`
//! implementation directly instead: exactly the `rtsp.rs` module's own
//! `multi_round_trip_rtsp_handshake_completes_through_feed_and_poll_transmit_only`
//! test, just replacing its canned response bytes with a genuine socket and
//! hand-rolled server. Every `SessionEvent` (`NewProgram` included) is
//! observable this way, so every original assertion is preserved.
//! `SessionEvent::Sample`'s payload is the same `transmux::Sample` a
//! `Trunk`'s `SampleCursorItem::Timed` would hand back — `IngestDriver`'s
//! dispatch does not transform it — so `sample.flags.is_sync` here is exactly
//! what a caller reading the sample back out of a `Trunk` would see.
//!
//! No feature gate is needed: `multimux::source::rtsp` is not behind any
//! Cargo feature (only `rtsps://` TLS support is, and this test uses plain
//! `rtsp://`).

use std::time::{Duration, Instant};

use broadcast_auth::{AuthResult, RequestContext, Verifier};
use broadcast_common::{Stage, Timestamp};
use media_plane::ingress::{Dialer, IngestSession, SessionEvent};
use multimux::MultimuxError;
use multimux::source::rtsp::{RtspDialer, RtspIngestSession};
use rtsp_runtime::{Credentials, InterleavedFrame, TransportSpec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use transmux::TrackSpec;
use transmux::pipeline::CodecConfig;

/// A known-good H.264 `sprop-parameter-sets` (SPS + PPS) — the same vector
/// `avc_config_from_sprop`/`avc_config_from_fmtp` are unit-tested against
/// elsewhere in this workspace (transmux/src/rtp_sdp.rs,
/// multimux/src/source/{rtsp,sdp}.rs, multimux/tests/origin_llhls.rs).
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

/// Bound on how many read/write rounds [`drive_and_collect`] will attempt —
/// a wiring bug must fail fast rather than spin; `TEST_TIMEOUT` is the real
/// backstop.
const MAX_ROUNDS: usize = 64;

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
         a=control:streamid=0\r\n"
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

/// Extracts the resolved track set from a `NewProgram` event, if one is
/// present in `events`.
fn new_program_tracks(events: &[SessionEvent]) -> Option<&[TrackSpec]> {
    events.iter().find_map(|e| match e {
        SessionEvent::NewProgram { tracks, .. } => Some(tracks.as_slice()),
        _ => None,
    })
}

/// Counts `SessionEvent::Sample` events collected so far.
fn sample_count(events: &[SessionEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, SessionEvent::Sample { .. }))
        .count()
}

/// Drives `session` against a real, already-connected `stream`: writes every
/// queued outbound request, reads the next response/media chunk, feeds it
/// in, and collects every `SessionEvent` produced — mirroring
/// `multimux::source::rtsp::run_rtsp`'s own loop (see this module's doc),
/// just test-owned so every event is directly observable. Stops and returns
/// as soon as `want` is satisfied by the events collected so far, or
/// propagates the first `feed` error (exactly the failure mode `run_rtsp`
/// itself would surface).
async fn drive_and_collect(
    session: &mut RtspIngestSession,
    stream: &mut TcpStream,
    want: impl Fn(&[SessionEvent]) -> bool,
) -> Result<Vec<SessionEvent>, MultimuxError> {
    let start = Instant::now();
    let mut buf = [0u8; 64 * 1024];
    let mut events = Vec::new();
    for _ in 0..MAX_ROUNDS {
        while let Some(bytes) = session.poll_transmit() {
            stream.write_all(&bytes).await.expect("write request");
        }
        while let Some(ev) = session.poll() {
            events.push(ev);
        }
        if want(&events) {
            return Ok(events);
        }
        let n = stream.read(&mut buf).await.expect("read response");
        assert!(n > 0, "peer closed before the expected events arrived");
        let now = Timestamp::from_instant(start, Instant::now());
        session.feed(&buf[..n], now)?;
        while let Some(ev) = session.poll() {
            events.push(ev);
        }
        if want(&events) {
            return Ok(events);
        }
    }
    panic!("drive_and_collect: exceeded {MAX_ROUNDS} rounds without satisfying `want`");
}

/// Runs the hand-rolled RTSP server side: DESCRIBE (SDP body) -> SETUP
/// (interleaved TCP transport) -> PLAY -> three interleaved RTP access units
/// (IDR, then two non-IDR, at distinct 90 kHz timestamps with the marker bit
/// set on each so the streaming depayloader can close every access unit).
async fn serve_one_session(mut sock: TcpStream) {
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
    let transport =
        TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1).to_header_value();
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
    let aus: [(u32, &[u8]); 3] = [(1000, &idr), (4000, &non1), (7000, &non2)];
    for (i, (ts, nal)) in aus.into_iter().enumerate() {
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

    let mut dialer = RtspDialer::new(format!("rtsp://{addr}/test"), None);
    let mut session = dialer
        .dial()
        .expect("dial: local construction only, no I/O");
    let mut stream = TcpStream::connect(addr).await.expect("connect loopback");

    let events = drive_and_collect(&mut session, &mut stream, |evs| sample_count(evs) >= 2)
        .await
        .expect("DESCRIBE->SETUP->PLAY->depayload must succeed");

    // The DESCRIBE SDP was really parsed and a real avcC built (not an empty
    // placeholder): one video track at the RTP clock rate, carrying the
    // sprop's SPS + PPS.
    let tracks = new_program_tracks(&events).expect("PLAY response must announce NewProgram");
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

    let samples: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Sample {
                track_id, sample, ..
            } => Some((*track_id, sample)),
            _ => None,
        })
        .collect();
    assert_eq!(
        samples.len(),
        2,
        "3 access units over interleaved RTP yield exactly 2 completed samples"
    );
    let (track_id0, sample0) = samples[0];
    assert_eq!(track_id0, 1, "the sole video track routes to track id 1");
    // The same `transmux::Sample` a `Trunk`'s `SampleCursorItem::Timed` would
    // hand back — see this module's doc for why this test reads it here
    // instead.
    assert!(sample0.flags.is_sync, "the first access unit was the IDR");
    let (track_id1, sample1) = samples[1];
    assert_eq!(track_id1, 1);
    assert!(!sample1.flags.is_sync, "the second access unit was non-IDR");

    server.await.expect("server task");
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
    let transport =
        TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1).to_header_value();
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
/// `RtspDialer`, and a URL still carrying userinfo would fail
/// `rtsp_types::Url::parse` inside `ClientSession::assemble`, so a request
/// ever reaching the server at all already proves the stripped form was
/// used).
///
/// Reverting the `rtsp.rs` fix (no credentials attached to `ClientSession`)
/// makes this test fail: the client would never answer the 401, `feed()`
/// would surface it as an error, and `drive_and_collect`'s `.expect(...)`
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

    let mut dialer = RtspDialer::new(format!("rtsp://camuser:camsecret@{addr}/test"), None);
    let mut session = dialer.dial().expect("dial: local construction only");
    let mut stream = TcpStream::connect(addr).await.expect("connect loopback");

    let events = drive_and_collect(&mut session, &mut stream, |evs| {
        new_program_tracks(evs).is_some()
    })
    .await
    .expect(
        "handshake must succeed: URL userinfo credentials should answer the server's 401 challenge",
    );
    assert_eq!(
        new_program_tracks(&events).expect("checked above").len(),
        1,
        "the SDP was parsed after the authenticated DESCRIBE succeeded"
    );

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

    let mut dialer = RtspDialer::new(format!("rtsp://{addr}/test"), None);
    let mut session = dialer.dial().expect("dial: local construction only");
    let mut stream = TcpStream::connect(addr).await.expect("connect loopback");

    let err = drive_and_collect(&mut session, &mut stream, |_| false)
        .await
        .expect_err("handshake must fail: no credentials to answer the server's 401 challenge");
    let msg = err.to_string();
    assert!(
        msg.contains("401") || msg.to_ascii_lowercase().contains("unauthorized"),
        "error should report the 401/Unauthorized DESCRIBE status, got: {msg}"
    );

    server.await.expect("server task");
}

// ---------------------------------------------------------------------------
// Config-supplied (`RtspDialer::new`'s `auth` parameter) Digest auth — the gap
// flagged in the client-auth story: config-supplied credentials (as opposed
// to URL userinfo, which `rtsp_ingest_basic_auth_from_url_userinfo_succeeds`
// above already covers) needed a test driving them against a real server.
// Mirrors `rtsp-runtime/tests/io_loopback.rs::digest_auth_over_loopback`, but
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
    let transport =
        TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1).to_header_value();
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
/// `RtspDialer::new`, must authenticate and succeed — proving the config path
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
    let mut dialer = RtspDialer::new(
        format!("rtsp://{addr}/test"),
        Some(Credentials::new("camuser", "camsecret")),
    );
    let mut session = dialer.dial().expect("dial: local construction only");
    let mut stream = TcpStream::connect(addr).await.expect("connect loopback");

    let events = drive_and_collect(&mut session, &mut stream, |evs| {
        new_program_tracks(evs).is_some()
    })
    .await
    .expect(
        "handshake must succeed: config-supplied Digest credentials should answer the \
         server's 401 challenge",
    );
    assert_eq!(
        new_program_tracks(&events).expect("checked above").len(),
        1,
        "the SDP was parsed after the authenticated DESCRIBE succeeded"
    );

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

    let mut dialer = RtspDialer::new(
        format!("rtsp://{addr}/test"),
        Some(Credentials::new("camuser", "WRONGPASSWORD")),
    );
    let mut session = dialer.dial().expect("dial: local construction only");
    let mut stream = TcpStream::connect(addr).await.expect("connect loopback");

    let err = drive_and_collect(&mut session, &mut stream, |_| false)
        .await
        .expect_err("handshake must fail: wrong password must not authenticate");
    let msg = err.to_string();
    assert!(
        msg.contains("401") || msg.to_ascii_lowercase().contains("unauthorized"),
        "error should report the 401/Unauthorized DESCRIBE status, got: {msg}"
    );

    server.await.expect("server task");
}
