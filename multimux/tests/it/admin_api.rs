//! Integration tests for the runtime admin API (issue #749): add/remove/list
//! routes and reload the config file without restarting the origin.
//!
//! Every test drives the *real* HTTP surface (two real bound TCP listeners —
//! media and admin — real `reqwest` requests), never `RouteRegistry`
//! directly (that type is crate-private): this is the same "drive it through
//! the real dispatch path" discipline `multimux/tests/it/dispatch_ingest.rs`
//! established for `serve_with_registry` itself.
//!
//! An `InputSpec::Custom` "instant" scheme (mirroring
//! `examples/custom_scheme.rs`'s `DemoDialer`/`DemoSession`) stands in for a
//! real camera: it announces one program and queues every synthetic sample
//! in its very first `feed` call, so a route becomes servable (real fMP4
//! init bytes + at least one closed segment) in well under a second with no
//! real network I/O — see `poll_until_extinf` (copied from
//! `dispatch_ingest.rs`) for the hang-guarded wait.
//!
//! The two tests that matter most — `delete_drains_route_without_disturbing_others`
//! and `reload_leaves_unchanged_route_running_restarts_changed_route` — are
//! exactly the ones the issue calls out as "what distinguishes this from
//! restart with extra steps".

use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use transmux::TrackId;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use multimux::config::{AdminSpec, Config, InputSpec, OutputAuthSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::{InputCtx, InputFactory};
use multimux::route::RouteHandle;
use multimux::source::{DriverProgress, advance_route};
use multimux::{
    Backoff, SchemeRegistry, serve_config_file_with_registry_on_admin, serve_with_registry_on_admin,
};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

use crate::support::listener;
use crate::support::shared_it::{nz, poll_until_extinf};
use listener::bind_tcp;
use test_fixtures::process::wait_for_port;

const ADMIN_TOKEN: &str = "admin-test-token";

// --- The "instant" synthetic Custom input scheme ---

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
/// ~0.8 s @ 30 fps — comfortably enough for several `target_duration_secs =
/// 0.2` closed segments once fed through the real segmenter.
const FRAME_COUNT: u32 = 24;
const SYNC_INTERVAL_FRAMES: u32 = 8;

fn instant_track_spec() -> TrackSpec {
    let config = transmux::avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        TrackId::new(1),
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 64,
            height: 64,
        },
    )
}

/// Mirrors `examples/custom_scheme.rs`'s `DemoSession`: announces one
/// program and queues every synthetic sample in its first `feed` call.
struct InstantSession {
    pending: VecDeque<SessionEvent>,
    sent: bool,
}

impl InstantSession {
    fn new() -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(SessionEvent::Established);
        InstantSession {
            pending,
            sent: false,
        }
    }
}

impl Stage for InstantSession {
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
                tracks: vec![instant_track_spec()],
            });
            for i in 0..FRAME_COUNT {
                let is_sync = i % SYNC_INTERVAL_FRAMES == 0;
                let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 32];
                let sample = Sample::new(
                    data,
                    Some(i64::from(i) * i64::from(FRAME_DUR)),
                    Some(i64::from(i) * i64::from(FRAME_DUR)),
                    Some(FRAME_DUR),
                    is_sync,
                );
                self.pending.push_back(SessionEvent::Sample {
                    program: ProgramId(0),
                    track_id: TrackId::new(1),
                    retention: RetentionClass::Timed,
                    sample,
                });
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for InstantSession {
    type Request = Infallible;
}

#[derive(Clone, Copy, Default)]
struct InstantDialer;

impl Dialer for InstantDialer {
    type Session = InstantSession;
    type Error = Infallible;

    fn dial(&mut self) -> Result<InstantSession, Infallible> {
        Ok(InstantSession::new())
    }
}

async fn run_instant(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
    let mut dialer = InstantDialer;
    let session = dialer.dial().unwrap_or_else(|never| match never {});
    let trunk_config = TrunkConfig::default()
        .with_timed_capacity(nz(64))
        .with_sparse_capacity(nz(16))
        .with_segment_capacity(nz(8))
        .with_event_capacity(nz(64))
        .with_part_capacity(nz(64));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver: IngestDriver<InstantSession> = IngestDriver::new(
        session,
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut progress = DriverProgress::new();
    driver.feed(&[], Timestamp::from_nanos(0));
    advance_route(&driver, &route_handle, &mut progress).await;
    driver.finish();
    advance_route(&driver, &route_handle, &mut progress).await;
    Ok(())
}

/// Like [`run_instant`] but the session never ends: the route reaches `Live`
/// and stays there until the supervisor is told to stop.
async fn run_hold(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
    let mut dialer = InstantDialer;
    let session = dialer.dial().unwrap_or_else(|never| match never {});
    let trunk_config = TrunkConfig::default()
        .with_timed_capacity(nz(64))
        .with_sparse_capacity(nz(16))
        .with_segment_capacity(nz(8))
        .with_event_capacity(nz(64))
        .with_part_capacity(nz(64));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver: IngestDriver<InstantSession> = IngestDriver::new(
        session,
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut progress = DriverProgress::new();
    driver.feed(&[], Timestamp::from_nanos(0));
    advance_route(&driver, &route_handle, &mut progress).await;
    std::future::pending::<()>().await;
    Ok(())
}

/// A steady live source for the production segmenting path: announces one program and then
/// publishes one 30 fps AVC sample per `feed`, a sync sample every `TICKER_GOP` frames.
struct TickerSession {
    pending: VecDeque<SessionEvent>,
    frame: u32,
}

const TICKER_GOP: u32 = 6;

impl Stage for TickerSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn demand(&self) -> Demand {
        Demand::new(1)
    }

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        if self.frame == 0 {
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks: vec![instant_track_spec()],
            });
        }
        let pts = i64::from(self.frame) * i64::from(FRAME_DUR);
        self.pending.push_back(SessionEvent::Sample {
            program: ProgramId(0),
            track_id: TrackId::new(1),
            retention: RetentionClass::Timed,
            sample: Sample::new(
                vec![0xAA; 32],
                Some(pts),
                Some(pts),
                Some(FRAME_DUR),
                self.frame.is_multiple_of(TICKER_GOP),
            ),
        });
        self.frame += 1;
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for TickerSession {
    type Request = Infallible;
}

/// Feeds a [`TickerSession`] through the real driver, segmenter and publisher
/// (`advance_route`) at 30 frames per second until the supervisor stops it.
async fn run_ticker(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
    let session = TickerSession {
        pending: VecDeque::from([SessionEvent::Established]),
        frame: 0,
    };
    let trunk_config = TrunkConfig::default()
        .with_timed_capacity(nz(256))
        .with_sparse_capacity(nz(16))
        .with_segment_capacity(nz(16))
        .with_event_capacity(nz(64))
        .with_part_capacity(nz(256));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver: IngestDriver<TickerSession> = IngestDriver::new(
        session,
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut progress = DriverProgress::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(1000 / 30));
    loop {
        ticker.tick().await;
        driver.feed(&[], Timestamp::from_nanos(0));
        advance_route(&driver, &route_handle, &mut progress).await;
    }
}

