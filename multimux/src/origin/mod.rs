//! HTTP origin server for stream delivery.
//!
//! Wires a per-stream [`crate::store::MediaStore`] to the axum sub-routers of
//! each stream's configured [`crate::output::Output`]s (LL-HLS today; DASH in
//! future), mounting each output's routes under `/{stream}/`. See
//! [`crate::output::llhls`] for the LL-HLS output itself (playlists, byte
//! ranges, bounded blocking playlist reload — RFC 8216bis §6.2.5.2).
//!
//! Also mounts three root-level (not `/{stream}/`-scoped) operability
//! endpoints (issue #663, P1c) — see [`router`]:
//! - `GET /metrics` — Prometheus text exposition ([`crate::prometheus`]).
//! - `GET /healthz` — liveness.
//! - `GET /readyz` — readiness.
//!
//! Every request the origin serves (root endpoints included) passes through
//! `track_http`, an axum middleware layer recording HTTP request/latency/
//! byte metrics.

pub mod supervisor;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::sync::watch;

use crate::output::Output;
use crate::output::llhls::LlHlsOutput;
use crate::store::{HealthState, MediaStore};
use supervisor::{Backoff, supervise};

/// How long `serve` waits for a route's supervisor task to notice shutdown
/// and return on its own, after axum has finished draining in-flight HTTP
/// requests, before forcibly aborting it. Generous relative to the tiny
/// `tokio::select!` the supervisor uses to make its backoff sleep
/// cancellable — this is just a backstop for a task wedged in a connect()
/// or pipeline call that doesn't itself observe shutdown mid-flight.
const SUPERVISOR_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// One stream's shared store plus the `Output`s configured to serve it.
pub type StreamRoute = (Arc<MediaStore>, Vec<Arc<dyn Output>>);

/// Shared HTTP origin state: one [`MediaStore`] plus its configured
/// [`Output`]s per served stream name, keyed by the stream name used in the
/// URL path (`/:stream/...`), plus the process-wide Prometheus metrics handle
/// rendered by `GET /metrics`.
pub struct AppState {
    /// Served stream name -> its rolling in-RAM store and the outputs
    /// serving it.
    pub streams: HashMap<String, StreamRoute>,
    /// Renders the current Prometheus text-exposition snapshot of every
    /// metric recorded anywhere in the process (see [`crate::prometheus`]).
    pub metrics_handle: PrometheusHandle,
}

impl AppState {
    /// Build a new `AppState` serving `streams`, installing (or — if one is
    /// already installed in this process, e.g. by another `AppState` built
    /// earlier in the same test binary — reusing) the process-wide
    /// Prometheus recorder via [`crate::prometheus::install`].
    pub fn new(streams: HashMap<String, StreamRoute>) -> Self {
        AppState {
            streams,
            metrics_handle: crate::prometheus::install(),
        }
    }
}

/// Build the axum router serving `state`'s streams plus the root operability
/// endpoints:
///
/// - For each stream, every configured `Output`'s router is merged
///   (`nest`ed) under `/{stream}/`, all sharing that stream's one
///   [`MediaStore`]. A request for a stream name not present in
///   `state.streams` matches no nest and 404s, same as an unknown filename
///   within a known stream 404s inside the output's own router.
/// - `GET /metrics`, `GET /healthz`, `GET /readyz` are mounted at the root
///   (never under `/{stream}/`) — see `metrics_handler`, `healthz`,
///   `readyz` (all crate-private handlers, below).
///
/// Every request — matched or not, root or per-stream — passes through
/// `track_http`, a global middleware layer recording HTTP request/
/// duration/byte metrics (applied via `.layer`, which wraps the whole router
/// including its 404 fallback, unlike `.route_layer` which only wraps
/// matched routes).
pub fn router(state: Arc<AppState>) -> Router {
    let mut router = Router::new();
    for (name, (store, outputs)) in &state.streams {
        for output in outputs {
            router = router.nest(&format!("/{name}"), output.router(store.clone()));
        }
    }

    let root = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state.clone());

    router
        .merge(root)
        .layer(middleware::from_fn_with_state(state, track_http))
}

