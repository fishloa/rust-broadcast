//! Issue #740 acceptance: a **real browser** publishing WHIP into multimux,
//! producing real segments on the LL-HLS endpoint -- the gate that decides
//! whether `crate::source::whip` actually interops with a genuine WebRTC
//! peer, not just that its own unit tests (SDP parsing, wire-byte
//! reconstruction, NAL scanning) pass in isolation.
//!
//! Feature-gated: this whole file compiles to nothing unless built with
//! `--features whip` (needs rustc >= 1.88 -- see `Cargo.toml`'s `whip`
//! feature doc), so it is inert for the crate's default, MSRV-1.86 build.
//!
//! # What this proves that the in-module unit tests can't
//!
//! `multimux/src/source/whip.rs`'s own tests exercise `parse_whip_offer`,
//! `rebuild_rtp_wire`, and the deferred-`avcC`-capture gate against
//! hand-built bytes. They do not prove a real browser's actual SDP offer
//! shape (candidate lines, `a=setup:actpass`, bundle attributes, real ICE
//! ufrag/pwd, a real self-signed DTLS handshake, real SRTP-encrypted RTP)
//! round-trips through this module end to end. Only a real
//! `RTCPeerConnection` can prove that.
//!
//! # Real browser, not a mock peer
//!
//! `tests/assets/whip_publish.mjs` drives headless Chromium via Playwright
//! (`fixtures`/vendored under `tests/assets/node_modules`, same harness
//! shape as `lldash_dashjs.rs`'s dash.js check): a real fake-video-device
//! capture, a real H.264 software encode, real ICE connectivity checks, a
//! real DTLS handshake against this crate's freshly-generated self-signed
//! certificate, and real SRTP encryption of the RTP this test then asserts
//! multimux turned into real LL-HLS segments. See `verification-before-
//! completion`: this is the honest alternative to asserting a hand-rolled
//! mock peer agrees with itself.
//!
//! # Scope: video only (see `crate::source::whip`'s own module doc)
//!
//! The harness requests video only and forces H.264 via
//! `RTCRtpSender.setCodecPreferences` -- this workspace has no RTP/Opus
//! depacketiser, so a real microphone track would be silently unroutable
//! (and `parse_whip_offer` rejects any offer with an audio section at all,
//! rather than admitting one it can't carry).
//!
//! # Skip-clean discipline
//!
//! Mirrors `lldash_dashjs.rs`: skips (printing why), rather than failing,
//! when `node` or the vendored Playwright/Chromium under `tests/assets/` are
//! missing -- green on a fresh clone before `bun install`
//! (`multimux/tests/assets/`) has been run, or in a CI image without Node.

#![cfg(feature = "whip")]

use std::net::SocketAddr;
use std::process::Command;
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry;

/// How long the harness waits for `RTCPeerConnection.connectionState` to
/// reach `"connected"` (ICE connectivity checks + the DTLS handshake against
/// this route's freshly-generated self-signed certificate) before giving up.
const CONNECT_TIMEOUT_MS: u64 = 15_000;

/// How long the harness keeps the connection open, actively sending real
/// encoded video, after connecting -- gives `multimux`'s segmenter (`0.5 s`
/// target duration below) real wall-clock time to close at least one real
/// segment before this test asks the LL-HLS endpoint for one.
const HOLD_MS: u64 = 6_000;

/// Generous hang guard on the LL-HLS playlist poll: connect, DTLS handshake,
/// and real-time H.264 encode/segment together are comfortably under this in
/// practice; the bound exists so a genuinely broken/dead WHIP path fails the
/// test instead of hanging the suite forever.
const PLAYLIST_HANG_GUARD: Duration = Duration::from_secs(30);

/// "Reserve then drop, hand the exact address to the thing that binds it" --
/// see `dispatch_ingest.rs`'s identical helper.
fn reserve_tcp_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
}

fn assets_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/assets"))
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn harness_ready() -> bool {
    assets_dir().join("node_modules/playwright").is_dir()
}