/// A [`SchemeRegistry`] with the `"instant"` tag registered — every test
/// below shares this factory (the tag is arbitrary and route-count-agnostic:
/// several routes can each independently name `"instant"`, exactly like
/// several real cameras could each name `"rtsp"`).
fn instant_registry() -> SchemeRegistry {
    let mut registry = SchemeRegistry::new();
    registry.register_input(
        "instant",
        Arc::new(|ctx: InputCtx| {
            Ok(tokio::spawn(multimux::supervise_driver(
                run_instant,
                ctx.store,
                Backoff::production_default(),
                ctx.name,
                ctx.cancel,
            )))
        }) as InputFactory,
    );
    registry.register_input(
        "ticker",
        Arc::new(|ctx: InputCtx| {
            Ok(tokio::spawn(multimux::supervise_driver(
                run_ticker,
                ctx.store,
                Backoff::production_default(),
                ctx.name,
                ctx.cancel,
            )))
        }) as InputFactory,
    );
    registry.register_input(
        "hold",
        Arc::new(|ctx: InputCtx| {
            Ok(tokio::spawn(multimux::supervise_driver(
                run_hold,
                ctx.store,
                Backoff::production_default(),
                ctx.name,
                ctx.cancel,
            )))
        }) as InputFactory,
    );
    registry
}

/// A route that reaches `Live` and stays there (see [`run_hold`]).
fn hold_route(name: &str) -> Route {
    Route {
        input: InputSpec::Custom {
            type_tag: "hold".to_string(),
            params: serde_json::Value::Null,
        },
        ..instant_route(name)
    }
}

fn instant_route(name: &str) -> Route {
    Route {
        name: name.to_string(),
        input: InputSpec::Custom {
            type_tag: "instant".to_string(),
            params: serde_json::Value::Null,
        },
        outputs: vec![OutputKind::LlHls],
        dvr: DvrConfig::default(),
        ts_hls_cut: Default::default(),
        ts_hls_idr_fallback: Default::default(),
        declared_bandwidth_bps: None,
        audio_renditions: Vec::new(),
        audio_rendition_sources: Vec::new(),
        closed_caption_overrides: Vec::new(),
        audio_profiles: Vec::new(),
        subtitles: true,
    }
}

/// An `InputSpec::Rtsp` route that never connects (loopback port `1`, which
/// nothing listens on) — used for routes this file only needs to *exist*
/// (for `GET`/`DELETE`/reload-diff bookkeeping), never to actually serve
/// media. `unique` keeps two such routes from comparing `PartialEq`-equal.
fn unreachable_rtsp_route(name: &str, unique: &str) -> Route {
    Route {
        name: name.to_string(),
        input: InputSpec::Rtsp {
            url: format!("rtsp://127.0.0.1:1/{unique}"),
            auth: None,
        },
        outputs: vec![OutputKind::LlHls],
        dvr: DvrConfig::default(),
        ts_hls_cut: Default::default(),
        ts_hls_idr_fallback: Default::default(),
        declared_bandwidth_bps: None,
        audio_renditions: Vec::new(),
        audio_rendition_sources: Vec::new(),
        closed_caption_overrides: Vec::new(),
        audio_profiles: Vec::new(),
        subtitles: true,
    }
}

/// `Route`/`InputSpec`/`AuthSpec` are `Deserialize`-only in production (they
/// may carry credentials that must never round-trip back out as JSON), so
/// this test-only helper hand-builds the equivalent request body/config-file
/// JSON for exactly the two `InputSpec` shapes this file uses.
/// `OutputKind` (config-safe, no credentials) does derive `Serialize` and is
/// reused directly.
fn route_json(route: &Route) -> serde_json::Value {
    let input = match &route.input {
        InputSpec::Rtsp { url, .. } => serde_json::json!({ "type": "rtsp", "url": url }),
        InputSpec::Custom { type_tag, params } => {
            serde_json::json!({ "type": "custom", "type_tag": type_tag, "params": params })
        }
        other => panic!("route_json: unsupported InputSpec variant in this test helper: {other:?}"),
    };
    let outputs: Vec<serde_json::Value> = route
        .outputs
        .iter()
        .map(|k| serde_json::to_value(k).expect("OutputKind serializes"))
        .collect();
    serde_json::json!({ "name": route.name, "input": input, "outputs": outputs })
}

fn admin_config(media_bind: SocketAddr, admin_bind: SocketAddr, routes: Vec<Route>) -> Config {
    Config {
        bind: media_bind.to_string(),
        target_duration_secs: 0.2,
        part_target_ms: 50,
        window_segments: 8,
        routes,
        admin: Some(AdminSpec {
            bind: admin_bind.to_string(),
            auth: OutputAuthSpec::Bearer {
                token: ADMIN_TOKEN.to_string(),
            },
        }),
        ..Config::default()
    }
}

/// Polls `playlist_url` until its body carries a real closed-segment
/// `#EXTINF:` line — a generous hang guard (issue #807 style), not a
/// latency assertion: the synthetic "instant" scheme produces samples with
/// no real I/O wait, so this normally lands in well under a second.
fn media_playlist_url(media_addr: SocketAddr, name: &str) -> String {
    format!("http://{media_addr}/{name}/media.m3u8")
}

async fn wait_until_live(client: &reqwest::Client, media_addr: SocketAddr, name: &str) -> String {
    poll_until_extinf(client, &media_playlist_url(media_addr, name)).await
}

async fn admin_json(
    client: &reqwest::Client,
    admin_addr: SocketAddr,
    method: reqwest::Method,
    path: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> reqwest::Response {
    let mut req = client.request(method, format!("http://{admin_addr}{path}"));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    req.send().await.expect("admin request must complete")
}

async fn admin_get(
    client: &reqwest::Client,
    admin_addr: SocketAddr,
    path: &str,
) -> reqwest::Response {
    admin_json(
        client,
        admin_addr,
        reqwest::Method::GET,
        path,
        Some(ADMIN_TOKEN),
        None,
    )
    .await
}

fn created_at_nanos(route_json: &serde_json::Value) -> u128 {
    route_json["created_at_unix_nanos"]
        .as_u64()
        .map(u128::from)
        .unwrap_or_else(|| panic!("no created_at_unix_nanos in {route_json}"))
}

/// Test 1: adding a route at runtime serves media, without restarting.
#[tokio::test]
async fn add_route_at_runtime_serves_media_without_restart() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    // `Config::validate` rejects an empty `routes` list, so start with one
    // real (but otherwise irrelevant) seed route already configured, then
    // add a brand-new second one at runtime -- exactly the real "add a
    // 41st camera without disturbing the other 40" scenario.
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![unreachable_rtsp_route("seed", "seed")],
    );
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    let resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        Some(ADMIN_TOKEN),
        Some(route_json(&instant_route("newcam"))),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    // Never in `config.routes` at startup -- if this serves real media, the
    // admin API genuinely added it at runtime, no restart involved.
    let playlist = wait_until_live(&client, media_addr, "newcam").await;
    assert!(playlist.contains("#EXTINF:"));

    server.abort();
}

