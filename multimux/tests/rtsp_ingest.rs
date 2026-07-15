//! Deterministic loopback e2e test for the RTSP client ingest path
//! (`multimux::source::rtsp::RtspSource`, issue #705 item 3): a real
//! `127.0.0.1:0` TCP socket, DESCRIBE -> SETUP -> PLAY, then a few interleaved
//! RTP packets — exercising the real socket -> SDP-parse ->
//! interleaved-depayload path with no live network and no flakiness (every
//! step is synchronized by the request/response handshake itself, and the
//! whole exchange is bounded by an outer timeout so a wiring bug fails fast
//! instead of hanging CI).
//!
//! # Why a hand-rolled server, not `rtsp_runtime::AsyncRtspServer`
//!
//! `ServerSession::handle_request` (rtsp-runtime/src/server.rs) always builds
//! a fixed, empty-body response — there is no way to hand it a custom DESCRIBE
//! body. `RtspSource::connect` needs a real, parseable SDP (carrying
//! `sprop-parameter-sets`) to build the AVC config, so a minimal raw-TCP
//! responder is used instead: it accepts the socket, reads each request,
//! and writes byte-correct `RTSP/1.0` responses (echoing `CSeq`) — DESCRIBE
//! gets a `Content-Type: application/sdp` body, SETUP negotiates interleaved
//! TCP on channel 0-1, PLAY is a bare 200. This still drives
//! `rtsp-runtime`'s real client parser (`AsyncRtspClient`) against real
//! response bytes, so the client-side wire path is exercised for real; only
//! the server side is hand-written (rtsp-runtime has no server-side
//! body-injection hook to reuse).
//!
//! No feature gate is needed: `multimux::source::rtsp::RtspSource` is not
//! behind any Cargo feature (only `rtsps://` TLS support is, and this test
//! uses plain `rtsp://`).

use std::time::Duration;

use multimux::source::rtsp::RtspSource;
use rtsp_runtime::{InterleavedFrame, TransportSpec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use transmux::pipeline::CodecConfig;

/// A known-good H.264 `sprop-parameter-sets` (SPS + PPS) — the same vector
/// `avc_config_from_sprop`/`avc_config_from_fmtp` are unit-tested against
/// elsewhere in this workspace (transmux/src/rtp_sdp.rs,
/// multimux/src/source/{rtsp,sdp}.rs, multimux/tests/origin_llhls.rs).
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

/// The interleaved RTP channel SETUP negotiates for the sole video media
/// (RTCP rides the paired odd channel, unused by this test).
const RTP_CHANNEL: u8 = 0;

/// Bound on the whole DESCRIBE->SETUP->PLAY->depayload exchange: a wiring bug
/// (e.g. a response the client can't parse) must fail fast, not hang CI.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

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
    // spacing at the SDP's 90 kHz clock rate. `RtpStreamDepacketizer` (see
    // transmux/src/rtp_stream.rs) only knows a sample's duration once the
    // *next* AU's timestamp has arrived, so 3 AUs yield exactly 2 completed
    // samples (AU2 stays pending until a `flush`, which this test never
    // calls).
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

    let source = RtspSource::new("cam", format!("rtsp://{addr}/test"));
    let mut session = source
        .connect()
        .await
        .expect("DESCRIBE->SETUP->PLAY connect");

    // The DESCRIBE SDP was really parsed and a real avcC built (not the empty
    // placeholder): one video track at the RTP clock rate, carrying the
    // sprop's SPS + PPS.
    let specs = session.track_specs();
    assert_eq!(specs.len(), 1, "one video track from the SDP");
    // NOTE: this does not by itself prove the `a=rtpmap` clock rate was parsed
    // (rather than defaulted): H.264's RTP clock is fixed at 90 kHz per RFC 6184
    // §8.2.1, which coincides with the crate's `DEFAULT_CLOCK_RATE_HZ`, so a
    // video-only conformant fixture can't distinguish the two. `rtpmap_clock_rate`
    // parsing is bitten separately in transmux's `rtp_sdp` unit tests.
    assert_eq!(
        specs[0].timescale, 90_000,
        "RTP clock rate becomes the IR timescale"
    );
    match &specs[0].config {
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

    // Drive the real interleaved-frame -> depayload path until at least 2
    // samples have come out (see the AU timing note in `serve_one_session`).
    let mut samples = Vec::new();
    while samples.len() < 2 {
        let batch = session
            .next_samples()
            .await
            .expect("next_samples")
            .expect("server closed before 2 samples were emitted");
        samples.extend(batch);
    }

    assert_eq!(
        samples.len(),
        2,
        "3 access units over interleaved RTP yield exactly 2 completed samples"
    );
    let (track_id0, sample0) = &samples[0];
    assert_eq!(*track_id0, 1, "the sole video track routes to track id 1");
    assert!(sample0.is_sync, "the first access unit was the IDR");
    let (track_id1, sample1) = &samples[1];
    assert_eq!(*track_id1, 1);
    assert!(!sample1.is_sync, "the second access unit was non-IDR");

    server.await.expect("server task");
}