macro_rules! skip_unless {
    ($cond:expr, $why:expr) => {
        if !$cond {
            eprintln!("SKIP whip_ingest: {}", $why);
            return;
        }
    };
}

/// The measured result `tests/assets/whip_publish.mjs` prints as JSON.
#[derive(Debug, serde::Deserialize)]
struct WhipPublishResult {
    ok: bool,
    error: Option<String>,
    #[serde(rename = "connectionState")]
    connection_state: Option<String>,
    #[serde(rename = "bytesSent")]
    bytes_sent: Option<u64>,
}

/// Shell out to the node/Playwright harness, returning its parsed JSON
/// result. Panics (a genuine test failure, not a skip) if the child process
/// itself couldn't be run or didn't print valid JSON -- by the time this is
/// called, [`harness_ready`]/[`node_available`] have already confirmed the
/// prerequisites are in place.
fn run_whip_publish_check(whip_url: &str) -> WhipPublishResult {
    let output = Command::new("node")
        .arg(assets_dir().join("whip_publish.mjs"))
        .arg(whip_url)
        .arg(CONNECT_TIMEOUT_MS.to_string())
        .arg(HOLD_MS.to_string())
        .current_dir(assets_dir())
        .output()
        .expect("spawn node whip_publish.mjs");

    if !output.stderr.is_empty() {
        eprintln!(
            "[whip_publish.mjs stderr]\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        output.status.success(),
        "whip_publish.mjs must exit 0 (a measured pass/fail is still exit 0 -- only a \
         harness-level failure, e.g. browser launch failure, exits non-zero): {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("whip_publish.mjs must print one JSON object: {e}\nstdout: {stdout}")
    })
}

/// Polls `playlist_url` until its body carries a real closed-segment
/// `#EXTINF:` line -- see `dispatch_ingest.rs`'s identical helper for why
/// this (not just a `200 OK`) is the real bar.
async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + PLAYLIST_HANG_GUARD;
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "no #EXTINF: line appeared in {playlist_url} within the hang guard -- the \
                 WHIP route never produced a closed segment"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_browser_whip_publish_produces_llhls_segments() {
    skip_unless!(node_available(), "node not on PATH");
    skip_unless!(
        harness_ready(),
        "tests/assets/node_modules/playwright missing -- run `bun install` (or `npm install`) \
         in multimux/tests/assets/ first"
    );

    let bind_addr = reserve_tcp_addr();
    let whip_addr = reserve_tcp_addr();
    let config = Config {
        bind: bind_addr.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input: InputSpec::Whip {
                listen: whip_addr.to_string(),
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
        }],
        ..Config::default()
    };

    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let whip_url = format!("http://{whip_addr}/whip");
    let whip_url_for_check = whip_url.clone();
    let publish = tokio::task::spawn_blocking(move || run_whip_publish_check(&whip_url_for_check));

    // Await the browser's own result *before* polling the playlist: if the
    // browser side failed (e.g. never connected), this prints exactly why
    // instead of the playlist poll's generic hang-guard panic masking it.
    let result = publish.await.expect("whip_publish.mjs task must not panic");
    eprintln!(
        "whip_ingest result: ok={} connectionState={:?} bytesSent={:?} error={:?}",
        result.ok, result.connection_state, result.bytes_sent, result.error
    );

    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = poll_until_extinf(&client, &playlist_url).await;
    server.abort();

    assert!(
        result.ok,
        "the browser must connect and send real encoded video: {:?} (connectionState={:?})",
        result.error, result.connection_state
    );
    assert_eq!(
        result.connection_state.as_deref(),
        Some("connected"),
        "RTCPeerConnection must reach connectionState=connected"
    );
    assert!(
        result.bytes_sent.unwrap_or(0) > 0,
        "the browser's own outbound-rtp stats must show real bytes sent"
    );
    assert!(
        playlist.contains("#EXTINF:"),
        "media playlist must carry a real closed-segment #EXTINF line: {playlist}"
    );
}