/// Test 2 (the one that matters most): deleting a route stops it, and every
/// OTHER route keeps serving completely uninterrupted.
#[tokio::test]
async fn delete_drains_route_without_disturbing_others() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![instant_route("cam1"), instant_route("cam2")],
    );
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "cam1").await;
    wait_until_live(&client, media_addr, "cam2").await;

    let cam2_before = admin_get(&client, admin_addr, "/admin/routes/cam2")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("cam2 status");

    let del = admin_json(
        &client,
        admin_addr,
        reqwest::Method::DELETE,
        "/admin/routes/cam1",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(del.status(), reqwest::StatusCode::NO_CONTENT);

    // cam1 stops serving immediately -- a new request 404s.
    let cam1_resp = client
        .get(media_playlist_url(media_addr, "cam1"))
        .send()
        .await
        .expect("GET cam1 after delete");
    assert_eq!(cam1_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // cam2 is COMPLETELY unaffected: still serving, and its RouteHandle was
    // never touched (identical created_at -- proof it's the same instance,
    // not a coincidentally-successful restart).
    let cam2_playlist = client
        .get(media_playlist_url(media_addr, "cam2"))
        .send()
        .await
        .expect("GET cam2 after cam1 delete");
    assert_eq!(cam2_playlist.status(), reqwest::StatusCode::OK);
    assert!(cam2_playlist.text().await.unwrap().contains("#EXTINF:"));

    let cam2_after = admin_get(&client, admin_addr, "/admin/routes/cam2")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("cam2 status after");
    assert_eq!(
        created_at_nanos(&cam2_before),
        created_at_nanos(&cam2_after),
        "cam2's RouteHandle must be the exact same instance -- deleting cam1 must not touch it"
    );

    server.abort();
}

/// The `multimux_route_up` value for `route` from `/metrics`.
async fn route_up(client: &reqwest::Client, media_addr: SocketAddr, route: &str) -> Option<f64> {
    let body = client
        .get(format!("http://{media_addr}/metrics"))
        .send()
        .await
        .expect("GET /metrics")
        .text()
        .await
        .expect("metrics body");
    body.lines()
        .filter(|l| l.starts_with("multimux_route_up{"))
        .find(|l| l.contains(&format!("route=\"{route}\"")))
        .and_then(|l| l.rsplit(' ').next()?.parse().ok())
}

/// Audit r07-O5 (#1083): a deleted route must not stay `multimux_route_up 1`
/// (its supervisor was stopped without ever reporting a final state, so the
/// gauge kept its last value forever). Unique route names: the gauge is
/// process-wide and other tests in this binary use `cam1`/`cam2`.
#[tokio::test]
async fn deleted_route_is_no_longer_reported_up() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![hold_route("o5-gone"), hold_route("o5-stays")],
    );
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "o5-gone").await;
    wait_until_live(&client, media_addr, "o5-stays").await;
    assert_eq!(route_up(&client, media_addr, "o5-gone").await, Some(1.0));

    let del = admin_json(
        &client,
        admin_addr,
        reqwest::Method::DELETE,
        "/admin/routes/o5-gone",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(del.status(), reqwest::StatusCode::NO_CONTENT);

    assert_eq!(
        route_up(&client, media_addr, "o5-gone").await,
        Some(0.0),
        "a deleted route must not be reported up"
    );
    assert_eq!(
        route_up(&client, media_addr, "o5-stays").await,
        Some(1.0),
        "other routes are untouched"
    );

    server.abort();
}

/// Test 3: `POST` a duplicate name -> `409`, original route untouched and
/// still live.
#[tokio::test]
async fn post_duplicate_name_is_conflict_and_original_stays_live() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "cam1").await;
    let before = admin_get(&client, admin_addr, "/admin/routes/cam1")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("cam1 status");

    let dup = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        Some(ADMIN_TOKEN),
        Some(route_json(&unreachable_rtsp_route("cam1", "dup"))),
    )
    .await;
    assert_eq!(dup.status(), reqwest::StatusCode::CONFLICT);

    let after = admin_get(&client, admin_addr, "/admin/routes/cam1")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("cam1 status after conflict");
    assert_eq!(
        created_at_nanos(&before),
        created_at_nanos(&after),
        "the original cam1 route must be completely untouched by the rejected duplicate POST"
    );

    let still_live = client
        .get(media_playlist_url(media_addr, "cam1"))
        .send()
        .await
        .expect("GET cam1 after conflict");
    assert_eq!(still_live.status(), reqwest::StatusCode::OK);

    server.abort();
}

/// Test 4: `DELETE` an unknown route -> `404`.
#[tokio::test]
async fn delete_unknown_route_is_not_found() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "cam1").await;

    let resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::DELETE,
        "/admin/routes/nope",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

    server.abort();
}

/// Test 5: malformed route JSON -> `400`, origin state (the route list)
/// unchanged.
#[tokio::test]
async fn malformed_route_body_is_bad_request_and_state_unchanged() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "cam1").await;

    let before = admin_get(&client, admin_addr, "/admin/routes")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("route list before");

    // Structurally invalid against `Route`'s schema (missing `input`
    // entirely, and `name` is a number, not a string) -- axum's `Json`
    // extractor rejects this before the handler ever runs.
    let malformed = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        Some(ADMIN_TOKEN),
        Some(serde_json::json!({ "name": 12345 })),
    )
    .await;
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);

    // Also exercise the semantic-validation 400 (structurally valid JSON,
    // but an empty `outputs` list): `validate_standalone` rejects it too.
    let semantically_invalid = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        Some(ADMIN_TOKEN),
        Some(serde_json::json!({
            "name": "bad",
            "input": { "type": "rtsp", "url": "rtsp://127.0.0.1:1/x" },
            "outputs": []
        })),
    )
    .await;
    assert_eq!(
        semantically_invalid.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    let after = admin_get(&client, admin_addr, "/admin/routes")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("route list after");
    assert_eq!(
        before, after,
        "route list must be byte-for-byte identical after two rejected POSTs"
    );

    server.abort();
}

/// Test 7: the admin API is unreachable on the media listener port.
#[tokio::test]
async fn admin_api_unreachable_on_media_port() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "cam1").await;

    // Same path, same (correct) bearer token, but sent to the MEDIA port
    // instead of the admin port.
    let resp = client
        .get(format!("http://{media_addr}/admin/routes"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("GET /admin/routes on the media port");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "the media listener must have no notion of /admin/* routes at all"
    );

    // Confirm the *real* admin API, on its own port, does resolve the same
    // path -- proving the 404 above is "wrong port", not "broken route".
    let real = admin_get(&client, admin_addr, "/admin/routes").await;
    assert_eq!(real.status(), reqwest::StatusCode::OK);

    server.abort();
}

