//! Issue #743 acceptance: a **real browser** publishing over WHIP and a
//! second real `RTCPeerConnection` **playing it back over WHEP**, in the
//! same running `multimux` origin -- the gate that decides whether
//! `crate::output::whep` actually interops with a genuine WebRTC peer, not
//! just that its own unit tests (SDP parsing, `patch_seq_and_timestamp`,
//! `choose_setup_role`) pass in isolation. Mirrors `whip_ingest.rs`'s own
//! "why a real browser, not a mock peer" reasoning exactly, on the egress
//! side this time.
//!
//! Feature-gated on **both** `whip` and `whep`: this file needs a WHIP
//! publisher to feed the route a real track before a WHEP viewer has
//! anything to negotiate against, so it compiles to nothing unless built
//! with `--features whip,whep` (both need rustc >= 1.88 -- see
//! `Cargo.toml`'s feature docs), inert for the crate's default, MSRV-1.86
//! build.
//!
//! # What this proves that the in-module unit tests can't
//!
//! `multimux/src/output/whep.rs`'s own tests exercise `parse_whep_offer`,
//! `choose_setup_role`, `rescale_to_90k`, and `patch_seq_and_timestamp`
//! against hand-built bytes. They do not prove that a real browser's WHEP
//! offer round-trips through this module end to end: a real self-signed
//! DTLS handshake (this side as DTLS server or client, depending on the
//! offer's `a=setup`), real SRTP encryption of RTP this module packetised
//! from real `Trunk` samples, and -- the actual bar -- a real
//! `RTCPeerConnection`'s own H.264 decoder producing real decoded frames
//! from it. Only a real browser can prove that.
//!
//! # Real browser, not a mock peer
//!
//! `tests/assets/whep_playback.mjs` drives headless Chromium via Playwright
//! (vendored under `tests/assets/node_modules`, same harness shape as
//! `whip_ingest.rs`'s own `whip_publish.mjs`): a real fake-video-device
//! capture publishes into this route over WHIP, then a second real
//! `RTCPeerConnection` in the same page negotiates WHEP playback, attaches
//! the received track to a real `<video>` element, and the harness asserts
//! `RTCRtpReceiver` stats (`framesDecoded > 0`) *and* the `<video>`
//! element's own `currentTime` advancing -- proof of actual decode, not
//! just packet receipt. See `verification-before-completion`: this is the
//! honest alternative to asserting a hand-rolled mock peer agrees with
//! itself.
//!
//! # Scope: video only (see `crate::output::whep`'s own module doc)
//!
//! Same constraint as WHIP ingest, in reverse: this workspace has no
//! RTP/Opus packetiser, so the WHEP viewer's offer requests video only.
//!
//! # Skip-clean discipline
//!
//! Mirrors `whip_ingest.rs`/`lldash_dashjs.rs`: skips (printing why),
//! rather than failing, when `node` or the vendored Playwright/Chromium
//! under `tests/assets/` are missing -- green on a fresh clone before `bun
//! install` (`multimux/tests/assets/`) has been run, or in a CI image
//! without Node.

#![cfg(all(feature = "whip", feature = "whep"))]

use std::net::SocketAddr;
use std::process::Command;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on;

#[path = "support/listener.rs"]
mod listener;
use listener::bind_tcp;

/// How long the harness waits for each `RTCPeerConnection.connectionState`
/// to reach `"connected"` -- see `whip_ingest.rs`'s identical constant.
const CONNECT_TIMEOUT_MS: u64 = 15_000;

/// How long the harness keeps both connections open, actively
/// sending/decoding real media, once both are connected.
const HOLD_MS: u64 = 6_000;

/// See `whip_ingest.rs`'s identical helper.
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
            eprintln!("SKIP whep_egress: {}", $why);
            return;
        }
    };
}

/// The measured result `tests/assets/whep_playback.mjs` prints as JSON.
#[derive(Debug, serde::Deserialize)]
struct WhepPlaybackResult {
    ok: bool,
    error: Option<String>,
    #[serde(rename = "publishConnectionState")]
    publish_connection_state: Option<String>,
    #[serde(rename = "viewerConnectionState")]
    viewer_connection_state: Option<String>,
    #[serde(rename = "bytesSent")]
    bytes_sent: Option<u64>,
    #[serde(rename = "bytesReceived")]
    bytes_received: Option<u64>,
    #[serde(rename = "framesDecoded")]
    frames_decoded: Option<u64>,
    #[serde(rename = "videoCurrentTime")]
    video_current_time: Option<f64>,
}

