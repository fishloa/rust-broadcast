//! Integration tests for the runtime admin API (issue #749): add/remove/list
//! routes and reload the config file without restarting the origin.
//!
//! Every test drives the *real* HTTP surface (two real bound TCP listeners —
//! media and admin — real `reqwest` requests), never `RouteRegistry`
//! directly (that type is crate-private): this is the same "drive it through
//! the real dispatch path" discipline `multimux/tests/dispatch_ingest.rs`
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
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

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
use multimux::{Backoff, SchemeRegistry, serve_config_file_with_registry, serve_with_registry};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

const ADMIN_TOKEN: &str = "admin-test-token";

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero capacity")
}

/// Reserves a free TCP port, then immediately releases it — the same
/// "reserve then drop, hand the exact address to the thing that binds it"
/// pattern `multimux/tests/dispatch_ingest.rs` uses.
fn reserve_tcp_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
}

/// Waits until `addr` accepts a bare TCP connection — the origin's own
/// listener bind happens inside the freshly-`tokio::spawn`ed server task, so
/// a request sent immediately after `tokio::spawn` can race it (especially
/// the *admin* listener, bound after the media one and after every startup
/// route's `add_route` call). A generous hang guard, not a latency
/// assertion.
async fn wait_for_port(addr: SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("nothing ever accepted a connection on {addr} within the hang guard");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

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
        1,
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
                    track_id: 1,
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

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

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
    let trunk_config = TrunkConfig::new(nz(64), nz(16), nz(8), nz(64), nz(64));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver: IngestDriver<InstantSession> = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let mut progress = DriverProgress::new();
    driver.feed(&[], Timestamp::from_nanos(0));
    advance_route(&driver, &route_handle, &mut progress);
    driver.finish();
    advance_route(&driver, &route_handle, &mut progress);
    Ok(())
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
                ctx.shutdown_rx,
            )))
        }) as InputFactory,
    );
    registry
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
async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.text().await {
                    if body.contains("#EXTINF:") {
                        return body;
                    }
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "no #EXTINF: line appeared in {playlist_url} within the hang guard -- route \
                 never produced a closed segment"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    // `Config::validate` rejects an empty `routes` list, so start with one
    // real (but otherwise irrelevant) seed route already configured, then
    // add a brand-new second one at runtime -- exactly the real "add a
    // 41st camera without disturbing the other 40" scenario.
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![unreachable_rtsp_route("seed", "seed")],
    );
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    let config = admin_config(
        media_addr,
        admin_addr,
        vec![instant_route("cam1"), instant_route("cam2")],
    );
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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

/// Test 3: `POST` a duplicate name -> `409`, original route untouched and
/// still live.
#[tokio::test]
async fn post_duplicate_name_is_conflict_and_original_stays_live() {
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();
    let config = admin_config(media_addr, admin_addr, vec![instant_route("cam1")]);
    let server = tokio::spawn(serve_with_registry(config, instant_registry()));
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
    let media_addr = reserve_tcp_addr();
    let admin_addr = reserve_tcp_addr();

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

    let server = tokio::spawn(serve_config_file_with_registry(
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