/// Test 8: an unauthenticated admin request -> `401`, and the mutation did
/// not happen.
#[tokio::test]
async fn unauthenticated_admin_request_is_unauthorized_and_no_mutation() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "cam1").await;

    // No `Authorization` header at all.
    let unauth = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        None,
        Some(route_json(&instant_route("sneaky"))),
    )
    .await;
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // A GET (with valid auth) proves the sneaky route was never added.
    let list = admin_get(&client, admin_addr, "/admin/routes")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("route list");
    let names: Vec<&str> = list
        .as_array()
        .expect("route list is an array")
        .iter()
        .map(|r| r["name"].as_str().expect("route name"))
        .collect();
    assert_eq!(
        names,
        vec!["cam1"],
        "the unauthenticated POST must not have mutated the route set"
    );

    // Wrong (but present) token also 401s.
    let wrong_token = admin_json(
        &client,
        admin_addr,
        reqwest::Method::GET,
        "/admin/routes",
        Some("wrong-token"),
        None,
    )
    .await;
    assert_eq!(wrong_token.status(), reqwest::StatusCode::UNAUTHORIZED);

    server.abort();
}

/// Test 6 (the other one that matters most): reload converges added/
/// removed/changed routes, and a THIRD, unchanged route is never restarted.
#[tokio::test]
async fn reload_leaves_unchanged_route_running_restarts_changed_route() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();

    let config_path = std::env::temp_dir().join(format!(
        "multimux-admin-api-test-reload-{}-{}.json",
        std::process::id(),
        admin_addr.port()
    ));
    let initial = admin_config(
        media_addr,
        admin_addr,
        vec![
            instant_route("keep"),
            unreachable_rtsp_route("change-me", "before"),
            unreachable_rtsp_route("remove-me", "gone"),
        ],
    );
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&initial)).expect("serialize config"),
    )
    .expect("write initial config");

    let server = tokio::spawn(serve_config_file_with_registry_on_admin(
        media_listener,
        admin_listener,
        config_path.clone(),
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "keep").await;

    let keep_before = admin_get(&client, admin_addr, "/admin/routes/keep")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("keep status before reload");
    let change_before = admin_get(&client, admin_addr, "/admin/routes/change-me")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("change-me status before reload");

    // Rewrite the file: "keep" byte-for-byte identical, "change-me" gets a
    // different URL (same name), "remove-me" is dropped, "added-route" is
    // new.
    let updated = admin_config(
        media_addr,
        admin_addr,
        vec![
            instant_route("keep"),
            unreachable_rtsp_route("change-me", "after"),
            unreachable_rtsp_route("added-route", "new"),
        ],
    );
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&updated)).expect("serialize config"),
    )
    .expect("rewrite config");

    let reload_resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/reload",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(reload_resp.status(), reqwest::StatusCode::OK);
    let summary = reload_resp
        .json::<serde_json::Value>()
        .await
        .expect("reload summary");

    let as_name_set = |field: &str| -> std::collections::HashSet<String> {
        summary[field]
            .as_array()
            .unwrap_or_else(|| panic!("summary.{field} must be an array: {summary}"))
            .iter()
            .map(|v| v.as_str().expect("name string").to_string())
            .collect()
    };
    assert_eq!(
        as_name_set("added"),
        ["added-route".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<String>>()
    );
    assert_eq!(
        as_name_set("removed"),
        ["remove-me".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<String>>()
    );
    assert_eq!(
        as_name_set("changed"),
        ["change-me".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<String>>()
    );
    assert_eq!(
        as_name_set("unchanged"),
        ["keep".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<String>>()
    );

    // The headline assertion: "keep" was NEVER restarted.
    let keep_after = admin_get(&client, admin_addr, "/admin/routes/keep")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("keep status after reload");
    assert_eq!(
        created_at_nanos(&keep_before),
        created_at_nanos(&keep_after),
        "an unchanged route must not have been restarted by reload"
    );

    // "change-me" WAS restarted (different created_at).
    let change_after = admin_get(&client, admin_addr, "/admin/routes/change-me")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("change-me status after reload");
    assert_ne!(
        created_at_nanos(&change_before),
        created_at_nanos(&change_after),
        "a route whose config changed must have been restarted (new RouteHandle)"
    );

    // "remove-me" is gone; "added-route" exists.
    let removed = admin_get(&client, admin_addr, "/admin/routes/remove-me").await;
    assert_eq!(removed.status(), reqwest::StatusCode::NOT_FOUND);
    let added = admin_get(&client, admin_addr, "/admin/routes/added-route").await;
    assert_eq!(added.status(), reqwest::StatusCode::OK);

    // "keep" is still genuinely live and serving throughout.
    let keep_playlist = client
        .get(media_playlist_url(media_addr, "keep"))
        .send()
        .await
        .expect("GET keep after reload");
    assert_eq!(keep_playlist.status(), reqwest::StatusCode::OK);

    server.abort();
    let _ = std::fs::remove_file(&config_path);
}

/// A reload that restarts a changed route never leaves its viewers without an answer: while it
/// runs, a loop requesting the route's media playlist and a viewer following it with LL-HLS
/// blocking reloads (`_HLS_msn`/`_HLS_part`, parked on the old route when it is replaced) see
/// only `200`s. The parked request is woken and served again against the replacement, which
/// answers it as soon as it has that segment.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reload_of_a_changed_route_never_answers_a_viewer_with_an_error() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config_path = std::env::temp_dir().join(format!(
        "multimux-admin-api-test-gapless-{}-{}.json",
        std::process::id(),
        admin_addr.port()
    ));
    let with_version = |version: u32| {
        let mut route = instant_route("live");
        route.input = InputSpec::Custom {
            type_tag: "ticker".to_string(),
            params: serde_json::json!({ "version": version }),
        };
        admin_config(media_addr, admin_addr, vec![route])
    };
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&with_version(1))).expect("serialize"),
    )
    .expect("write config");
    let server = tokio::spawn(serve_config_file_with_registry_on_admin(
        media_listener,
        admin_listener,
        config_path.clone(),
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "live").await;
    let before = admin_get(&client, admin_addr, "/admin/routes/live")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("status before");

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let viewer = tokio::spawn({
        let (client, stop, answered) = (client.clone(), Arc::clone(&stop), Arc::clone(&answered));
        let url = media_playlist_url(media_addr, "live");
        async move {
            let mut statuses = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let status = client.get(&url).send().await.map(|r| r.status());
                statuses.push(status.map_err(|e| e.to_string()));
                answered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
            statuses
        }
    });

    let blocking_answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let blocking_viewer = tokio::spawn({
        let (client, stop, answered) = (
            client.clone(),
            Arc::clone(&stop),
            Arc::clone(&blocking_answered),
        );
        let url = media_playlist_url(media_addr, "live");
        async move {
            let mut statuses = Vec::new();
            let mut next = url.clone();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(resp) = client.get(&next).send().await else {
                    statuses.push(Err("transport error".to_string()));
                    break;
                };
                let status = resp.status();
                statuses.push(Ok(status));
                if status != reqwest::StatusCode::OK {
                    break;
                }
                answered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let body = resp.text().await.unwrap_or_default();
                let first: u64 = body
                    .lines()
                    .find_map(|l| l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let closed = body.lines().filter(|l| l.starts_with("#EXTINF")).count() as u64;
                next = format!("{url}?_HLS_msn={}&_HLS_part=0", first + closed);
            }
            statuses
        }
    });

    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&with_version(2))).expect("serialize"),
    )
    .expect("rewrite config");
    let reload = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/reload",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(reload.status(), reqwest::StatusCode::OK);
    let summary = reload.json::<serde_json::Value>().await.expect("summary");
    assert_eq!(summary["changed"], serde_json::json!(["live"]), "{summary}");
    wait_until_live(&client, media_addr, "live").await;
    let seen = answered.load(std::sync::atomic::Ordering::Relaxed);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while answered.load(std::sync::atomic::Ordering::Relaxed) < seen + 20 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the viewer stopped being answered"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let blocking_seen = blocking_answered.load(std::sync::atomic::Ordering::Relaxed);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while blocking_answered.load(std::sync::atomic::Ordering::Relaxed) < blocking_seen + 5 {
        if tokio::time::Instant::now() >= deadline {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let seen = blocking_viewer.await.expect("blocking viewer task");
            panic!("the blocking-reload viewer stopped being answered: {seen:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut statuses = viewer.await.expect("viewer task");
    statuses.extend(blocking_viewer.await.expect("blocking viewer task"));

    let after = admin_get(&client, admin_addr, "/admin/routes/live")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("status after");
    assert_ne!(
        created_at_nanos(&before),
        created_at_nanos(&after),
        "the route was restarted"
    );
    let bad: Vec<_> = statuses
        .iter()
        .filter(|s| !matches!(s, Ok(status) if *status == reqwest::StatusCode::OK))
        .collect();
    assert!(
        statuses.len() > 20 && bad.is_empty(),
        "{} requests, non-200 answers during the reload: {bad:?}",
        statuses.len()
    );

    server.abort();
    let _ = std::fs::remove_file(&config_path);
}

/// A blocking-reload viewer parked on a route that is then removed is served again against the
/// router that is live then, which has no route of that name: it ends with `404`, not with a `503`
/// after the blocking-reload timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_a_route_answers_a_parked_blocking_reload_with_404() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config_path = std::env::temp_dir().join(format!(
        "multimux-admin-api-test-remove-parked-{}-{}.json",
        std::process::id(),
        admin_addr.port()
    ));
    let mut route = instant_route("live");
    route.input = InputSpec::Custom {
        type_tag: "ticker".to_string(),
        params: serde_json::Value::Null,
    };
    let config = admin_config(media_addr, admin_addr, vec![route]);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&config)).expect("serialize"),
    )
    .expect("write config");
    let server = tokio::spawn(serve_config_file_with_registry_on_admin(
        media_listener,
        admin_listener,
        config_path.clone(),
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "live").await;

    let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let viewer = tokio::spawn({
        let (client, answered) = (client.clone(), Arc::clone(&answered));
        let url = media_playlist_url(media_addr, "live");
        async move {
            let mut next = url.clone();
            loop {
                let resp = client.get(&next).send().await.expect("answer");
                let status = resp.status();
                if status != reqwest::StatusCode::OK {
                    return status;
                }
                answered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let body = resp.text().await.unwrap_or_default();
                let first: u64 = body
                    .lines()
                    .find_map(|l| l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let closed = body.lines().filter(|l| l.starts_with("#EXTINF")).count() as u64;
                next = format!("{url}?_HLS_msn={}&_HLS_part=0", first + closed);
            }
        }
    });
    while answered.load(std::sync::atomic::Ordering::Relaxed) < 3 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let removed = admin_json(
        &client,
        admin_addr,
        reqwest::Method::DELETE,
        "/admin/routes/live",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert!(removed.status().is_success(), "{}", removed.status());
    let ended = tokio::time::timeout(std::time::Duration::from_secs(20), viewer)
        .await
        .expect("the viewer is answered")
        .expect("viewer task");
    assert_eq!(ended, reqwest::StatusCode::NOT_FOUND);

    server.abort();
    let _ = std::fs::remove_file(&config_path);
}

/// A changed route's replacement continues the playlist the viewers follow: its media sequence
/// numbers climb on from the replaced route's, the first segment it publishes carries
/// `EXT-X-DISCONTINUITY` (and no other segment does), and `EXT-X-DISCONTINUITY-SEQUENCE` counts the
/// discontinuities the replaced route's playlist already showed (RFC 8216bis section 6.2.2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_changed_routes_replacement_continues_numbering_with_a_discontinuity() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config_path = std::env::temp_dir().join(format!(
        "multimux-admin-api-test-continue-{}-{}.json",
        std::process::id(),
        admin_addr.port()
    ));
    let with_version = |version: u32| {
        let mut route = instant_route("live");
        route.outputs = vec![OutputKind::LlHls, OutputKind::Dash];
        route.input = InputSpec::Custom {
            type_tag: "ticker".to_string(),
            params: serde_json::json!({ "version": version }),
        };
        admin_config(media_addr, admin_addr, vec![route])
    };
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&with_version(1))).expect("serialize"),
    )
    .expect("write config");
    let server = tokio::spawn(serve_config_file_with_registry_on_admin(
        media_listener,
        admin_listener,
        config_path.clone(),
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();
    let url = media_playlist_url(media_addr, "live");

    // Let the first route publish a few segments, then read its playlist as the viewers see it.
    let old = loop {
        let body = wait_until_live(&client, media_addr, "live").await;
        let playlist = broadcast_hls::MediaPlaylist::parse(&body).expect("old playlist parses");
        if playlist.segments.len() >= 3 {
            break playlist;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let old_last = old.media_sequence + old.segments.len() as u64 - 1;
    let mpd_url = format!("http://{media_addr}/live/manifest.mpd");
    let old_mpd = client
        .get(&mpd_url)
        .send()
        .await
        .expect("old mpd")
        .text()
        .await
        .expect("old mpd body");
    let before = admin_get(&client, admin_addr, "/admin/routes/live")
        .await
        .json::<serde_json::Value>()
        .await
        .expect("status before");

    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&with_version(2))).expect("serialize"),
    )
    .expect("rewrite config");
    let reload = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/reload",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(reload.status(), reqwest::StatusCode::OK);

    // The first segment the replacement closes.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let new = loop {
        let after = admin_get(&client, admin_addr, "/admin/routes/live")
            .await
            .json::<serde_json::Value>()
            .await
            .expect("status after");
        if created_at_nanos(&before) != created_at_nanos(&after)
            && let Ok(resp) = client.get(&url).send().await
            && resp.status() == reqwest::StatusCode::OK
            && let Ok(body) = resp.text().await
            && let Ok(playlist) = broadcast_hls::MediaPlaylist::parse(&body)
            && !playlist.segments.is_empty()
        {
            break (playlist, body);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the replacement never published a segment"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let (new, new_text) = new;

    assert!(
        new.media_sequence > old_last,
        "media sequence {} does not continue past the replaced route's {old_last}",
        new.media_sequence
    );
    assert!(new.segments[0].discontinuous, "{new_text}");
    assert!(
        new.segments[1..]
            .iter()
            .all(|segment| !segment.discontinuous),
        "only the replacement's first segment is a discontinuity: {new_text}"
    );
    let old_tagged = old.segments.iter().filter(|s| s.discontinuous).count() as u64;
    assert_eq!(
        new.discontinuity_sequence,
        old.discontinuity_sequence + old_tagged,
        "{new_text}"
    );
    let lines: Vec<&str> = new_text.lines().collect();
    let tag_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| **l == "#EXT-X-DISCONTINUITY")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        tag_lines.len(),
        1,
        "exactly one discontinuity tag: {new_text}"
    );
    let first_segment_line = lines
        .iter()
        .position(|l| *l == new.segments[0].uri)
        .expect("first segment line");
    let segment_lines_between = lines[tag_lines[0]..first_segment_line]
        .iter()
        .filter(|l| !l.starts_with('#'))
        .count();
    assert_eq!(
        segment_lines_between, 0,
        "the tag belongs to the first segment, with no other segment between: {new_text}"
    );
    // DASH: the replacement keeps the presentation origin and the start number, so the
    // timeline and the segment numbers carry on in the same single Period.
    let attribute = |text: &str, name: &str| -> String {
        let start = text.find(&format!("{name}=\"")).expect(name) + name.len() + 2;
        text[start..].split('"').next().expect("value").to_string()
    };
    let new_mpd = client
        .get(&mpd_url)
        .send()
        .await
        .expect("new mpd")
        .text()
        .await
        .expect("new mpd body");
    assert_eq!(
        attribute(&new_mpd, "availabilityStartTime"),
        attribute(&old_mpd, "availabilityStartTime"),
        "{new_mpd}"
    );
    assert_eq!(
        attribute(&new_mpd, "startNumber"),
        attribute(&old_mpd, "startNumber"),
        "{new_mpd}"
    );

    server.abort();
    let _ = std::fs::remove_file(&config_path);
}