/// Shell out to the node/Playwright harness -- see `whip_ingest.rs`'s
/// identical `run_whip_publish_check` for why a non-zero exit is a genuine
/// test failure, not a measured `ok: false`.
fn run_whep_playback_check(whip_url: &str, whep_url: &str) -> WhepPlaybackResult {
    let output = Command::new("node")
        .arg(assets_dir().join("whep_playback.mjs"))
        .arg(whip_url)
        .arg(whep_url)
        .arg(CONNECT_TIMEOUT_MS.to_string())
        .arg(HOLD_MS.to_string())
        .current_dir(assets_dir())
        .output()
        .expect("spawn node whep_playback.mjs");

    if !output.stderr.is_empty() {
        eprintln!(
            "[whep_playback.mjs stderr]\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        output.status.success(),
        "whep_playback.mjs must exit 0 (a measured pass/fail is still exit 0 -- only a \
         harness-level failure, e.g. browser launch failure, exits non-zero): {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("whep_playback.mjs must print one JSON object: {e}\nstdout: {stdout}")
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_browser_whep_playback_decodes_real_video() {
    skip_unless!(node_available(), "node not on PATH");
    skip_unless!(
        harness_ready(),
        "tests/assets/node_modules/playwright missing -- run `bun install` (or `npm install`) \
         in multimux/tests/assets/ first"
    );

    let (bind_addr, bind_listener) = bind_tcp();
    let whip_addr = reserve_tcp_addr();
    let whep_addr = reserve_tcp_addr();
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
            outputs: vec![OutputKind::Whep {
                listen: whep_addr.to_string(),
            }],
            dvr: DvrConfig::default(),
        }],
        ..Config::default()
    };

    let server = tokio::spawn(serve_with_registry_on(bind_listener, config, SchemeRegistry::new()));

    let whip_url = format!("http://{whip_addr}/whip");
    let whep_url = format!("http://{whep_addr}/whep");
    let playback =
        tokio::task::spawn_blocking(move || run_whep_playback_check(&whip_url, &whep_url));

    let result = playback
        .await
        .expect("whep_playback.mjs task must not panic");
    server.abort();

    eprintln!(
        "whep_egress result: ok={} publish={:?} viewer={:?} bytesSent={:?} bytesReceived={:?} \
         framesDecoded={:?} videoCurrentTime={:?} error={:?}",
        result.ok,
        result.publish_connection_state,
        result.viewer_connection_state,
        result.bytes_sent,
        result.bytes_received,
        result.frames_decoded,
        result.video_current_time,
        result.error,
    );

    assert_eq!(
        result.publish_connection_state.as_deref(),
        Some("connected"),
        "the WHIP publisher must reach connectionState=connected: {:?}",
        result.error
    );
    assert_eq!(
        result.viewer_connection_state.as_deref(),
        Some("connected"),
        "the WHEP viewer must reach connectionState=connected: {:?}",
        result.error
    );
    assert!(
        result.bytes_sent.unwrap_or(0) > 0,
        "the publisher's own outbound-rtp stats must show real bytes sent"
    );
    assert!(
        result.bytes_received.unwrap_or(0) > 0,
        "the viewer's own inbound-rtp stats must show real bytes received over WHEP"
    );
    assert!(
        result.frames_decoded.unwrap_or(0) > 0,
        "the viewer's own browser decoder must have actually decoded a real frame: {:?}",
        result.error
    );
    assert!(
        result.video_current_time.unwrap_or(0.0) > 0.0,
        "the <video> element's currentTime must have advanced: real playback, not just receipt"
    );
    assert!(
        result.ok,
        "whep_playback.mjs's own aggregate check must pass: {:?}",
        result.error
    );
}