/// `GET /metrics` — the process's current Prometheus text-exposition
/// snapshot: every metric recorded anywhere in the process via the `metrics`
/// crate's macros (ingest health/reconnects, segment/part production,
/// blocking-request concurrency, HTTP request volume/latency/bytes — see
/// [`crate::prometheus`]).
async fn metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    let body = state.metrics_handle.render();
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

/// `GET /healthz` — liveness: `200 OK` whenever the process is up and
/// answering HTTP requests at all, regardless of any route's ingest state.
/// A process manager restarts the origin if this ever stops responding.
async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// `GET /readyz` — readiness: `200 OK` once **at least one** configured
/// route's [`HealthState`] is `Live`, `503 Service Unavailable` otherwise
/// (including before any route has ever connected, and if every route is
/// currently down). Chosen policy: an origin serving several streams has
/// *something* playable to offer as soon as any one of them is live, so a
/// load balancer should start sending it traffic — per-route health for
/// routing/alerting decisions on a *specific* stream is exposed separately
/// via the [`crate::prometheus::ROUTE_UP`] gauge, not this endpoint. An
/// origin configured with zero routes is never ready.
async fn readyz(State(state): State<Arc<AppState>>) -> Response {
    let any_live = state
        .streams
        .values()
        .any(|(store, _)| store.health() == HealthState::Live);
    if any_live {
        StatusCode::OK.into_response()
    } else {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

/// Classify a request path into `(route, path-kind)` labels for the HTTP
/// metrics [`track_http`] records, keeping cardinality bounded: `route` is
/// either a name present in `state.streams` or the fixed token `"unknown"`
/// (never an arbitrary/attacker-controlled path segment), and the returned
/// path-kind is one of a small fixed set — never a raw filename/URI.
fn classify_path(state: &AppState, path: &str) -> (String, &'static str) {
    match path {
        "/metrics" => return ("-".to_string(), "metrics"),
        "/healthz" | "/readyz" => return ("-".to_string(), "health"),
        _ => {}
    }
    let mut segments = path.trim_start_matches('/').splitn(2, '/');
    let first = segments.next().unwrap_or("");
    let rest = segments.next().unwrap_or("");
    let route = if state.streams.contains_key(first) {
        first.to_string()
    } else {
        "unknown".to_string()
    };
    let kind = if rest.ends_with("master.m3u8") || rest.ends_with("media.m3u8") {
        "playlist"
    } else if rest.starts_with("seg-") {
        "segment"
    } else if rest.starts_with("part-") {
        "part"
    } else if rest.starts_with("init-") {
        "init"
    } else {
        "other"
    };
    (route, kind)
}

/// Global HTTP middleware (mounted via `.layer` in [`router`]): records
/// [`crate::prometheus::HTTP_REQUESTS_TOTAL`],
/// [`crate::prometheus::HTTP_REQUEST_DURATION_SECONDS`], and
/// [`crate::prometheus::BYTES_SERVED_TOTAL`] for *every* request the origin
/// serves — root endpoints and per-stream routes alike, matched or 404.
///
/// Buffers the response body (`axum::body::to_bytes`) to get an exact byte
/// count regardless of whether a handler set `Content-Length` — every
/// response this origin produces today is already a bounded in-memory
/// buffer (no streaming bodies yet), so this adds no meaningful overhead.
async fn track_http(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let start = std::time::Instant::now();
    let resp = next.run(req).await;
    let elapsed = start.elapsed();

    let (route, kind) = classify_path(&state, &path);
    let status = resp.status().as_u16().to_string();

    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    let byte_len = bytes.len() as u64;

    metrics::counter!(
        crate::prometheus::HTTP_REQUESTS_TOTAL,
        "route" => route.clone(),
        "path" => kind,
        "status" => status,
    )
    .increment(1);
    metrics::histogram!(
        crate::prometheus::HTTP_REQUEST_DURATION_SECONDS,
        "route" => route.clone(),
        "path" => kind,
    )
    .record(elapsed.as_secs_f64());
    metrics::counter!(
        crate::prometheus::BYTES_SERVED_TOTAL,
        "route" => route,
        "path" => kind,
    )
    .increment(byte_len);

    Response::from_parts(parts, Body::from(bytes))
}

/// Run the multimux origin: one [`MediaStore`] + one supervised ingest task
/// per `config.routes` entry, then bind `config.bind` and serve them all
/// under [`router`]. Each route is served by a single [`LlHlsOutput`] (the
/// default — and today the only — output wiring).
///
/// Each route's task is driven by [`supervisor::supervise`]: depending on
/// that route's [`crate::config::InputSpec`], it connects
/// [`crate::source::rtsp::RtspSource`], [`crate::source::rtp_udp::RtpUdpSource`],
/// [`crate::source::ts_udp::TsUdpSource`], [`crate::source::ts_http::TsHttpSource`],
/// or [`crate::source::hls_pull::HlsPullSource`] — one `match` arm per variant,
/// each instantiating the generic `supervise::<ThatConnector>` (the
/// connectors have different `Source` associated types, so this dispatch
/// stays monomorphized rather than boxed) — runs it through
/// [`crate::pipeline::run_pipeline`], and on either a connect failure, a
/// pipeline error, or source end-of-stream, reconnects with capped backoff
/// instead of dying — a bad/flaky source degrades that route's
/// [`crate::store::MediaStore::health`] rather than freezing it forever, and
/// never brings the server (or any other route) down.
///
/// Installs a graceful-shutdown signal (Ctrl-C, plus SIGTERM on unix): on
/// receipt, axum stops accepting new connections and drains in-flight
/// requests (including blocked LL-HLS long-poll reloads) via
/// [`axum::serve::Serve::with_graceful_shutdown`], the same signal breaks
/// every route's supervise loop, and `serve` joins each supervisor task
/// (forcibly aborting one that doesn't return within a short grace period)
/// before returning `Ok(())`.
///
/// Otherwise returns only on a bind failure or if the HTTP server itself
/// stops (e.g. a fatal accept-loop I/O error).
pub async fn serve(config: crate::config::Config) -> crate::Result<()> {
    config.validate()?;

    tracing::info!(
        bind = %config.bind,
        routes = config.routes.len(),
        "multimux origin starting"
    );

    let mut streams: HashMap<String, StreamRoute> = HashMap::new();
    let target_duration_secs = config.target_duration_secs;
    let part_target_ms = config.part_target_ms;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut supervisor_handles: Vec<(String, tokio::task::JoinHandle<()>)> = Vec::new();

    for route in &config.routes {
        let store = Arc::new(MediaStore::new(
            target_duration_secs,
            part_target_ms,
            config.window_segments,
        ));
        let outputs: Vec<Arc<dyn Output>> = vec![Arc::new(LlHlsOutput)];
        streams.insert(route.name.clone(), (store.clone(), outputs));

        let name = route.name.clone();
        let shutdown_rx = shutdown_rx.clone();
        let handle = match &route.input {
            crate::config::InputSpec::Rtsp { url } => {
                let connector = crate::source::rtsp::RtspSource::new(name.clone(), url.clone());
                tokio::spawn(supervise(
                    connector,
                    store,
                    target_duration_secs,
                    part_target_ms,
                    Backoff::production_default(),
                    name.clone(),
                    shutdown_rx,
                ))
            }
            crate::config::InputSpec::Rtp {
                addr,
                sdp,
                multicast_group,
            } => {
                let connector = crate::source::rtp_udp::RtpUdpSource::new(
                    name.clone(),
                    addr.clone(),
                    sdp.clone(),
                    multicast_group.clone(),
                );
                tokio::spawn(supervise(
                    connector,
                    store,
                    target_duration_secs,
                    part_target_ms,
                    Backoff::production_default(),
                    name.clone(),
                    shutdown_rx,
                ))
            }
            crate::config::InputSpec::TsUdp {
                addr,
                multicast_group,
            } => {
                let connector = crate::source::ts_udp::TsUdpSource::new(
                    name.clone(),
                    addr.clone(),
                    multicast_group.clone(),
                );
                tokio::spawn(supervise(
                    connector,
                    store,
                    target_duration_secs,
                    part_target_ms,
                    Backoff::production_default(),
                    name.clone(),
                    shutdown_rx,
                ))
            }
            crate::config::InputSpec::TsHttp { url } => {
                let connector =
                    crate::source::ts_http::TsHttpSource::new(name.clone(), url.clone());
                tokio::spawn(supervise(
                    connector,
                    store,
                    target_duration_secs,
                    part_target_ms,
                    Backoff::production_default(),
                    name.clone(),
                    shutdown_rx,
                ))
            }
            crate::config::InputSpec::HlsPull { url } => {
                let connector =
                    crate::source::hls_pull::HlsPullSource::new(name.clone(), url.clone());
                tokio::spawn(supervise(
                    connector,
                    store,
                    target_duration_secs,
                    part_target_ms,
                    Backoff::production_default(),
                    name.clone(),
                    shutdown_rx,
                ))
            }
        };
        supervisor_handles.push((name, handle));
    }

    let state = Arc::new(AppState::new(streams));
    let listener = tokio::net::TcpListener::bind(config.bind.as_str()).await?;
    let shutdown_future = async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        // Best-effort: only fails if every receiver (every supervisor task)
        // has already exited, which just means there's nothing left to
        // notify.
        let _ = shutdown_tx.send(true);
    };
    let serve_result = axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_future)
        .await;

    // axum has stopped accepting connections and drained in-flight requests
    // by the time `.await` above returns (whether that's because shutdown
    // fired, or because the accept loop itself errored out) — join every
    // route's supervisor task in orderly fashion, aborting any stragglers
    // rather than leaving them running detached past `serve`'s return.
    for (name, handle) in supervisor_handles {
        let abort_handle = handle.abort_handle();
        if tokio::time::timeout(SUPERVISOR_SHUTDOWN_GRACE, handle)
            .await
            .is_err()
        {
            tracing::warn!(
                route = %name,
                "supervisor task did not exit within the shutdown grace period; aborting"
            );
            abort_handle.abort();
        }
    }

    serve_result?;
    Ok(())
}