/// `Config` is `Deserialize`-only (no `Serialize` — several nested types
/// carry credentials that must never round-trip to JSON in production), so
/// this test-only helper hand-builds the equivalent JSON `serde_json::Value`
/// from the handful of fields these tests actually vary, mirroring the
/// config-file shape a real operator would hand-author (see this crate's
/// README "Config shape" section).
fn initial_as_json(config: &Config) -> serde_json::Value {
    let routes: Vec<serde_json::Value> = config.routes.iter().map(route_json).collect();
    let admin = config.admin.as_ref().map(|a| {
        serde_json::json!({
            "bind": a.bind,
            "auth": { "scheme": "bearer", "token": ADMIN_TOKEN },
        })
    });
    serde_json::json!({
        "bind": config.bind,
        "target_duration_secs": config.target_duration_secs,
        "part_target_ms": config.part_target_ms,
        "window_segments": config.window_segments,
        "routes": routes,
        "admin": admin,
    })
}

// --- W5 (audit run 7): path traversal via a route name, and route-add with
// a name that would panic the router build ---

/// A `POST /admin/routes` whose name contains an encoded path-traversal
/// component must be rejected (`400`) and must NEVER create a route whose
/// archive/dvr writes land outside `archive_root`. The name is the DVR's
/// on-disk directory component, so `..` would escape the archive root.
#[tokio::test]
async fn admin_add_rejects_a_traversal_route_name() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![unreachable_rtsp_route("seed", "seed")],
    );
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();

    for bad in ["..", ".", "a*b", "a/b"] {
        let resp = admin_json(
            &client,
            admin_addr,
            reqwest::Method::POST,
            "/admin/routes",
            Some(ADMIN_TOKEN),
            Some(serde_json::json!({
                "name": bad,
                "input": { "type": "rtsp", "url": "rtsp://127.0.0.1:1/x" },
                "outputs": ["llhls"],
            })),
        )
        .await;
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "route name {bad:?} must be rejected with 400"
        );
    }

    // The registry is untouched: only the seed route remains.
    let resp = admin_get(&client, admin_addr, "/admin/routes").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let routes: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        routes.as_array().map(Vec::len),
        Some(1),
        "no traversal route may have been registered: {routes}"
    );

    server.abort();
}