/// Audit run 7, W6: a route added through the runtime admin API
/// (`POST /admin/routes`, which is also how every startup route is added when
/// the admin API is enabled) must actually start its WHEP egress listener.
///
/// `spawn_route` previously never called `spawn_whep_outputs`, so a WHEP
/// output configured this way was silently never served — the listen port was
/// never bound. This asserts the port accepts a real TCP connection after the
/// admin add, and (as the counter-check) that it does NOT before.
#[tokio::test]
async fn admin_added_route_starts_its_whep_listener() {
    use multimux::config::{AdminSpec, OutputAuthSpec};
    use multimux::serve_config_file_with_registry_on_admin;

    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let whep_addr = reserve_tcp_addr();

    let config = Config {
        bind: media_addr.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        // `/admin/routes` requires at least one route in the config's own
        // validation to reload, but add_route itself is independent — start
        // empty and add the WHEP route entirely through the API.
        routes: vec![],
        admin: Some(AdminSpec {
            bind: admin_addr.to_string(),
            auth: OutputAuthSpec::Bearer {
                token: "admin-test-token".to_string(),
            },
        }),
        ..Config::default()
    };

    let dir = std::env::temp_dir().join(format!("multimux-admin-whep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let cfg_path = dir.join("config.json");
    std::fs::write(
        &cfg_path,
        serde_json::to_vec(&serde_json::json!({
            "bind": config.bind,
            "admin": { "bind": config.admin.as_ref().unwrap().bind,
                       "auth": { "scheme": "bearer", "token": "admin-test-token" } },
            "routes": [ { "name": "seed",
                          "input": { "type": "rtsp", "url": "rtsp://127.0.0.1:1/seed" },
                          "outputs": ["llhls"] } ],
        }))
        .unwrap(),
    )
    .expect("write config");

    let server = tokio::spawn(serve_config_file_with_registry_on_admin(
        media_listener,
        admin_listener,
        cfg_path,
        instant_registry(),
    ));

    // Wait for the admin listener.
    {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::net::TcpStream::connect(admin_addr).await.is_err() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "admin listener never bound (server task returned: {:?})",
                server.is_finished()
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    // Before the add, nothing is listening on the WHEP port.
    assert!(
        tokio::net::TcpStream::connect(whep_addr).await.is_err(),
        "nothing may be listening on the WHEP port before the route is added"
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{admin_addr}/admin/routes"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "name": "cam",
            "input": { "type": "custom", "type_tag": "instant" },
            "outputs": [ { "whep": { "listen": whep_addr.to_string() } } ],
        }))
        .send()
        .await
        .expect("admin POST");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::CREATED,
        "adding the WHEP route must succeed"
    );

    // After the add, the WHEP listener must actually be bound — the pre-fix
    // `spawn_route` never spawned it, so this connect would fail forever.
    {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if tokio::net::TcpStream::connect(whep_addr).await.is_ok() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the admin-added route's WHEP listener ({whep_addr}) was never bound"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

/// An `"instant"` `Custom` input scheme: announces one program and queues a
/// handful of synthetic samples in its first `feed`, so the route publishes a
/// `Trunk` (which every egress, WHEP included, awaits before binding) with no
/// real network I/O. Mirrors `tests/admin_api.rs`'s identically-named scheme.
mod instant {
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::sync::Arc;

    use broadcast_common::{Demand, Stage, Timestamp};
    use media_plane::ingress::{
        Dialer, HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent,
    };
    use media_plane::trunk::{RetentionClass, TrunkConfig};
    use multimux::route::RouteHandle;
    use multimux::source::{DriverProgress, advance_route};
    use multimux::{Backoff, InputCtx, InputFactory, SchemeRegistry};
    use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

    pub const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
    const FRAME_COUNT: u32 = 24;

    fn track_spec() -> TrackSpec {
        let config = transmux::avc_config_from_sprop(SPROP).expect("valid sprop");
        TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 64,
                height: 64,
            },
        )
    }

    struct Session {
        pending: VecDeque<SessionEvent>,
        sent: bool,
    }

    impl Stage for Session {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = Infallible;

        fn demand(&self) -> Demand {
            Demand::new(1)
        }

        fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
            if !self.sent {
                self.sent = true;
                self.pending.push_back(SessionEvent::NewProgram {
                    program: ProgramId(0),
                    tracks: vec![track_spec()],
                });
                for i in 0..FRAME_COUNT {
                    let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 32];
                    self.pending.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id: 1,
                        retention: RetentionClass::Timed,
                        sample: Sample::new(
                            data,
                            Some(i64::from(i) * i64::from(FRAME_DUR)),
                            Some(i64::from(i) * i64::from(FRAME_DUR)),
                            Some(FRAME_DUR),
                            i % 8 == 0,
                        ),
                    });
                }
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<SessionEvent> {
            self.pending.pop_front()
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn finish(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }

    impl IngestSession for Session {
        type Request = Infallible;
    }

    #[derive(Clone, Copy, Default)]
    struct Dial;

    impl Dialer for Dial {
        type Session = Session;
        type Error = Infallible;

        fn dial(&mut self) -> Result<Session, Infallible> {
            Ok(Session {
                pending: VecDeque::new(),
                sent: false,
            })
        }
    }

    async fn run(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
        let mut dialer = Dial;
        let session = dialer.dial().unwrap_or_else(|never| match never {});
        let trunk_config = TrunkConfig::new(
            std::num::NonZeroUsize::new(64).unwrap(),
            std::num::NonZeroUsize::new(16).unwrap(),
            std::num::NonZeroUsize::new(8).unwrap(),
            std::num::NonZeroUsize::new(64).unwrap(),
            std::num::NonZeroUsize::new(64).unwrap(),
        );
        let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
        let mut driver: IngestDriver<Session> = IngestDriver::new(
            session,
            trunk_config,
            handshake,
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut progress = DriverProgress::new();
        driver.feed(&[], Timestamp::from_nanos(0));
        advance_route(&driver, &route_handle, &mut progress).await;
        driver.finish();
        advance_route(&driver, &route_handle, &mut progress).await;
        Ok(())
    }

    pub fn registry() -> SchemeRegistry {
        let mut registry = SchemeRegistry::new();
        registry.register_input(
            "instant",
            Arc::new(|ctx: InputCtx| {
                Ok(tokio::spawn(multimux::supervise_driver(
                    run,
                    ctx.store,
                    Backoff::production_default(),
                    ctx.name,
                    ctx.cancel,
                )))
            }) as InputFactory,
        );
        registry
    }
}

use instant::registry as instant_registry;