/// Resolves once an external shutdown signal is received: Ctrl-C
/// (`SIGINT`) on every platform, plus `SIGTERM` on unix (the signal a
/// process manager / `docker stop` / `systemd` sends for a graceful stop).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MediaStore;
    use tower::ServiceExt;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (store, vec![Arc::new(LlHlsOutput) as Arc<dyn Output>]),
        );
        Arc::new(AppState::new(streams))
    }

    fn get(uri: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// Sanity-checks real axum route dispatch (not just the handler
    /// functions in isolation): the static `master.m3u8`/`media.m3u8` routes
    /// must win over the `:file` catch-all registered for the same
    /// `/:stream/*` prefix, and the catch-all must still serve dynamic
    /// filenames.
    #[tokio::test]
    async fn router_dispatches_static_routes_over_catch_all() {
        let app = router(make_state());

        let resp = app.clone().oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .contains("#EXT-X-STREAM-INF")
        );

        let resp = app.clone().oneshot(get("/cam1/init-1.mp4")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(bytes.to_vec(), vec![0xAA; 4]);

        let resp = app.oneshot(get("/cam1/no-such-file.bin")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    // --- "unknown stream" 404 tests, moved from `output::llhls`'s handlers:
    // a stream name absent from `state.streams` matches no nested output
    // router at all, so every route under it 404s — same externally-visible
    // behaviour as the pre-refactor per-handler `contains_key` check, now
    // proven at the router-dispatch level since the handlers themselves no
    // longer know about stream names. ---

    #[tokio::test]
    async fn master_playlist_unknown_stream_404() {
        let app = router(make_state());
        let resp = app.oneshot(get("/nope/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_playlist_unknown_stream_404() {
        let app = router(make_state());
        let resp = app.oneshot(get("/nope/media.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_unknown_stream_404() {
        let app = router(make_state());
        let resp = app.oneshot(get("/nope/init-1.mp4")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    // --- Observability endpoints (issue #663, P1c) ---

    async fn body_string(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Biting test 1: `GET /metrics` must actually serve a Prometheus text
    /// exposition body (not an empty/placeholder 200) — a request is made
    /// first so at least one `multimux_http_requests_total` series is
    /// guaranteed to exist by the time `/metrics` is rendered, regardless of
    /// whatever else has (or hasn't) run earlier in this process.
    #[tokio::test]
    async fn metrics_endpoint_serves_prometheus_exposition() {
        let app = router(make_state());

        let warm = app.clone().oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(warm.status(), axum::http::StatusCode::OK);

        let resp = app.oneshot(get("/metrics")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "text/plain; version=0.0.4"
        );
        let body = body_string(resp).await;
        assert!(
            body.contains("multimux_"),
            "metrics body must contain at least one multimux_ metric: {body}"
        );
    }

    /// Biting test 2: `GET /healthz` is always 200 — liveness, independent
    /// of any route's ingest state.
    #[tokio::test]
    async fn healthz_always_200() {
        let app = router(make_state());
        let resp = app.oneshot(get("/healthz")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Biting test 3a: `GET /readyz` must 503 when no route is `Live` —
    /// `make_state()`'s store defaults to `HealthState::Connecting` (never
    /// set `Live`). A `/readyz` that ignored health entirely (always 200)
    /// would fail this case.
    #[tokio::test]
    async fn readyz_503_when_no_route_live() {
        let app = router(make_state());
        let resp = app.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Biting test 3b: `GET /readyz` must 200 once a route's `MediaStore` is
    /// `Live` — the counterpart to 3a, proving `/readyz` actually reads
    /// `HealthState` rather than being hardcoded to one status.
    #[tokio::test]
    async fn readyz_200_when_a_route_is_live() {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 4]);
        store.set_health(HealthState::Live);
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (store, vec![Arc::new(LlHlsOutput) as Arc<dyn Output>]),
        );
        let app = router(Arc::new(AppState::new(streams)));
        let resp = app.oneshot(get("/readyz")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Extract a rendered Prometheus metric's value: the first line starting
    /// with `metric` whose label set contains every string in `must_contain`,
    /// parsed as the trailing whitespace-separated value. `0.0` if no such
    /// line exists (a metric/label combination never recorded reads as
    /// "never incremented", the natural zero baseline for a delta assertion).
    fn metric_value(rendered: &str, metric: &str, must_contain: &[&str]) -> f64 {
        rendered
            .lines()
            .find(|l| l.starts_with(metric) && must_contain.iter().all(|s| l.contains(s)))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    }

    /// Biting test 4: serving real HTTP requests must actually move a
    /// metric's rendered *value*, not just cause its name to appear. Uses a
    /// stream name (`metrics-probe`) not touched by any other test in this
    /// file, so the before/after snapshot is a clean delta regardless of
    /// what other tests have recorded under other route labels. A no-op
    /// recorder (or a `track_http` that never actually calls
    /// `metrics::counter!`) would leave `after == before == 0.0` and fail
    /// this assertion.
    #[tokio::test]
    async fn http_requests_total_counter_increases_on_requests() {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert(
            "metrics-probe".to_string(),
            (store, vec![Arc::new(LlHlsOutput) as Arc<dyn Output>]),
        );
        let state = Arc::new(AppState::new(streams));
        let app = router(state.clone());

        let labels = [
            "route=\"metrics-probe\"",
            "path=\"playlist\"",
            "status=\"200\"",
        ];
        let before = metric_value(
            &state.metrics_handle.render(),
            "multimux_http_requests_total",
            &labels,
        );

        const REQUESTS: usize = 3;
        for _ in 0..REQUESTS {
            let resp = app
                .clone()
                .oneshot(get("/metrics-probe/master.m3u8"))
                .await
                .unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::OK);
        }

        let after = metric_value(
            &state.metrics_handle.render(),
            "multimux_http_requests_total",
            &labels,
        );
        assert_eq!(
            after - before,
            REQUESTS as f64,
            "multimux_http_requests_total must increase by exactly the number of requests made"
        );
    }
}