/// Raw-socket traversal test (issue #1083, G): `reqwest`/`url` normalise
/// `..` client-side, so the existing test never sent a real traversal. This
/// writes literal request lines — including encoded `..`, `%2f`, a backslash
/// and a NUL — straight to the socket, so the origin's own decoding is what is
/// under test. None may read outside the stream root.
#[tokio::test]
async fn raw_traversal_requests_never_read_outside_the_root() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(media_addr).await;
    let playlist = wait_until_live(&reqwest::Client::new(), media_addr, "cam1").await;
    assert!(playlist.contains("#EXTINF:"));

    const RAW_PATHS: [&str; 6] = [
        "/cam1/../../etc/passwd",
        "/cam1/%2e%2e/%2e%2e/etc/passwd",
        "/cam1/..%2f..%2fetc%2fpasswd",
        "/cam1/..%5c..%5cetc%5cpasswd",
        "/cam1/%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "/cam1/seg-1-1.m4s%00/../../etc/passwd",
    ];

    for path in RAW_PATHS {
        let raw = raw_get(media_addr, path).await;
        // The status line's code and whether any file content leaked.
        assert!(
            !raw.contains("\r\n\r\nroot:"),
            "raw path {path:?} must never return /etc/passwd contents: {raw:?}"
        );
        let status_line = raw.lines().next().unwrap_or_default();
        let code: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or_else(|| panic!("no status code in {status_line:?} for {path:?}"));
        assert!(
            (400..500).contains(&code),
            "raw path {path:?} must be a 4xx, got {code} ({status_line:?})"
        );
    }

    server.abort();
}

/// Send a literally-constructed `GET` for `path` over a raw TCP connection
/// and return the whole response as a string. No URL library is involved, so
/// the bytes on the wire are exactly `path`.
async fn raw_get(addr: SocketAddr, path: &str) -> String {
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    crate::support::raw_http::exchange(addr, &req, Duration::from_secs(10)).await
}

// --- W5/A1 (audit run 7): a router-build panic must not register the route ---

/// A `Custom` output whose `manifest_routes` panics — a genuine router-build
/// panic (the same class as an overlapping-path `Router::merge` panic),
/// reachable through the public registry with no test-only hook.
struct PanickingManifestOutput;

impl multimux::output::Output for PanickingManifestOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::Custom {
            type_tag: "boom".to_string(),
            params: serde_json::Value::Null,
        }
    }

    fn manifest_routes(&self, _route: Arc<RouteHandle>) -> axum::Router {
        panic!("router build panics on purpose (test)")
    }
}

fn panicking_router_registry() -> SchemeRegistry {
    use multimux::registry::OutputFactory;
    let mut registry = instant_registry();
    registry.register_output(
        "boom",
        Arc::new(|_ctx: &multimux::registry::OutputCtx| {
            Ok(Arc::new(PanickingManifestOutput) as Arc<dyn multimux::output::Output>)
        }) as OutputFactory,
    );
    registry
}

/// A1: a route whose outputs panic the router build must NOT be left in the
/// registry. Pre-fix, `add_route` inserted into `inner` before
/// `rebuild_router`, so a build panic left the route registered (and every
/// later rebuild panicked too), plus leaked the supervisor/push/WHEP tasks
/// and their bound ports.
#[tokio::test]
async fn a_router_build_panic_does_not_register_the_route() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    // Bind the WHEP listen socket ourselves and hand it in via `Config::prebound`
    // (the admin POST's route consumes `base_config.prebound`) — never a probe
    // address the route then races to re-bind.
    let (whep_addr, whep_listener) = bind_tcp();
    let mut config = admin_config(
        media_addr,
        admin_addr,
        vec![unreachable_rtsp_route("seed", "seed")],
    );
    config
        .prebound
        .with_tcp(whep_addr.to_string(), whep_listener);
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        panicking_router_registry(),
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();

    // The route carries a WHEP output (a real listen socket) *and* the
    // panicking Custom output, so a leak would show as the port staying
    // bound.
    let resp = client
        .post(format!("http://{admin_addr}/admin/routes"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&serde_json::json!({
            "name": "boom",
            "input": { "type": "rtsp", "url": "rtsp://127.0.0.1:1/boom" },
            "outputs": [
                { "custom": { "type_tag": "boom" } },
                { "whep": { "listen": whep_addr.to_string() } }
            ],
        }))
        .send()
        .await
        .expect("admin POST completes");
    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "a panicking router build must be a clean error, got {}",
        resp.status()
    );

    // The route must not be registered.
    let resp = admin_get(&client, admin_addr, "/admin/routes").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let routes: serde_json::Value = resp.json().await.unwrap();
    let names: Vec<String> = routes
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        names,
        vec!["seed".to_string()],
        "only the seed route remains"
    );

    // An ordinary route must still be addable afterwards — the registry is
    // not left in a permanently-panicking state.
    let resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        Some(ADMIN_TOKEN),
        Some(route_json(&instant_route("after"))),
    )
    .await;
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::CREATED,
        "a valid add after the router-build panic must succeed"
    );

    // The WHEP listener the route consumed from `Config::prebound` must have
    // been released with the torn-down route — the port is free. Bind it
    // ourselves to prove it (no leaked listener holding the socket).
    //
    // Only meaningful with the `whep` feature: without it no WHEP output is
    // started, so nothing consumes the pre-bound socket from `Config::prebound`
    // and the server's own config keeps holding it (AddrInUse is then
    // expected, not a leak).
    if cfg!(feature = "whep") {
        let rebound = tokio::net::TcpListener::bind(whep_addr).await;
        assert!(
            rebound.is_ok(),
            "the WHEP port must be free after the failed add (no leaked listener): {:?}",
            rebound.err()
        );
        drop(rebound);
    }

    server.abort();
}

/// A2: a `Custom` input factory that spawns its driver task (which publishes
/// a program, unblocking the WHEP egress's `await_first_trunk`) and *then*
/// returns `Err`, makes `spawn_ingest` fail AFTER the push/WHEP tasks were
/// spawned. Pre-fix those ran forever — the WHEP listener bound its port for
/// a route that was never installed.
#[tokio::test]
async fn a_failed_route_does_not_leak_its_push_or_whep_tasks() {
    use multimux::registry::InputFactory;

    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    // Bind the WHEP listen socket and hand it in via `Config::prebound`. The
    // `spawn-then-fail` factory below makes `spawn_ingest` fail BEFORE
    // `spawn_whep_outputs` runs, so the route must consume NOTHING from the map
    // — a consumed entry is exactly the pre-fix leak (the WHEP task took the
    // socket and kept it bound for a route that was never installed).
    let (whep_addr, whep_listener) = bind_tcp();
    let mut config = admin_config(
        media_addr,
        admin_addr,
        vec![unreachable_rtsp_route("seed", "seed")],
    );
    config
        .prebound
        .with_tcp(whep_addr.to_string(), whep_listener);
    // A clone shares the inner `Arc<Mutex<..>>`, so it observes the same slot
    // after the (moved) `config` has been consumed by the server.
    let prebound = config.prebound.clone();

    // A factory that spawns a real publisher (so `await_first_trunk`
    // resolves) and then fails, exactly the shape a broken third-party
    // scheme produces.
    let mut registry = instant_registry();
    registry.register_input(
        "spawn-then-fail",
        Arc::new(|ctx: InputCtx| {
            tokio::spawn(multimux::supervise_driver(
                run_instant,
                ctx.store,
                Backoff::production_default(),
                ctx.name,
                ctx.cancel,
            ));
            Err(multimux::MultimuxError::UnknownScheme {
                kind: "input",
                tag: "spawn-then-fail".to_string(),
            })
        }) as InputFactory,
    );

    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        registry,
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{admin_addr}/admin/routes"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&serde_json::json!({
            "name": "bad",
            "input": { "type": "custom", "type_tag": "spawn-then-fail" },
            "outputs": [ { "whep": { "listen": whep_addr.to_string() } } ],
        }))
        .send()
        .await
        .expect("admin POST completes");
    assert!(
        resp.status().is_client_error(),
        "a failing Custom input factory must be a clean error, got {}",
        resp.status()
    );

    // The pre-bound WHEP listener must NOT have been consumed: `spawn_ingest`
    // failed first, so `spawn_whep_outputs` never ran (pre-fix it ran first and
    // its detached task kept the socket, i.e. the entry was taken and never
    // released). Untouched == the WHEP egress never started for this route.
    assert!(
        prebound
            .tcp
            .lock()
            .expect("prebound tcp lock")
            .contains_key(&whep_addr.to_string()),
        "the failed route consumed the pre-bound WHEP listener -- its egress          started for a route that was never installed (leaked task)"
    );

    server.abort();
}

// --- W6/A1 (audit run 7): reload rollback + displaced drain ---

/// A reload whose file contains an unbuildable route must apply NOTHING: the
/// existing routes keep serving and the port a would-be-added WHEP listener
/// wanted is never left bound (the rollback cancels/aborts the prepared
/// runtimes — issue #1083, A2/G).
#[cfg(feature = "whep")]
#[tokio::test]
async fn a_failed_reload_rolls_back_and_leaves_no_bound_port() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    // Bind the WHEP listen socket the reloaded file's "keep" route names and
    // hand it in via `Config::prebound` (spliced into the file-loaded config by
    // the `_prebound` entry point) — never a probe address the route re-binds.
    let (whep_addr, whep_listener) = bind_tcp();
    let mut prebound = multimux::config::PreboundBinds::default();
    prebound.with_tcp(whep_addr.to_string(), whep_listener);

    let config_path = std::env::temp_dir().join(format!(
        "multimux-admin-api-test-reload-rollback-{}-{}.json",
        std::process::id(),
        admin_addr.port()
    ));
    let initial = admin_config(media_addr, admin_addr, vec![instant_route("wide")]);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&initial_as_json(&initial)).expect("serialize"),
    )
    .expect("write config");

    // A registry whose "fails-at-build" input factory *passes* standalone
    // validation but returns `Err` when `spawn_route` invokes it — so the
    // reload reaches the build loop and actually rolls back (an unknown tag
    // would instead be rejected at validation, before anything is built).
    let mut registry = instant_registry();
    registry.register_input(
        "fails-at-build",
        Arc::new(|_ctx: InputCtx| {
            Err(multimux::MultimuxError::UnknownScheme {
                kind: "input",
                tag: "fails-at-build".to_string(),
            })
        }) as InputFactory,
    );

    let server = tokio::spawn(multimux::serve_config_file_with_registry_on_admin_prebound(
        media_listener,
        admin_listener,
        config_path.clone(),
        registry,
        prebound,
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();
    wait_until_live(&client, media_addr, "wide").await;

    // A file that ADDS "keep" (carrying a WHEP output — a real listen socket a
    // pre-fix rollback would leak) and simultaneously CHANGES "wide" to an
    // input that fails while being built. Reload builds ADDED routes before
    // CHANGED ones, so "keep" is spawned (and consumes its pre-bound listener)
    // before "wide" fails — exercising the real rollback path.
    let mut cfg = initial_as_json(&admin_config(
        media_addr,
        admin_addr,
        vec![instant_route("wide")],
    ));
    cfg["routes"] = serde_json::json!([
        { "name": "keep",
          "input": { "type": "custom", "type_tag": "instant" },
          "outputs": [ { "whep": { "listen": whep_addr.to_string() } } ] },
        { "name": "wide",
          "input": { "type": "custom", "type_tag": "fails-at-build" },
          "outputs": [ "llhls" ] },
    ]);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&cfg).expect("serialize"),
    )
    .expect("rewrite config");

    let resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/reload",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert!(
        resp.status().is_client_error(),
        "a reload with an unbuildable route must fail, got {}",
        resp.status()
    );

    // The original route still serves (rollback applied nothing).
    let playlist = wait_until_live(&client, media_addr, "wide").await;
    assert!(
        playlist.contains("#EXTINF:"),
        "the original route must survive"
    );

    // The rolled-back batch must have released the WHEP listener it consumed
    // from `Config::prebound` — the port is free. Poll so a torn-down task has
    // every chance to drop the socket.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match tokio::net::TcpListener::bind(whep_addr).await {
            Ok(l) => {
                drop(l);
                break;
            }
            Err(e) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the WHEP port must be free after a rolled-back reload: {e}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    server.abort();
    let _ = std::fs::remove_file(&config_path);
}

/// A reload that RESTARTS a route (same name, changed config) must drain the
/// displaced runtime — its old WHEP listen port must be released and the new
/// one bound, rather than both lingering (issue #1083, A1/G).
#[cfg(feature = "whep")]
#[tokio::test]
async fn reloading_a_route_drains_the_displaced_runtime() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    // Bind BOTH WHEP listen sockets the two config revisions name and hand
    // them in via `Config::prebound` — never probe addresses the routes race
    // to re-bind.
    let (old_whep, old_listener) = bind_tcp();
    let (new_whep, new_listener) = bind_tcp();
    let mut prebound = multimux::config::PreboundBinds::default();
    prebound.with_tcp(old_whep.to_string(), old_listener);
    prebound.with_tcp(new_whep.to_string(), new_listener);

    let config_path = std::env::temp_dir().join(format!(
        "multimux-admin-api-test-reload-restart-{}-{}.json",
        std::process::id(),
        admin_addr.port()
    ));
    let mut cfg = initial_as_json(&admin_config(
        media_addr,
        admin_addr,
        vec![instant_route("cam")],
    ));
    cfg["routes"] = serde_json::json!([
        { "name": "cam",
          "input": { "type": "custom", "type_tag": "instant" },
          "outputs": [ { "whep": { "listen": old_whep.to_string() } } ] },
    ]);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&cfg).expect("serialize"),
    )
    .expect("write config");

    let server = tokio::spawn(multimux::serve_config_file_with_registry_on_admin_prebound(
        media_listener,
        admin_listener,
        config_path.clone(),
        instant_registry(),
        prebound,
    ));
    wait_for_port(admin_addr).await;
    let client = reqwest::Client::new();

    // Wait for the old WHEP listener to bind.
    wait_for_port(old_whep).await;

    // Change the route's WHEP listen address: the reload restarts it.
    cfg["routes"] = serde_json::json!([
        { "name": "cam",
          "input": { "type": "custom", "type_tag": "instant" },
          "outputs": [ { "whep": { "listen": new_whep.to_string() } } ] },
    ]);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&cfg).expect("serialize"),
    )
    .expect("rewrite config");

    let resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/reload",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // The new port binds ...
    wait_for_port(new_whep).await;
    // ... and the OLD one is released (the displaced runtime was drained).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::net::TcpListener::bind(old_whep).await {
            Ok(l) => {
                drop(l);
                break;
            }
            Err(e) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the displaced runtime's old WHEP port must be released: {e}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    server.abort();
    let _ = std::fs::remove_file(&config_path);
}

/// Admin-added routes inherit the origin's `program_date_time` default
/// (arrival): a freshly added route's media playlist carries
/// `#EXT-X-PROGRAM-DATE-TIME` without any per-route config knob.
#[tokio::test]
async fn admin_added_route_emits_pdt_by_default() {
    let (media_addr, media_listener) = bind_tcp();
    let (admin_addr, admin_listener) = bind_tcp();
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![unreachable_rtsp_route("seed", "seed")],
    );
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        config,
        instant_registry(),
    ));
    wait_for_port(admin_addr).await;

    let client = reqwest::Client::new();
    let resp = admin_json(
        &client,
        admin_addr,
        reqwest::Method::POST,
        "/admin/routes",
        Some(ADMIN_TOKEN),
        Some(route_json(&instant_route("newcam"))),
    )
    .await;
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    let playlist = wait_until_live(&client, media_addr, "newcam").await;
    assert!(
        playlist.contains("EXT-X-PROGRAM-DATE-TIME"),
        "admin-added route must emit PDT by default:\\n{playlist}"
    );

    server.abort();
}
