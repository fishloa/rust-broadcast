//! HTTP origin server for stream delivery.
//!
//! Wires a per-stream [`crate::route::RouteHandle`] to the axum sub-routers of
//! each stream's configured [`crate::output::Output`]s (LL-HLS, DASH — issue
//! #663 P4), mounting under `/{stream}/`:
//! - the **shared resource route** (`resource`) — `init-*.mp4`/`seg-*.m4s`/
//!   `part-*.m4s` byte serving, identical for every output since LL-HLS and
//!   DASH are both fMP4/CMAF over the same produced bytes. Mounted **once
//!   per stream**, not per-output (two outputs each mounting their own
//!   `/:file` catch-all under the same nest previously panicked axum — the
//!   "multi-output nest collision" this module fixes).
//! - each configured output's **manifest routes**
//!   ([`crate::output::Output::manifest_routes`]) — `master.m3u8`/
//!   `media.m3u8` for LL-HLS ([`crate::output::llhls`]), `manifest.mpd` for
//!   DASH ([`crate::output::dash`]).
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
//!
//! # Shared output auth (issue #663 "shared output auth")
//!
//! When [`crate::config::Config::output_auth`] is configured, one
//! [`broadcast_auth::Verifier`] gates **every** media output route
//! (`/{stream}/…` — manifests and the shared resource route alike, across
//! every configured stream) via `output_auth_gate`, mounted on the
//! per-stream nests *before* they are merged with the root ops endpoints —
//! so `/metrics`/`/healthz`/`/readyz` are never behind it (load balancer
//! probes and metrics scraping must stay open regardless of output auth).
//! This is intentionally independent of any route's own ingest auth
//! (`crate::config::AuthSpec`/URL userinfo): one output credential guards
//! every stream this origin serves (e.g. 40 cameras under
//! `/camN/index.m3u8`), regardless of how differently each camera
//! authenticates its own upstream feed. `output_auth: None` (the default)
//! leaves every output route open, unchanged from pre-#663 behaviour.

pub mod admin;
pub(crate) mod resource;
pub mod supervisor;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_auth::{AuthResult, Verifier};
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::sync::watch;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use crate::output::Output;
use crate::registry::{AuthCtx, InputCtx, OutputCtx, SchemeRegistry};
use crate::route::{HealthState, RouteHandle};
use supervisor::Backoff;

/// Realm advertised by the shared output-auth `Verifier`'s Basic/Digest
/// challenge (`crate::config::OutputAuthSpec`) — fixed rather than
/// per-config, since it names the origin itself, not any individual camera.
const OUTPUT_AUTH_REALM: &str = "multimux";

/// HTTP-layer resource limits applied process-wide by [`router`] (issue #663
/// P5, audit-concurrency #3: "slow-loris kills all routes" — with no cap
/// anywhere, one client opening many connections and never completing a
/// request, or drip-feeding one slowly, exhausts the tokio task pool/file
/// descriptors for *every* route, not just a misbehaving source). Three
/// independent bounds, applied together:
///
/// - [`Self::request_timeout`] — [`tower_http::timeout::TimeoutLayer`]:
///   unlike `tower::timeout`, this returns a `408 Request Timeout` response
///   rather than erroring the connection, so it composes directly with
///   axum's `Infallible`-error `Router` with no `HandleErrorLayer`. Must stay
///   above the LL-HLS blocking-reload cap (5 s —
///   `output::llhls`/`origin::resource`'s own `BLOCKING_RELOAD_TIMEOUT`) so
///   a legitimate long-poll `_HLS_msn`/`_HLS_part` blocking request is never
///   killed by this layer instead of resolving normally or falling back at
///   its own 5 s cap — [`crate::config::Config::validate`] enforces this.
/// - [`Self::max_concurrent_requests`] —
///   [`tower::limit::ConcurrencyLimitLayer`]: bounds how many requests (across
///   every route) are serviced at once; beyond the limit, a new request
///   simply waits for a slot rather than spawning unbounded concurrent work.
/// - [`Self::max_request_body_bytes`] —
///   [`tower_http::limit::RequestBodyLimitLayer`]: the origin only ever
///   serves `GET`s, so any non-trivial request body is already anomalous; an
///   oversized body (by `Content-Length`, checked before the body is read)
///   gets an immediate `413 Payload Too Large`.
///
/// Config-surfaced via [`crate::config::Config`] (sane defaults below);
/// [`AppState::new`] applies [`HttpLimits::default`] so existing call sites
/// (tests, examples) are unaffected, and [`AppState::with_limits`] overrides
/// it with `Config`'s configured values (wired by [`serve`]).
#[derive(Debug, Clone, Copy)]
pub struct HttpLimits {
    /// Per-request timeout — see the struct docs.
    pub request_timeout: Duration,
    /// Maximum requests serviced concurrently, across every route.
    pub max_concurrent_requests: usize,
    /// Maximum accepted request body size, in bytes.
    pub max_request_body_bytes: usize,
}

/// Default per-request timeout: comfortably above the 5 s LL-HLS
/// blocking-reload cap (double it) so an ordinary long-poll request is never
/// affected, while still bounding a genuinely stuck connection.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Default concurrent-request bound, across every configured route.
pub const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 4096;

/// Default request-body cap: 16 KiB — comfortably above anything a
/// legitimate `GET` needs (query string only, no body) and far below what a
/// slow-loris-style oversized POST would need to pressure memory.
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 16 * 1024;

impl Default for HttpLimits {
    fn default() -> Self {
        HttpLimits {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_concurrent_requests: DEFAULT_MAX_CONCURRENT_REQUESTS,
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
        }
    }
}

impl From<&crate::config::Config> for HttpLimits {
    fn from(cfg: &crate::config::Config) -> Self {
        HttpLimits {
            request_timeout: Duration::from_secs_f64(cfg.request_timeout_secs),
            max_concurrent_requests: cfg.max_concurrent_requests,
            max_request_body_bytes: cfg.max_request_body_bytes,
        }
    }
}

/// How long `serve` waits for a route's supervisor task to notice shutdown
/// and return on its own, after axum has finished draining in-flight HTTP
/// requests, before forcibly aborting it. Generous relative to the tiny
/// `tokio::select!` the supervisor uses to make its backoff sleep
/// cancellable — this is just a backstop for a task wedged in a connect()
/// or pipeline call that doesn't itself observe shutdown mid-flight.
const SUPERVISOR_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// One stream's shared store plus the `Output`s configured to serve it.
pub type StreamRoute = (Arc<RouteHandle>, Vec<Arc<dyn Output>>);

/// Shared HTTP origin state: one [`RouteHandle`] plus its configured
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
    /// HTTP-layer resource limits [`router`] applies (issue #663 P5). Defaults
    /// via [`HttpLimits::default`]; see [`Self::with_limits`].
    limits: HttpLimits,
    /// Shared output-auth verifier (issue #663 "shared output auth") gating
    /// every media output route — `None` (the default) leaves every route
    /// open. See [`Self::with_output_auth`] and this module's docs.
    output_auth: Option<Arc<Verifier>>,
}

impl AppState {
    /// Build a new `AppState` serving `streams`, installing (or — if one is
    /// already installed in this process, e.g. by another `AppState` built
    /// earlier in the same test binary — reusing) the process-wide
    /// Prometheus recorder via [`crate::prometheus::install`]. Applies
    /// [`HttpLimits::default`] and no output auth — use [`Self::with_limits`]/
    /// [`Self::with_output_auth`] to override either.
    pub fn new(streams: HashMap<String, StreamRoute>) -> Self {
        AppState {
            streams,
            metrics_handle: crate::prometheus::install(),
            limits: HttpLimits::default(),
            output_auth: None,
        }
    }

    /// Overrides the default [`HttpLimits`] — [`serve`] uses this to apply
    /// `Config`'s configured request-timeout/concurrency/body-size limits;
    /// callers that only want the defaults (most tests/examples) keep using
    /// [`Self::new`] unchanged.
    #[must_use]
    pub fn with_limits(mut self, limits: HttpLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Gates every media output route behind `verifier` (issue #663 "shared
    /// output auth") — [`serve`] uses this when
    /// [`crate::config::Config::output_auth`] is configured; callers that
    /// only want the default open behaviour (most tests/examples) keep using
    /// [`Self::new`] unchanged.
    #[must_use]
    pub fn with_output_auth(mut self, verifier: Arc<Verifier>) -> Self {
        self.output_auth = Some(verifier);
        self
    }
}

/// Build the axum router serving `state`'s streams plus the root operability
/// endpoints:
///
/// - For each stream, the shared resource route (`resource::router`) is
///   merged with every configured `Output`'s manifest routes
///   ([`Output::manifest_routes`]) — all sharing that stream's one
///   [`RouteHandle`] — into **one** router, wrapped in
///   `add_response_headers`, then `nest`ed under `/{stream}/` **once**
///   (merging first, rather than nesting each output separately, is what
///   avoids axum's duplicate-nest panic — see this module's docs). A request
///   for a stream name not present in `state.streams` matches no nest and
///   404s, same as an unknown filename within a known stream 404s inside the
///   merged router's own fallback.
/// - `GET /metrics`, `GET /healthz`, `GET /readyz` are mounted at the root
///   (never under `/{stream}/`) — see `metrics_handler`, `healthz`,
///   `readyz` (all crate-private handlers, below).
///
/// Every request — matched or not, root or per-stream — passes through, in
/// order (outermost to innermost): `track_http` (HTTP request/duration/byte
/// metrics; applied via `.layer`, which wraps the whole router including its
/// 404 fallback, unlike `.route_layer` which only wraps matched routes),
/// [`HttpLimits::max_request_body_bytes`] (rejects an oversized body by
/// `Content-Length` before it is read or a concurrency slot is spent),
/// [`HttpLimits::max_concurrent_requests`], then
/// [`HttpLimits::request_timeout`] (so the timeout clock only runs once a
/// request actually has a concurrency slot) — see [`HttpLimits`] (issue #663
/// P5, audit-concurrency #3).
pub fn router(state: Arc<AppState>) -> Router {
    let limits = state.limits;
    let mut router = Router::new();
    for (name, (store, outputs)) in &state.streams {
        let mut stream_router = resource::router(store.clone());
        for output in outputs {
            stream_router = stream_router.merge(output.manifest_routes(store.clone()));
        }
        // Shared output auth (issue #663 "shared output auth") gates every
        // route in this stream's router — layered *inside*
        // `add_response_headers` (added next) so a `401` this gate produces
        // still gets the same CORS/`Cache-Control` headers as any other
        // response (a cross-origin browser client needs the CORS headers on
        // the `401` itself to see the status/`WWW-Authenticate` at all, not
        // just on a successful `200`). Never applied to the root ops
        // endpoints (`/metrics`/`/healthz`/`/readyz`, merged in below,
        // outside this per-stream loop) — see this module's docs.
        stream_router = stream_router.layer(middleware::from_fn_with_state(
            state.clone(),
            output_auth_gate,
        ));
        stream_router = stream_router.layer(middleware::from_fn(add_response_headers));
        router = router.nest(&format!("/{name}"), stream_router);
    }

    let root = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state.clone());

    router
        .merge(root)
        .layer(TimeoutLayer::new(limits.request_timeout))
        .layer(ConcurrencyLimitLayer::new(limits.max_concurrent_requests))
        .layer(RequestBodyLimitLayer::new(limits.max_request_body_bytes))
        .layer(middleware::from_fn_with_state(state, track_http))
}

/// Middleware gating every route in the router it wraps (see [`router`], the
/// only caller — applied to the per-stream nests, never the root ops
/// endpoints) behind `state.output_auth` (issue #663 "shared output auth"):
/// a no-op pass-through when it is `None`.
///
/// `OPTIONS` (CORS preflight) requests always bypass the check: a browser's
/// preflight for a cross-origin request carrying a custom `Authorization`
/// header is itself sent *without* one (RFC 9110/Fetch — preflight never
/// includes the credentials of the request it precedes), so gating it would
/// make the preflight fail and the browser would never send the real,
/// authenticated request at all. [`resource::cors_preflight`]/each `Output`'s
/// own `OPTIONS` handler still runs, so the preflight's CORS response is
/// unaffected.
///
/// Builds a [`broadcast_auth::RequestContext`] carrying every request header
/// (not just `Authorization`) plus the transport peer address (from
/// [`ConnectInfo`], present when [`serve`] wires the router through
/// `into_make_service_with_connect_info` — `None` in a test harness that
/// `oneshot`s the router directly), so a `Forwarded`-scheme verifier
/// (`crate::config::OutputAuthSpec::Forwarded`) can read `X-Forwarded-User`/
/// `X-Forwarded-For` the same way Basic/Digest/Bearer read `Authorization` —
/// all through the one [`Verifier::verify`] call, keeping every scheme's
/// logic inside `broadcast-auth` rather than duplicated here.
async fn output_auth_gate(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(verifier) = &state.output_auth else {
        return next.run(req).await;
    };
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let method = req.method().as_str().to_string();
    // Use the pre-`nest`-rewrite URI (`OriginalUri`, e.g. `/cam1/master.m3u8`)
    // for the Digest `uri` check, not `req.uri()` (which — inside a nested
    // stream router — has had the `/cam1` prefix already stripped down to
    // `/master.m3u8` by the time this middleware runs): a real client's
    // Digest `Authorization` header is computed against the full request
    // target it actually sent, so verifying against anything else would
    // reject every legitimate Digest request.
    let uri = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|o| o.0.clone())
        .unwrap_or_else(|| req.uri().clone());
    let uri = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| uri.path().to_string());
    let headers: Vec<(&str, &str)> = req
        .headers()
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str(), v)))
        .collect();
    let peer_addr = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0);
    let mut ctx = broadcast_auth::RequestContext::new(&method, &uri).with_headers(&headers);
    if let Some(peer_addr) = peer_addr {
        ctx = ctx.with_peer_addr(peer_addr);
    }
    // Observability only (see `Verifier::forwarded_for`'s docs): surfaces
    // the proxy-forwarded client IP for a `Forwarded`-scheme verifier, no
    // trust decision is made here or in `broadcast-auth` from this value.
    if let Some(forwarded_for) = verifier.forwarded_for(&ctx) {
        tracing::debug!(%forwarded_for, "output-auth: forwarded-for header");
    }
    match verifier.verify(&ctx) {
        AuthResult::Ok => next.run(req).await,
        AuthResult::Unauthorized => {
            let mut resp = StatusCode::UNAUTHORIZED.into_response();
            if let Ok(value) = HeaderValue::from_str(&verifier.challenge()) {
                resp.headers_mut().insert(header::WWW_AUTHENTICATE, value);
            }
            resp
        }
        // `AuthResult` is `#[non_exhaustive]` (broadcast-auth may add finer-
        // grained outcomes later, e.g. a rate-limited variant) — default-deny
        // any variant this middleware doesn't yet know how to treat as
        // authenticated, rather than silently letting an unrecognized
        // outcome through.
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Router-wide middleware (mounted via `.layer` on each stream's *merged*
/// router in [`router`], so it wraps every route this stream serves —
/// manifests, resources, and the `:file` catch-all's 404 fallback alike):
/// adds `Access-Control-Allow-*` (permissive CORS — LL-HLS/DASH players are
/// commonly browsers on a different origin than the API, e.g. hls.js/dash.js)
/// and a `Cache-Control` appropriate to the resource kind — `no-cache` for a
/// manifest (`.m3u8`/`.mpd`; must always be re-fetched for liveness),
/// `max-age=31536000, immutable` for init/segment/part byte ranges (a
/// produced segment/part never changes) — to every response this router
/// serves. Applied once at the origin level (not per-`Output`) precisely
/// because it must cover the shared resource route too, which no single
/// `Output` owns.
async fn add_response_headers(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let is_manifest = path.ends_with(".m3u8") || path.ends_with(".mpd");
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if is_manifest {
            CACHE_CONTROL_MANIFEST
        } else {
            CACHE_CONTROL_IMMUTABLE
        }),
    );
    resp
}

/// `Cache-Control` for manifests (`master.m3u8`/`media.m3u8`/`manifest.mpd`):
/// they must always be re-fetched for liveness, never served stale from a
/// cache.
const CACHE_CONTROL_MANIFEST: &str = "no-cache";

/// `Cache-Control` for init/segment/part byte ranges: once produced, a given
/// URI's bytes never change (each segment/part is generated exactly once
/// under a unique filename), so these are safe to cache indefinitely.
const CACHE_CONTROL_IMMUTABLE: &str = "max-age=31536000, immutable";

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

/// Which [`hls_runtime::server::Container`] `route` must be served as (issue
/// #887): [`hls_runtime::server::Container::MpegTs`] if
/// [`crate::config::Route::outputs`] names [`crate::output::OutputKind::TsHls`],
/// else [`hls_runtime::server::Container::Fmp4`]. `ts_hls` is mutually
/// exclusive with `llhls`/`dash`/`ll_dash` on the same route
/// (`crate::config::Route::validate_standalone`, enforced before this ever
/// runs), so this check is always unambiguous. Shared by
/// [`serve_with_registry`]'s own per-route loop and
/// [`admin::RouteRegistry::spawn_route`] — the runtime `POST /admin/routes`/
/// reload path — so a `ts_hls` route added or reloaded at runtime gets
/// exactly the same container a route configured at startup does.
pub(crate) fn route_container(route: &crate::config::Route) -> hls_runtime::server::Container {
    if route
        .outputs
        .iter()
        .any(|k| matches!(k, crate::output::OutputKind::TsHls))
    {
        hls_runtime::server::Container::MpegTs
    } else {
        hls_runtime::server::Container::Fmp4
    }
}

/// Build the [`Output`]s a route's [`crate::config::Route::outputs`] names,
/// resolving any [`crate::output::OutputKind::Custom`] entry via `registry`
/// (issue #663 external scheme plugin registry) and every built-in kind via
/// [`crate::output::OutputKind::build_with_playlist_name`] — the fallible
/// counterpart [`serve_with_registry`] uses in place of a bare
/// `build_with_playlist_name` call (which would panic on `Custom`; see that
/// method's docs).
fn build_output(
    kind: &crate::output::OutputKind,
    playlist_name: &str,
    registry: &SchemeRegistry,
) -> crate::Result<Arc<dyn Output>> {
    match kind {
        crate::output::OutputKind::Custom { type_tag, params } => {
            let factory =
                registry
                    .output(type_tag)
                    .ok_or_else(|| crate::MultimuxError::UnknownScheme {
                        kind: "output",
                        tag: type_tag.clone(),
                    })?;
            factory(&OutputCtx {
                params: params.clone(),
                playlist_name,
            })
        }
        builtin => Ok(builtin.build_with_playlist_name(playlist_name)),
    }
}

/// Resolves `spec` into a real [`Verifier`] — `Custom` via `registry`'s
/// `auth` factory ([`crate::registry::SchemeRegistry::auth`]), every
/// built-in scheme via [`crate::config::OutputAuthSpec::build_verifier`].
/// Shared by the shared-output-auth verifier ([`serve_with_registry`]) and
/// the mandatory admin-auth verifier ([`admin::serve_with_admin`], issue
/// #749) — one resolution path, so the two can never drift.
fn resolve_verifier(
    spec: &crate::config::OutputAuthSpec,
    realm: &str,
    registry: &SchemeRegistry,
) -> crate::Result<broadcast_auth::Verifier> {
    match spec {
        crate::config::OutputAuthSpec::Custom { type_tag, params } => {
            let factory =
                registry
                    .auth(type_tag)
                    .ok_or_else(|| crate::MultimuxError::UnknownScheme {
                        kind: "auth",
                        tag: type_tag.clone(),
                    })?;
            factory(&AuthCtx {
                params: params.clone(),
                realm,
            })
        }
        builtin => Ok(builtin.build_verifier(realm)),
    }
}

/// Run the multimux origin with an empty [`SchemeRegistry`] — equivalent to
/// `serve_with_registry(config, SchemeRegistry::new())`. A config whose
/// route/output-auth uses a `Custom` scheme always fails with
/// [`crate::MultimuxError::UnknownScheme`] under plain `serve`; use
/// [`serve_with_registry`] with a populated registry to resolve one.
///
/// No [`crate::config::Config::admin`] reload support: `POST /admin/reload`
/// needs a config file path to re-read, which a bare in-memory [`crate::config::Config`]
/// doesn't have — use [`serve_config_file`] (or [`serve_config_file_with_registry`])
/// when the config is (or may be) loaded from a file and the admin API is
/// configured. A `serve`/`serve_with_registry` origin with the admin API
/// enabled still serves `/admin/routes` add/remove/list normally; only
/// `/admin/reload` fails (with a clear error, not a panic) — see
/// `admin::RouteRegistry::reload`.
pub async fn serve(config: crate::config::Config) -> crate::Result<()> {
    serve_with_registry(config, SchemeRegistry::new()).await
}

/// [`serve_config_file`] with a caller-supplied [`SchemeRegistry`] — see that
/// function's docs.
pub async fn serve_config_file_with_registry(
    path: impl AsRef<Path>,
    registry: SchemeRegistry,
) -> crate::Result<()> {
    let path = path.as_ref().to_path_buf();
    let config = crate::config::Config::from_json_file(&path)?;
    serve_with_registry_impl(config, registry, Some(path)).await
}

/// Load a JSON config from `path` and run the multimux origin exactly like
/// [`serve`], but also remembering `path` so, if
/// [`crate::config::Config::admin`] is configured, `POST /admin/reload`
/// (issue #749) can re-read it later — see `admin::RouteRegistry::reload`.
/// Equivalent to `serve_config_file_with_registry(path, SchemeRegistry::new())`.
pub async fn serve_config_file(path: impl AsRef<Path>) -> crate::Result<()> {
    serve_config_file_with_registry(path, SchemeRegistry::new()).await
}

/// Run the multimux origin: one [`RouteHandle`] + one ingest task per
/// `config.routes` entry, then bind `config.bind` and serve them all under
/// [`router`]. Each route is served by the [`Output`]s named in its
/// [`crate::config::Route::outputs`] (LL-HLS by default — see
/// [`crate::output::OutputKind`]).
///
/// Every configured [`crate::config::InputSpec`] variant now drives a route
/// (issue #805 tasks 2/4/5 — see `spawn_ingest`, which this function's
/// per-route loop calls) through [`supervisor::supervise_driver`] (a
/// `media_plane::ingress::IngestDriver`- (or, for a push/`Listener`-shaped
/// source like RTMP, `ListenDriver`-) driven `crate::source::*::run_*` entry
/// point — or, for [`crate::config::InputSpec::Custom`], the equivalent
/// driver loop a registered [`crate::registry::InputFactory`] builds itself),
/// publishing each announced program's driver-minted `Trunk` into the route's
/// registry via [`crate::source::advance_route`]. A connect
/// failure, protocol error, or clean end-of-stream reconnects with capped
/// backoff instead of dying — a bad/flaky source degrades that route's
/// [`crate::route::RouteHandle::health`] rather than freezing it forever,
/// and never brings the server (or any other route) down.
///
/// [`crate::config::InputSpec::Custom`]/[`crate::output::OutputKind::Custom`]/
/// [`crate::config::OutputAuthSpec::Custom`] (issue #663 external scheme
/// plugin registry) are instead resolved through `registry`: an unregistered
/// `type_tag` fails route setup with [`crate::MultimuxError::UnknownScheme`]
/// rather than panicking or silently no-opping.
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
///
/// If [`crate::config::Config::admin`] is configured, this delegates whole to
/// `admin::serve_with_admin` (issue #749) instead of the static
/// single-`Router` path below — see that function's docs for the runtime
/// admin API (separate listener, mandatory auth, add/remove/list routes +
/// reload). `POST /admin/reload` has no config file path to re-read under
/// this entry point (`config` may not have come from a file at all); use
/// [`serve_config_file_with_registry`] if reload support is needed.
pub async fn serve_with_registry(
    config: crate::config::Config,
    registry: SchemeRegistry,
) -> crate::Result<()> {
    serve_with_registry_impl(config, registry, None).await
}

async fn serve_with_registry_impl(
    config: crate::config::Config,
    registry: SchemeRegistry,
    config_path: Option<PathBuf>,
) -> crate::Result<()> {
    config.validate()?;

    if config.admin.is_some() {
        return admin::serve_with_admin(config, registry, config_path).await;
    }

    tracing::info!(
        bind = %config.bind,
        routes = config.routes.len(),
        "multimux origin starting"
    );

    let mut streams: HashMap<String, StreamRoute> = HashMap::new();
    let target_duration_secs = config.target_duration_secs;
    let part_target_ms = config.part_target_ms;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut supervisor_handles: Vec<(String, tokio::task::JoinHandle<()>)> = Vec::new();

    for route in &config.routes {
        let store = Arc::new(
            RouteHandle::new(target_duration_secs, part_target_ms, config.window_segments)
                .with_name(route.name.clone())
                .with_container(route_container(route))
                // Issue #900: without this, `route.dvr` was validated
                // (`Route::validate_dvr`) but never actually reached the
                // `RouteHandle` that `ProgramServing::new` builds a
                // `DvrRecorder` from — DVR recording configured via
                // `Config`/JSON silently never ran outside this crate's own
                // `with_dvr`-calling unit tests. `with_dvr` is a no-op when
                // `route.dvr.enabled` is `false` (the default), so wiring it
                // unconditionally is safe for every existing config.
                .with_dvr(route.dvr.clone()),
        );
        let outputs: Vec<Arc<dyn Output>> = route
            .outputs
            .iter()
            .filter(|k| !k.is_push() && !k.is_whep())
            .map(|k| build_output(k, &config.playlist_name, &registry))
            .collect::<crate::Result<Vec<_>>>()?;
        streams.insert(route.name.clone(), (store.clone(), outputs));

        let push_handles = spawn_push_outputs(route, Arc::clone(&store), &cancel);
        for h in push_handles {
            supervisor_handles.push((route.name.clone(), h));
        }
        let whep_handles = spawn_whep_outputs(route, Arc::clone(&store), &cancel);
        for h in whep_handles {
            supervisor_handles.push((route.name.clone(), h));
        }

        let name = route.name.clone();
        let shutdown_rx = shutdown_rx.clone();
        let handle = spawn_ingest(route, store, &config, &registry, shutdown_rx)?;
        supervisor_handles.push((name, handle));
    }

    let mut app_state = AppState::new(streams).with_limits(HttpLimits::from(&config));
    if let Some(output_auth) = &config.output_auth {
        let verifier = resolve_verifier(output_auth, OUTPUT_AUTH_REALM, &registry)?;
        app_state = app_state.with_output_auth(Arc::new(verifier));
    }
    let state = Arc::new(app_state);
    let listener = tokio::net::TcpListener::bind(config.bind.as_str()).await?;
    let shutdown_future = async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        cancel.cancel();
        // Best-effort: only fails if every receiver (every supervisor task)
        // has already exited, which just means there's nothing left to
        // notify.
        let _ = shutdown_tx.send(true);
    };
    // `into_make_service_with_connect_info` inserts a
    // `ConnectInfo<SocketAddr>` extension into every accepted request, which
    // `output_auth_gate` reads for `RequestContext::peer_addr` (issue #663
    // extensibility wave part 1) — without it, `peer_addr` would always be
    // `None`, same as it is in tests that `oneshot` the router directly.
    let serve_result = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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

/// Builds and spawns the ingest task for one configured route — pulled out of
/// [`serve_with_registry`]'s per-route loop (issue #805 task 2) so the
/// per-`InputSpec` wiring is callable, and individually testable, without
/// spinning up the whole HTTP server (see this module's own tests for the
/// `media_plane`-ported input kinds).
///
/// Every built-in variant drives [`supervisor::supervise_driver`] with the
/// matching `crate::source::*::run_*` entry point;
/// Spawn one [`crate::push::drive_push`] task per push output on `route`,
/// each awaiting the route's first `Trunk` (via
/// [`RouteHandle::await_first_trunk`]) before subscribing. Returns a
/// `JoinHandle` per push output; empty vec when `route` has no push outputs.
fn spawn_push_outputs(
    route: &crate::config::Route,
    store: Arc<RouteHandle>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    for kind in &route.outputs {
        match kind {
            crate::output::OutputKind::SrtPush {
                url,
                format,
                reconnect,
            } => {
                let url = url.clone();
                let format = format.unwrap_or(crate::config::PushFormat::Ts);
                let reconnect = reconnect.clone().unwrap_or_default();
                let store = Arc::clone(&store);
                let cancel = cancel.clone();
                handles.push(tokio::spawn(async move {
                    let trunk = store.await_first_trunk().await;
                    tracing::info!(%url, "SRT push output starting");
                    let config = crate::push::SrtTransportConfig::default();
                    crate::push::drive_push::<crate::push::SrtTransport>(
                        trunk, url, config, format, reconnect, cancel,
                    )
                    .await;
                }));
            }
            crate::output::OutputKind::RtmpPush {
                url,
                format,
                reconnect,
            } => {
                let url = url.clone();
                let _format = format.unwrap_or(crate::config::PushFormat::Ts);
                let reconnect = reconnect.clone().unwrap_or_default();
                let store = Arc::clone(&store);
                let cancel = cancel.clone();
                handles.push(tokio::spawn(async move {
                    let trunk = store.await_first_trunk().await;
                    tracing::info!(%url, "RTMP push output starting");
                    let (app, stream_key) = rtmp_app_and_stream_key(&url);
                    let config = crate::push::RtmpTransportConfig { app, stream_key };
                    crate::push::drive_push::<crate::push::RtmpTransport>(
                        trunk, url, config, _format, reconnect, cancel,
                    )
                    .await;
                }));
            }
            crate::output::OutputKind::RtspPush {
                url,
                format,
                reconnect,
            } => {
                let url = url.clone();
                let format = format.unwrap_or(crate::config::PushFormat::Ts);
                let reconnect = reconnect.clone().unwrap_or_default();
                let store = Arc::clone(&store);
                let cancel = cancel.clone();
                handles.push(tokio::spawn(async move {
                    let trunk = store.await_first_trunk().await;
                    tracing::info!(%url, "RTSP push output starting");
                    let config = crate::push::RtspTransportConfig::default();
                    crate::push::drive_push::<crate::push::RtspTransport>(
                        trunk, url, config, format, reconnect, cancel,
                    )
                    .await;
                }));
            }
            _ => {}
        }
    }
    handles
}

/// Spawn one `crate::output::whep::run_whep` task per `OutputKind::Whep`
/// output on `route` (issue #743) — the egress-side mirror of
/// [`spawn_ingest`]'s WHIP handling: unlike [`spawn_push_outputs`]'s
/// dial-out-to-a-fixed-URL outputs, WHEP *accepts* inbound viewer
/// connections, so it gets its own listen socket rather than a
/// `crate::push::drive_push` task. Each awaits the route's first `Trunk`
/// (via [`RouteHandle::await_first_trunk`]) before serving, exactly like
/// [`spawn_push_outputs`]. Returns a `JoinHandle` per `whep` output; empty
/// vec when `route` has none — including unconditionally whenever the
/// `whep` Cargo feature is off, in which case `OutputKind::Whep` does not
/// exist and this function's body is a no-op by construction (not just an
/// empty loop) — see the two `#[cfg]` bodies below.
#[cfg(feature = "whep")]
fn spawn_whep_outputs(
    route: &crate::config::Route,
    store: Arc<RouteHandle>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    for kind in &route.outputs {
        if let crate::output::OutputKind::Whep { listen } = kind {
            let route_cfg = crate::output::whep::WhepRoute::new(listen.clone());
            let store = Arc::clone(&store);
            let cancel = cancel.clone();
            handles.push(tokio::spawn(async move {
                let trunk = store.await_first_trunk().await;
                tracing::info!(listen = %route_cfg.listen(), "WHEP egress starting");
                crate::output::whep::run_whep(&route_cfg, trunk, cancel).await;
            }));
        }
    }
    handles
}

/// The `whep`-feature-off stub: see the feature-gated overload's own doc.
#[cfg(not(feature = "whep"))]
fn spawn_whep_outputs(
    route: &crate::config::Route,
    store: Arc<RouteHandle>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Vec<tokio::task::JoinHandle<()>> {
    let _ = (route, store, cancel);
    Vec::new()
}

/// Split an `rtmp_push` URL's path into `(app, stream_key)` (issue #934). An
/// RTMP URL is `rtmp://host[:port]/app/streamkey` — Adobe's convention also
/// allows a multi-segment app (`.../app/instance/streamkey`), so the **last**
/// path segment is always the stream key and everything before it (joined
/// back with `/`) is the app:
///
/// - 2+ segments (`/app/streamkey`, `/app/instance/streamkey`, …): app = all
///   but the last segment, stream_key = the last segment.
/// - exactly 1 segment (`/app`): no stream key was given — app = that
///   segment, stream_key = "" (previously always the case, even for
///   `/app/streamkey` URLs — the bug this fixes).
/// - no path / unparseable URL: app = `"live"` (the pre-existing default),
///   stream_key = "".
fn rtmp_app_and_stream_key(url: &str) -> (String, String) {
    let path = url::Url::parse(url)
        .ok()
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    let segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    match segments.len() {
        0 => ("live".to_string(), String::new()),
        1 => (segments[0].to_string(), String::new()),
        _ => {
            let (last, rest) = segments.split_last().expect("len >= 2 checked above");
            (rest.join("/"), last.to_string())
        }
    }
}

/// [`crate::config::InputSpec::Custom`] resolves its `type_tag` through
/// `registry` and calls the registered [`crate::registry::InputFactory`] instead,
/// which builds and spawns the equivalent `supervise_driver`-wrapped loop
/// itself — see [`serve_with_registry`]'s own doc for the full picture.
fn spawn_ingest(
    route: &crate::config::Route,
    store: Arc<RouteHandle>,
    config: &crate::config::Config,
    registry: &SchemeRegistry,
    shutdown_rx: watch::Receiver<bool>,
) -> crate::Result<tokio::task::JoinHandle<()>> {
    let name = route.name.clone();
    let target_duration_secs = config.target_duration_secs;
    let part_target_ms = config.part_target_ms;
    let window_segments = config.window_segments;
    let timeouts = crate::source::IngestTimeouts::from(config);

    Ok(match &route.input {
        crate::config::InputSpec::Rtsp { url, auth } => {
            let route_cfg = Arc::new(
                crate::source::rtsp::RtspRoute::new(name.clone(), url.clone())
                    .with_timeouts(timeouts)
                    .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials)),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        Err(crate::source::rtsp::run_rtsp(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await)
                    }
                },
                store,
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
            let route_cfg = Arc::new(
                crate::source::rtp_udp::RtpUdpRoute::new(
                    name.clone(),
                    addr.clone(),
                    sdp.clone(),
                    multicast_group.clone(),
                )
                .with_timeouts(timeouts),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        Err(crate::source::rtp_udp::run_rtp_udp(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await)
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::TsUdp {
            addr,
            multicast_group,
        } => {
            let route_cfg = Arc::new(
                crate::source::ts_udp::TsUdpRoute::new(
                    name.clone(),
                    addr.clone(),
                    multicast_group.clone(),
                )
                .with_timeouts(timeouts),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        Err(crate::source::ts_udp::run_ts_udp(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await)
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::TsHttp { url, auth } => {
            let route_cfg = Arc::new(
                crate::source::ts_http::TsHttpRoute::new(name.clone(), url.clone())
                    .with_timeouts(timeouts)
                    .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials)),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        crate::source::ts_http::run_ts_http(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::Srt {
            listen,
            remote,
            stream_id,
            latency_ms,
        } => {
            // `config.validate()` (called at the top of `serve_with_registry`)
            // already enforced exactly one of `listen`/`remote` is `Some`.
            let is_listener = listen.is_some();
            let mut srt_route = match (listen, remote) {
                (Some(l), None) => {
                    crate::source::srt::SrtRoute::new_listener(name.clone(), l.clone())
                }
                (None, Some(r)) => {
                    crate::source::srt::SrtRoute::new_caller(name.clone(), r.clone())
                }
                _ => unreachable!("config.validate() enforces exactly one of Srt listen/remote"),
            };
            srt_route = srt_route
                .with_stream_id(stream_id.clone())
                .with_latency_ms(*latency_ms)
                .with_timeouts(timeouts);
            let route_cfg = Arc::new(srt_route);
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        if is_listener {
                            crate::source::srt::run_srt_listener_once(
                                &route_cfg,
                                trunk_config,
                                handshake,
                                &route_handle,
                            )
                            .await
                        } else {
                            crate::source::srt::run_srt_caller(
                                &route_cfg,
                                trunk_config,
                                handshake,
                                &route_handle,
                            )
                            .await
                        }
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::HlsPull { url, auth } => {
            let route_cfg = Arc::new(
                crate::source::hls_pull::HlsPullRoute::new(name.clone(), url.clone())
                    .with_timeouts(timeouts)
                    .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials)),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        crate::source::hls_pull::run_hls_pull(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::DashPull { url, auth } => {
            let route_cfg = Arc::new(
                crate::source::dash_pull::DashPullRoute::new(name.clone(), url.clone())
                    .with_timeouts(timeouts)
                    .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials)),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        crate::source::dash_pull::run_dash_pull(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::SmoothPull { url, auth } => {
            let route_cfg = Arc::new(
                crate::source::smooth_pull::SmoothPullRoute::new(name.clone(), url.clone())
                    .with_timeouts(timeouts)
                    .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials)),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        crate::source::smooth_pull::run_smooth_pull(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::Rtmp {
            listen,
            app,
            stream_key,
        } => {
            let route_cfg = Arc::new(
                crate::source::rtmp::RtmpRoute::new(name.clone(), listen.clone())
                    .with_app(app.clone())
                    .with_stream_key(stream_key.clone())
                    .with_timeouts(timeouts),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        Err(crate::source::rtmp::run_rtmp(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await)
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        #[cfg(feature = "whip")]
        crate::config::InputSpec::Whip { listen } => {
            let route_cfg = Arc::new(
                crate::source::whip::WhipRoute::new(name.clone(), listen.clone())
                    .with_timeouts(timeouts),
            );
            tokio::spawn(supervisor::supervise_driver(
                move |route_handle| {
                    let route_cfg = Arc::clone(&route_cfg);
                    async move {
                        let trunk_config = crate::source::driver_trunk_config(window_segments);
                        let handshake = crate::source::handshake_policy(timeouts.connect);
                        Err(crate::source::whip::run_whip(
                            &route_cfg,
                            trunk_config,
                            handshake,
                            &route_handle,
                        )
                        .await)
                    }
                },
                store,
                Backoff::production_default(),
                name.clone(),
                shutdown_rx,
            ))
        }
        crate::config::InputSpec::Custom { type_tag, params } => {
            let factory =
                registry
                    .input(type_tag)
                    .ok_or_else(|| crate::MultimuxError::UnknownScheme {
                        kind: "input",
                        tag: type_tag.clone(),
                    })?;
            factory(InputCtx {
                name: name.clone(),
                params: params.clone(),
                store,
                target_duration_secs,
                part_target_ms,
                shutdown_rx,
            })?
        }
    })
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
    use crate::dvr::DvrConfig;
    use crate::output::llhls::LlHlsOutput;
    use crate::route::RouteHandle;
    use tower::ServiceExt;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        // Issue #805 tasks 3/6: the migrated egress call sites (`dynamic_file`
        // et al.) now resolve through the registry, not a `RouteHandle`-owned
        // `Trunk` directly -- publish this bare-constructed test route's own
        // program first, exactly as a real driver-backed route's
        // `crate::source::report_driver_progress` call would for its own
        // driver-minted Trunk once live, so there is a `ProgramServing`
        // bundle for `set_init` to write into at all.
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
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

    /// Biting test 3b: `GET /readyz` must 200 once a route's `RouteHandle` is
    /// `Live` — the counterpart to 3a, proving `/readyz` actually reads
    /// `HealthState` rather than being hardcoded to one status.
    #[tokio::test]
    async fn readyz_200_when_a_route_is_live() {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store.set_health(HealthState::Live);
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
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
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert(
            "metrics-probe".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
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

    // --- issue #663 P4: DASH alongside LL-HLS, from the shared store ---

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    /// Minimal well-formedness check: every opening tag has a matching
    /// closing tag in LIFO order, with no tag left open at the end. There is
    /// no XML-parsing crate anywhere in this workspace (checked before
    /// writing this), so this is a hand-rolled substitute for "parse it,
    /// don't just check non-empty" — genuinely biting (a mismatched/
    /// unclosed tag panics), without pulling in a new dependency for one
    /// test. Skips `<?...?>`/`<!...>` declarations (no matching close
    /// required).
    fn assert_well_formed_xml(xml: &str) {
        let mut stack: Vec<String> = Vec::new();
        let mut rest = xml;
        while let Some(start) = rest.find('<') {
            let end = rest[start..]
                .find('>')
                .unwrap_or_else(|| panic!("unterminated tag starting at {:?}", &rest[start..]))
                + start;
            let tag = &rest[start + 1..end];
            rest = &rest[end + 1..];
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            if let Some(name) = tag.strip_prefix('/') {
                let name = name.trim();
                let opened = stack
                    .pop()
                    .unwrap_or_else(|| panic!("closing tag </{name}> with nothing open"));
                assert_eq!(
                    opened, name,
                    "mismatched closing tag: opened <{opened}>, closed </{name}>"
                );
                continue;
            }
            let self_closing = tag.trim_end().ends_with('/');
            let name = tag
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            if !self_closing {
                stack.push(name);
            }
        }
        assert!(stack.is_empty(), "unclosed tags remain: {stack:?}");
    }

    /// The headline P4 test: one stream configured with **both** outputs
    /// serves LL-HLS's `media.m3u8` AND DASH's `manifest.mpd`, and the MPD's
    /// `SegmentTemplate` (once its `$RepresentationID$`/`$Number$` tokens are
    /// substituted exactly like a real DASH client would) names the *same*
    /// `seg-*.m4s` file the LL-HLS playlist already references — proving
    /// ingest-once/many-outputs from one shared `RouteHandle`, not a
    /// per-output re-mux.
    #[tokio::test]
    async fn both_outputs_serve_from_shared_segments_and_mpd_resolves() {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![transmux::TrackSpec::new(
                9,
                90_000,
                transmux::CodecConfig::Vp8 {
                    width: 640,
                    height: 480,
                },
            )],
        );
        store.add_segment(
            crate::route::SPTS_PROGRAM_ID,
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x33; 16],
                duration: 4.0,
                segment_seq: 1,
                part_count: 1,
            },
        );

        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![
                    Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
                    Arc::new(crate::output::dash::DashOutput) as Arc<dyn Output>,
                ],
            ),
        );
        let app = router(Arc::new(AppState::new(streams)));

        // LL-HLS media playlist.
        let resp = app.clone().oneshot(get("/cam1/media.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let hls_body = body_string(resp).await;
        assert!(hls_body.contains("#EXTM3U"));
        assert!(hls_body.contains("seg-1-1.m4s"), "hls body: {hls_body}");

        // DASH manifest: well-formed XML, carrying the required DASH
        // elements (not just a non-empty body).
        let resp = app
            .clone()
            .oneshot(get("/cam1/manifest.mpd"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "application/dash+xml"
        );
        let mpd_body = body_string(resp).await;
        assert_well_formed_xml(&mpd_body);
        assert!(mpd_body.contains("<MPD"), "{mpd_body}");
        assert!(
            mpd_body.contains(r#"xmlns="urn:mpeg:dash:schema:mpd:2011""#),
            "{mpd_body}"
        );
        assert!(mpd_body.contains(r#"type="dynamic""#), "{mpd_body}");
        assert!(mpd_body.contains("<Period"), "{mpd_body}");
        assert!(mpd_body.contains("<AdaptationSet"), "{mpd_body}");
        assert!(mpd_body.contains("<Representation"), "{mpd_body}");
        assert!(mpd_body.contains("<SegmentTemplate"), "{mpd_body}");
        assert!(mpd_body.contains(r#"startNumber="1""#), "{mpd_body}");
        assert!(
            mpd_body.contains("seg-$RepresentationID$-$Number$.m4s"),
            "{mpd_body}"
        );

        // Substitute the MPD's template tokens exactly like a real DASH
        // client would ($RepresentationID$ -> the Representation's own @id,
        // 1; $Number$ -> startNumber, 1 for the first/only segment) and
        // confirm the resolved filename is the SAME one the LL-HLS playlist
        // above referenced, AND that the shared resource route actually
        // serves it.
        let resolved_uri = "seg-1-1.m4s";
        assert!(
            hls_body.contains(resolved_uri),
            "LL-HLS playlist must reference the same resolved filename: {hls_body}"
        );
        let resp = app
            .oneshot(get(&format!("/cam1/{resolved_uri}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0x33; 16]);
    }

    /// A DASH-only route (no LL-HLS output configured) never mounts the
    /// `master.m3u8`/`media.m3u8` routes — proves `manifest_routes` is
    /// genuinely per-output, not a hardcoded LL-HLS+DASH pair.
    #[tokio::test]
    async fn dash_only_route_has_no_llhls_routes() {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![transmux::TrackSpec::new(
                1,
                90_000,
                transmux::CodecConfig::Vp8 {
                    width: 640,
                    height: 480,
                },
            )],
        );
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(crate::output::dash::DashOutput) as Arc<dyn Output>],
            ),
        );
        let app = router(Arc::new(AppState::new(streams)));

        let resp = app
            .clone()
            .oneshot(get("/cam1/manifest.mpd"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// Issue #663 P4.2 / #721: a route configured with all three outputs
    /// (`ll_hls`+`dash`+`ll_dash`) serves the LL-DASH `manifest-ll.mpd`
    /// alongside the regular `dash`/`ll_hls` manifests unchanged (the
    /// regression this story must not break), and the LL-DASH manifest's
    /// `SegmentTemplate` — a whole-segment `$Number$` template, exactly like
    /// `manifest.mpd`'s — resolves against the shared resource route via the
    /// **chunked-transfer** path while the segment is still in progress: the
    /// response body streams the segment's parts as they land and completes
    /// once the segment closes.
    #[tokio::test]
    async fn ll_dash_output_signals_and_resolves_alongside_dash_and_llhls() {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![transmux::TrackSpec::new(
                9,
                90_000,
                transmux::CodecConfig::Vp8 {
                    width: 640,
                    height: 480,
                },
            )],
        );
        store.add_segment(
            crate::route::SPTS_PROGRAM_ID,
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x33; 16],
                duration: 4.0,
                segment_seq: 1,
                part_count: 2,
            },
        );
        // Live parts of the in-progress segment (seq 2) -- not yet closed.
        store.add_part(
            crate::route::SPTS_PROGRAM_ID,
            transmux::ll_hls::PartInfo {
                bytes: vec![0x50; 4],
                duration: 0.5,
                independent: true,
                segment_seq: 2,
                part_index: 0,
            },
        );
        store.add_part(
            crate::route::SPTS_PROGRAM_ID,
            transmux::ll_hls::PartInfo {
                bytes: vec![0x51; 4],
                duration: 0.5,
                independent: false,
                segment_seq: 2,
                part_index: 1,
            },
        );

        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store.clone(),
                vec![
                    Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
                    Arc::new(crate::output::dash::DashOutput) as Arc<dyn Output>,
                    Arc::new(crate::output::ll_dash::LlDashOutput) as Arc<dyn Output>,
                ],
            ),
        );
        let app = router(Arc::new(AppState::new(streams)));

        // --- Regression: standard DASH + LL-HLS unaffected. ---
        let resp = app
            .clone()
            .oneshot(get("/cam1/manifest.mpd"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let dash_body = body_string(resp).await;
        assert_well_formed_xml(&dash_body);
        assert!(dash_body.contains("seg-$RepresentationID$-$Number$.m4s"));

        let resp = app.clone().oneshot(get("/cam1/media.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let hls_body = body_string(resp).await;
        assert!(hls_body.contains("#EXTM3U"));

        // --- The new LL-DASH manifest: true chunked-transfer design. ---
        let resp = app
            .clone()
            .oneshot(get("/cam1/manifest-ll.mpd"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "application/dash+xml"
        );
        let ll_body = body_string(resp).await;
        assert_well_formed_xml(&ll_body);
        assert!(ll_body.contains("<MPD"), "{ll_body}");
        assert!(ll_body.contains(r#"type="dynamic""#), "{ll_body}");
        // True chunked design: availabilityTimeOffset is a real, non-zero
        // segment-minus-chunk figure (4.0 - 0.5 = 3.5), not the old
        // parts-signalling design's honest "0" -- see `output::ll_dash`'s
        // module docs.
        assert!(
            ll_body.contains("availabilityTimeOffset=\"3.5\""),
            "{ll_body}"
        );
        assert!(
            ll_body.contains("availabilityTimeComplete=\"false\""),
            "{ll_body}"
        );
        assert!(ll_body.contains("<ServiceDescription"), "{ll_body}");
        assert!(ll_body.contains("<Latency target="), "{ll_body}");
        assert!(
            ll_body.contains("seg-$RepresentationID$-$Number$.m4s"),
            "LL-DASH addresses whole segments, exactly like manifest.mpd \
             (parts are an internal chunked-transfer delivery detail, never \
             addressed by the MPD itself): {ll_body}"
        );
        assert!(
            !ll_body.contains("part-"),
            "no part-addressed URI in the MPD: {ll_body}"
        );

        // --- Fetch the in-progress segment (seq 2): must resolve via the
        // chunked-transfer path (no Content-Length -- the body streams),
        // completing once the segment closes with the concatenated part
        // bytes. Close the segment concurrently so the request completes
        // promptly instead of waiting out the blocking-reload timeout on a
        // part index that will never come.
        let store_for_close = store.clone();
        let closer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_close.add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x99; 8], // distinct from the concatenated parts
                    duration: 1.0,
                    segment_seq: 2,
                    part_count: 2,
                },
            );
        });
        let resp = app.oneshot(get("/cam1/seg-1-2.m4s")).await.unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::OK,
            "in-progress whole-segment request must stream, not 404"
        );
        let bytes = body_bytes(resp).await;
        assert_eq!(
            bytes,
            [vec![0x50; 4], vec![0x51; 4]].concat(),
            "streamed body must be the segment's parts concatenated in order"
        );
        closer.await.unwrap();
    }

    /// The shared response-header middleware treats `.mpd` the same as
    /// `.m3u8` (`no-cache`) and everything else as immutable — proving the
    /// generalisation from `output::llhls`'s old per-output middleware
    /// (which only ever checked `.m3u8`) actually covers DASH too.
    #[tokio::test]
    async fn manifest_and_resource_responses_carry_expected_cache_control_and_cors() {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![transmux::TrackSpec::new(
                1,
                90_000,
                transmux::CodecConfig::Vp8 {
                    width: 640,
                    height: 480,
                },
            )],
        );
        store.add_segment(
            crate::route::SPTS_PROGRAM_ID,
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x33; 16],
                duration: 4.0,
                segment_seq: 1,
                part_count: 1,
            },
        );
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![
                    Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
                    Arc::new(crate::output::dash::DashOutput) as Arc<dyn Output>,
                ],
            ),
        );
        let app = router(Arc::new(AppState::new(streams)));

        for (uri, expected_cache) in [
            ("/cam1/media.m3u8", CACHE_CONTROL_MANIFEST),
            ("/cam1/manifest.mpd", CACHE_CONTROL_MANIFEST),
            ("/cam1/seg-1-1.m4s", CACHE_CONTROL_IMMUTABLE),
        ] {
            let resp = app.clone().oneshot(get(uri)).await.unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::OK, "{uri}");
            assert_eq!(
                resp.headers()
                    .get(axum::http::header::CACHE_CONTROL)
                    .unwrap(),
                expected_cache,
                "{uri}"
            );
            assert_eq!(
                resp.headers()
                    .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "*",
                "{uri}"
            );
        }
    }

    // --- issue #663 P5: HTTP-layer resource limits (audit-concurrency #3) ---

    fn post_with_body(uri: &str, body: Vec<u8>) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header(axum::http::header::CONTENT_LENGTH, body.len().to_string())
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    /// Biting test 1: a request whose `Content-Length` exceeds
    /// [`HttpLimits::max_request_body_bytes`] must be rejected `413 Payload
    /// Too Large` — proving [`RequestBodyLimitLayer`] is actually wired into
    /// [`router`], not just configured and ignored. `tower_http`'s layer
    /// checks `Content-Length` synchronously (RFC 9110 §8.6), so this never
    /// even reaches a handler.
    #[tokio::test]
    async fn oversized_request_body_is_rejected_413() {
        const TINY_LIMIT: usize = 8;
        let app = router(Arc::new(AppState::new(make_state_streams()).with_limits(
            HttpLimits {
                max_request_body_bytes: TINY_LIMIT,
                ..HttpLimits::default()
            },
        )));

        let resp = app
            .oneshot(post_with_body(
                "/cam1/master.m3u8",
                vec![0u8; TINY_LIMIT + 1],
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// A normal, well-formed request must still succeed once the limit
    /// layers are wired in — proving they don't break the ordinary path
    /// (a body well within the cap, one request, well within the timeout).
    #[tokio::test]
    async fn normal_request_still_succeeds_with_limits_applied() {
        let app = router(Arc::new(AppState::new(make_state_streams())));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// A request body within the cap must still succeed (not just "under the
    /// cap is untested") — the counterpart to
    /// `oversized_request_body_is_rejected_413`.
    #[tokio::test]
    async fn request_body_within_limit_still_succeeds() {
        const TINY_LIMIT: usize = 64;
        let app = router(Arc::new(AppState::new(make_state_streams()).with_limits(
            HttpLimits {
                max_request_body_bytes: TINY_LIMIT,
                ..HttpLimits::default()
            },
        )));
        let resp = app
            .oneshot(post_with_body("/cam1/master.m3u8", vec![0u8; TINY_LIMIT]))
            .await
            .unwrap();
        // POST isn't a route axum has registered for master.m3u8 (only GET/
        // OPTIONS), so this 404s — the point is it must NOT 413, proving the
        // body-size check passed and control reached routing.
        assert_ne!(resp.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// Biting test 2: [`TimeoutLayer`] must actually cut off a slow request —
    /// a legitimate, in-abuse-bound blocking `_HLS_msn` reload that never
    /// resolves (nothing ever closes the awaited segment) would otherwise sit
    /// out the LL-HLS engine's own 5 s `BLOCKING_RELOAD_TIMEOUT` before
    /// falling back to a `200`. A configured `request_timeout` far shorter
    /// than that must return `408 Request Timeout` well before 5 s elapses,
    /// proving the global timeout layer is wired into [`router`] and set
    /// *above* (not blind to) the LL-HLS blocking cap by default, but does
    /// still bind when configured tighter.
    #[tokio::test]
    async fn global_timeout_layer_cuts_off_a_slow_blocking_request() {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store.add_segment(
            crate::route::SPTS_PROGRAM_ID,
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x20; 8],
                duration: 4.0,
                segment_seq: 1,
                part_count: 1,
            },
        );
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
        );
        let app = router(Arc::new(AppState::new(streams).with_limits(HttpLimits {
            request_timeout: std::time::Duration::from_millis(50),
            ..HttpLimits::default()
        })));

        let started = std::time::Instant::now();
        // msn=2 is within ABUSE_MSN_FUTURE_BOUND of the current max (1), so
        // this is a genuine WouldBlock — not the fast-400 abuse-rejection
        // path — that nothing in this test ever satisfies.
        let resp = app
            .oneshot(get("/cam1/media.m3u8?_HLS_msn=2&_HLS_part=0"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::REQUEST_TIMEOUT);
        // NOT a pure hang guard (issue #807): this must stay meaningfully
        // below the 5s internal LL-HLS blocking-reload cap it is
        // distinguishing from, or a broken `TimeoutLayer` (one that let the
        // request fall through to the 5s internal cap instead of cutting it
        // off at the configured 50ms) would still pass by coincidence.
        // Raised from 1s to 3s for scheduling headroom under load while
        // staying clearly below the 5s cap it must distinguish from.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "must be cut off by the 50ms configured timeout, not the 5s \
             internal LL-HLS blocking-reload cap: {:?}",
            started.elapsed()
        );
    }

    /// Helper: a single populated `cam1` stream (mirrors [`make_state`]'s
    /// store, but returning the raw map so tests can attach their own
    /// [`HttpLimits`] via [`AppState::with_limits`], which [`make_state`]
    /// itself doesn't expose).
    fn make_state_streams() -> HashMap<String, StreamRoute> {
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
        );
        streams
    }

    // --- issue #663 "shared output auth" ---

    use broadcast_auth::{Credentials, RequestContext, respond};

    fn get_with_auth(uri: &str, authorization: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .header(axum::http::header::AUTHORIZATION, authorization)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn get_with_header(
        uri: &str,
        name: &str,
        value: &str,
    ) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .header(name, value)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn basic_header(username: &str, password: &str) -> String {
        use base64::Engine as _;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
        )
    }

    /// A `cam1` app gated by `verifier` — every other stream/root behaviour
    /// unchanged from [`make_state_streams`].
    fn app_with_output_auth(verifier: Verifier) -> Router {
        router(Arc::new(
            AppState::new(make_state_streams()).with_output_auth(Arc::new(verifier)),
        ))
    }

    /// Biting test: with Basic `output_auth` configured, a request with no
    /// `Authorization` header must `401` and carry a `WWW-Authenticate:
    /// Basic realm=...` challenge.
    #[tokio::test]
    async fn output_auth_basic_missing_creds_401_with_challenge() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        ));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        let challenge = resp
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .expect("401 must carry WWW-Authenticate")
            .to_str()
            .unwrap();
        assert!(challenge.starts_with("Basic realm="), "{challenge}");
    }

    /// Correct Basic credentials must `200`.
    #[tokio::test]
    async fn output_auth_basic_correct_creds_200() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        ));
        let resp = app
            .oneshot(get_with_auth(
                "/cam1/master.m3u8",
                &basic_header("admin", "hunter2"),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Wrong Basic credentials must `401`.
    #[tokio::test]
    async fn output_auth_basic_wrong_creds_401() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        ));
        let resp = app
            .oneshot(get_with_auth(
                "/cam1/master.m3u8",
                &basic_header("admin", "WRONG"),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Biting test: with Digest `output_auth` configured, a request with no
    /// `Authorization` header must `401` and carry a `WWW-Authenticate:
    /// Digest realm=..., nonce=..., qop="auth", algorithm=MD5` challenge.
    #[tokio::test]
    async fn output_auth_digest_missing_creds_401_with_challenge() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        ));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        let challenge = resp
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .expect("401 must carry WWW-Authenticate")
            .to_str()
            .unwrap();
        assert!(challenge.starts_with("Digest "), "{challenge}");
        assert!(challenge.contains("nonce="), "{challenge}");
        assert!(challenge.contains("qop=\"auth\""), "{challenge}");
    }

    /// Correct Digest credentials, computed by a real `broadcast_auth`
    /// client answering the server's own challenge (round trip through the
    /// real production `Verifier`, not a hand-rolled header), must `200`.
    #[tokio::test]
    async fn output_auth_digest_correct_creds_200() {
        let verifier = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        );
        let challenge = verifier.challenge();
        let app = router(Arc::new(
            AppState::new(make_state_streams()).with_output_auth(Arc::new(verifier)),
        ));
        let authorization = respond(
            &challenge,
            &RequestContext::new("GET", "/cam1/master.m3u8"),
            Credentials::new("admin", "hunter2"),
        )
        .unwrap();
        let resp = app
            .oneshot(get_with_auth("/cam1/master.m3u8", &authorization))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Wrong Digest credentials must `401`.
    #[tokio::test]
    async fn output_auth_digest_wrong_creds_401() {
        let verifier = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        );
        let challenge = verifier.challenge();
        let app = router(Arc::new(
            AppState::new(make_state_streams()).with_output_auth(Arc::new(verifier)),
        ));
        let authorization = respond(
            &challenge,
            &RequestContext::new("GET", "/cam1/master.m3u8"),
            Credentials::new("admin", "WRONG"),
        )
        .unwrap();
        let resp = app
            .oneshot(get_with_auth("/cam1/master.m3u8", &authorization))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Biting test: with Bearer `output_auth` configured, a request with no
    /// `Authorization` header must `401`.
    #[tokio::test]
    async fn output_auth_bearer_missing_creds_401() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Correct Bearer token must `200`.
    #[tokio::test]
    async fn output_auth_bearer_correct_token_200() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let resp = app
            .oneshot(get_with_auth("/cam1/master.m3u8", "Bearer secrettoken"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Wrong Bearer token must `401`.
    #[tokio::test]
    async fn output_auth_bearer_wrong_token_401() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let resp = app
            .oneshot(get_with_auth("/cam1/master.m3u8", "Bearer WRONG"))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Biting test: `/healthz` (an ops endpoint) must stay `200` with **no**
    /// credentials even when `output_auth` is configured — load-balancer
    /// probes/scraping must never be gated. Reverting the "apply
    /// `output_auth_gate` only to the per-stream nests, not the merged root"
    /// wiring makes this fail (this same test would then also need
    /// credentials).
    #[tokio::test]
    async fn output_auth_configured_healthz_still_open() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let resp = app.oneshot(get("/healthz")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Biting test: `/metrics` must also stay open with `output_auth`
    /// configured.
    #[tokio::test]
    async fn output_auth_configured_metrics_still_open() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let resp = app.oneshot(get("/metrics")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// `output_auth: None` (the default, via [`make_state_streams`]/`AppState::new`
    /// with no `with_output_auth` call) leaves the stream route open, exactly
    /// as every pre-#663 test in this module already assumes.
    #[tokio::test]
    async fn output_auth_none_stream_route_stays_open() {
        let app = router(Arc::new(AppState::new(make_state_streams())));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    // --- issue #663 extensibility wave part 1: Forwarded output-auth ---

    /// Biting test: with `Forwarded` `output_auth` configured, a request
    /// carrying `X-Forwarded-User` (non-empty) must `200` — the whole
    /// mechanism is trusting a fronting reverse proxy to have set it.
    #[tokio::test]
    async fn output_auth_forwarded_with_user_header_200() {
        let app = app_with_output_auth(Verifier::forwarded(
            "X-Forwarded-User",
            Some("X-Forwarded-For".to_string()),
        ));
        let resp = app
            .oneshot(get_with_header(
                "/cam1/master.m3u8",
                "X-Forwarded-User",
                "alice",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Biting test: with `Forwarded` `output_auth` configured, a request with
    /// no `X-Forwarded-User` header must `401` — a client hitting the origin
    /// directly (bypassing the trusted proxy) must not get in.
    #[tokio::test]
    async fn output_auth_forwarded_without_user_header_401() {
        let app = app_with_output_auth(Verifier::forwarded(
            "X-Forwarded-User",
            Some("X-Forwarded-For".to_string()),
        ));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// An empty `X-Forwarded-User` header must not count as authenticated
    /// (a misbehaving proxy forwarding a blank header must not silently
    /// grant access).
    #[tokio::test]
    async fn output_auth_forwarded_empty_user_header_401() {
        let app = app_with_output_auth(Verifier::forwarded(
            "X-Forwarded-User",
            Some("X-Forwarded-For".to_string()),
        ));
        let resp = app
            .oneshot(get_with_header("/cam1/master.m3u8", "X-Forwarded-User", ""))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Biting test: `output_auth_gate` reads `X-Forwarded-For` via
    /// `Verifier::forwarded_for` — confirmed here by checking the verifier
    /// resolves it from a `RequestContext` built the same way the gate
    /// builds one (headers collected from the request), rather than only
    /// asserting on the HTTP status (which a `Forwarded` scheme would return
    /// `200` for either way, since `X-Forwarded-For` is never part of the
    /// trust decision).
    #[tokio::test]
    async fn output_auth_forwarded_reads_x_forwarded_for() {
        let verifier = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        let headers: &[(&str, &str)] = &[
            ("X-Forwarded-User", "alice"),
            ("X-Forwarded-For", "203.0.113.7"),
        ];
        let ctx = RequestContext::new("GET", "/cam1/master.m3u8").with_headers(headers);
        assert_eq!(verifier.forwarded_for(&ctx), Some("203.0.113.7"));

        // And the end-to-end request carrying both headers still `200`s —
        // `X-Forwarded-For` is read for observability, never gates access.
        let app = app_with_output_auth(verifier);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/cam1/master.m3u8")
                    .header("X-Forwarded-User", "alice")
                    .header("X-Forwarded-For", "203.0.113.7")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// Basic/Digest/Bearer output-auth are unaffected by the `Forwarded`
    /// addition and by `RequestContext` gaining `headers`/`peer_addr` —
    /// re-run here as an explicit "still pass" marker for the extensibility
    /// wave (the bulk of the Basic/Digest/Bearer coverage is the pre-existing
    /// `output_auth_basic_*`/`output_auth_digest_*`/`output_auth_bearer_*`
    /// tests above, all still green).
    #[tokio::test]
    async fn output_auth_basic_digest_bearer_unaffected_by_forwarded_addition() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        ));
        let resp = app
            .oneshot(get_with_auth(
                "/cam1/master.m3u8",
                &basic_header("admin", "hunter2"),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// The `401` response from `output_auth_gate` must still carry the same
    /// CORS header as a normal response — needed for a cross-origin browser
    /// client to see the `401`/`WWW-Authenticate` at all rather than an
    /// opaque failed-CORS network error.
    #[tokio::test]
    async fn output_auth_401_response_still_carries_cors_header() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let resp = app.oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*"
        );
    }

    // --- issue #663 external scheme plugin registry ---

    /// A route naming a `Custom` input's `type_tag` (`"nope"`) with nothing
    /// registered for it must fail route setup with
    /// `MultimuxError::UnknownScheme` — not panic, and not block. This
    /// resolves (or errors) before `serve_with_registry` ever binds the
    /// listener or enters axum's blocking accept loop, since the route-build
    /// loop runs first and returns via `?` on the first error, so this test
    /// can simply `.await` the whole call without a timeout wrapper.
    #[tokio::test]
    async fn serve_with_registry_unregistered_custom_input_tag_errors_not_panics() {
        let cfg = crate::config::Config {
            routes: vec![crate::config::Route {
                name: "cam1".into(),
                input: crate::config::InputSpec::Custom {
                    type_tag: "nope".into(),
                    params: serde_json::Value::Null,
                },
                outputs: vec![crate::output::OutputKind::LlHls],
                dvr: DvrConfig::default(),
            }],
            bind: "127.0.0.1:0".into(),
            ..crate::config::Config::default()
        };
        let err = serve_with_registry(cfg, SchemeRegistry::new())
            .await
            .expect_err("an unregistered custom input tag must error, not silently succeed");
        match err {
            crate::MultimuxError::UnknownScheme { kind, tag } => {
                assert_eq!(kind, "input");
                assert_eq!(tag, "nope");
            }
            other => panic!("expected MultimuxError::UnknownScheme, got {other:?}"),
        }
    }

    /// Same property for a `Custom` output.
    #[tokio::test]
    async fn serve_with_registry_unregistered_custom_output_tag_errors_not_panics() {
        let cfg = crate::config::Config {
            routes: vec![crate::config::Route {
                name: "cam1".into(),
                input: crate::config::InputSpec::Rtsp {
                    url: "rtsp://host/stream".into(),
                    auth: None,
                },
                outputs: vec![crate::output::OutputKind::Custom {
                    type_tag: "webrtc".into(),
                    params: serde_json::Value::Null,
                }],
                dvr: DvrConfig::default(),
            }],
            bind: "127.0.0.1:0".into(),
            ..crate::config::Config::default()
        };
        let err = serve_with_registry(cfg, SchemeRegistry::new())
            .await
            .expect_err("an unregistered custom output tag must error, not silently succeed");
        match err {
            crate::MultimuxError::UnknownScheme { kind, tag } => {
                assert_eq!(kind, "output");
                assert_eq!(tag, "webrtc");
            }
            other => panic!("expected MultimuxError::UnknownScheme, got {other:?}"),
        }
    }

    /// Same property for a `Custom` output-auth scheme.
    #[tokio::test]
    async fn serve_with_registry_unregistered_custom_auth_tag_errors_not_panics() {
        let cfg = crate::config::Config {
            routes: vec![crate::config::Route {
                name: "cam1".into(),
                input: crate::config::InputSpec::Rtsp {
                    url: "rtsp://host/stream".into(),
                    auth: None,
                },
                outputs: vec![crate::output::OutputKind::LlHls],
                dvr: DvrConfig::default(),
            }],
            bind: "127.0.0.1:0".into(),
            output_auth: Some(crate::config::OutputAuthSpec::Custom {
                type_tag: "hmac".into(),
                params: serde_json::Value::Null,
            }),
            ..crate::config::Config::default()
        };
        let err = serve_with_registry(cfg, SchemeRegistry::new())
            .await
            .expect_err("an unregistered custom auth tag must error, not silently succeed");
        match err {
            crate::MultimuxError::UnknownScheme { kind, tag } => {
                assert_eq!(kind, "auth");
                assert_eq!(tag, "hmac");
            }
            other => panic!("expected MultimuxError::UnknownScheme, got {other:?}"),
        }
    }

    /// A registered custom input factory, once looked up out of a
    /// `SchemeRegistry` and invoked with an `InputCtx` — exactly the shape
    /// `serve_with_registry`'s own `InputSpec::Custom` arm builds and passes
    /// — actually drives real state: the spawned task reads `ctx.params` and
    /// writes into `ctx.store`, proving the context multimux hands the
    /// factory is wired correctly end-to-end, not just that the factory is
    /// present in the map.
    #[tokio::test]
    async fn registered_custom_input_factory_runs_against_a_real_input_ctx() {
        let mut registry = SchemeRegistry::new();
        registry.register_input(
            "silence",
            Arc::new(|ctx: crate::registry::InputCtx| {
                assert_eq!(
                    ctx.params.get("marker").and_then(|v| v.as_str()),
                    Some("ok")
                );
                ctx.store.set_health(HealthState::Live);
                Ok(tokio::spawn(async move {
                    // Hold the shutdown receiver alive until told to stop,
                    // mirroring a real supervised connector task.
                    let mut rx = ctx.shutdown_rx;
                    let _ = rx.changed().await;
                }))
            }),
        );

        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let factory = registry.input("silence").expect("factory registered above");
        let handle = factory(crate::registry::InputCtx {
            name: "cam1".into(),
            params: serde_json::json!({"marker": "ok"}),
            store: store.clone(),
            target_duration_secs: 4.0,
            part_target_ms: 500,
            shutdown_rx,
        })
        .expect("factory must succeed");

        // HANG GUARD (issue #807): the factory closure sets `Live`
        // synchronously before ever yielding, so this normally resolves on
        // its first 1ms poll. Raised for load tolerance -- only job is to
        // fail "never becomes Live" rather than hang, not a timing claim.
        let became_live = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if store.health() == HealthState::Live {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .is_ok();
        assert!(became_live, "factory-spawned task must reach the store");

        handle.abort();
    }

    /// Issue #805 task 2's own bite: a route configured with
    /// [`crate::config::InputSpec::TsUdp`] (one of the eight
    /// `media_plane`-ported input kinds) must actually ingest through
    /// [`spawn_ingest`]'s real wiring and become resolvable through the
    /// registry — not just compile against the new signatures.
    ///
    /// MUTATION VERIFIED: reverting `spawn_ingest`'s `InputSpec::TsUdp` arm
    /// (and the other seven driver-backed arms) to the pre-#805 combined stub
    /// (`{ tokio::spawn(async move { tracing::error!(..); }) }`, matching
    /// what every one of those eight `InputSpec` variants used to do) makes
    /// this test fail: `assert!(resolved, ...)` fails because
    /// `store.resolve_program` never leaves `ProgramResolution::NotYetAnnounced`
    /// within the 5 s timeout (the stub never touches `store` at all) —
    /// exactly the dead-route regression issue #805 exists to fix. Rebuilt
    /// and re-run to confirm the failure, then reverted.
    #[tokio::test]
    async fn ts_udp_input_ingests_and_becomes_resolvable_through_the_registry() {
        let reserved = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let cfg = crate::config::Config {
            routes: vec![crate::config::Route {
                name: "cam1".into(),
                input: crate::config::InputSpec::TsUdp {
                    addr: addr.to_string(),
                    multicast_group: None,
                },
                outputs: vec![crate::output::OutputKind::LlHls],
                dvr: DvrConfig::default(),
            }],
            bind: "127.0.0.1:0".into(),
            ..crate::config::Config::default()
        };
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = spawn_ingest(
            &cfg.routes[0],
            store.clone(),
            &cfg,
            &SchemeRegistry::new(),
            shutdown_rx,
        )
        .expect("spawn_ingest must accept a TsUdp route");

        // Real muxed TS bytes (the same fixture builder `ts_udp`'s own
        // loopback test uses), sent repeatedly so the supervisor's own
        // (asynchronous) socket bind has ample time to land before the first
        // datagram that matters arrives.
        let ts_bytes = crate::source::ts_program::test_support::build_ts_bytes(1, 0xAB, 60);
        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind sender");
        let send_task = tokio::spawn(async move {
            loop {
                for chunk in ts_bytes.chunks(7 * 188) {
                    let _ = sender.send_to(chunk, addr).await;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        });

        // HANG GUARD (issue #807): real loopback UDP + real TS demux, so this
        // needs actual socket/scheduling time (unlike the in-process cases
        // elsewhere in this file), but still normally resolves in low tens
        // of ms given the sender retries every 5ms. Raised for load
        // tolerance -- only job is to fail "never ingests" rather than hang.
        let resolved = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if matches!(
                    store.resolve_program(crate::route::SPTS_PROGRAM_ID),
                    crate::route::ProgramResolution::Found(_)
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok();

        assert!(
            resolved,
            "a TsUdp route must actually ingest and publish its program into the registry"
        );

        send_task.abort();
        handle.abort();
    }

    /// A minimal, `sdp-types`-parseable single-media SDP (mirrors
    /// `multimux/tests/rtsp_ingest.rs`'s own `sdp_body`) — just enough for
    /// [`crate::config::InputSpec::Rtp`]'s `sdp` field to be genuinely valid,
    /// so that variant's dispatch reaches its real "bind the UDP socket and
    /// wait for datagrams" path rather than failing earlier on SDP parsing.
    fn minimal_rtp_sdp() -> String {
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         t=0 0\r\n\
         m=video 0 RTP/AVP 96\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=control:streamid=0\r\n"
            .to_string()
    }

    /// Compile-time exhaustiveness net (issue #805 task 3): a **separate**
    /// exhaustive match from `spawn_ingest`'s own — that one only proves
    /// *production* wiring keeps pace with a newly-added `InputSpec` variant
    /// (it would fail to compile on its own); this one proves this
    /// **regression test's enumeration** does too. `InputSpec` is
    /// `#[non_exhaustive]`, so an external crate's `match` would be forced to
    /// carry a `_ =>` arm that silently swallows a new variant — this only
    /// works as a real compile-time net because this test lives in-crate
    /// (`multimux/src/origin/mod.rs`), not in `multimux/tests/*.rs`. Every
    /// arm returns the same value; the only thing under test is the *shape*
    /// of the match itself.
    fn every_input_spec_variant_is_named_here(spec: &crate::config::InputSpec) -> bool {
        match spec {
            crate::config::InputSpec::Rtsp { .. } => true,
            crate::config::InputSpec::Rtp { .. } => true,
            crate::config::InputSpec::TsUdp { .. } => true,
            crate::config::InputSpec::TsHttp { .. } => true,
            crate::config::InputSpec::Srt { .. } => true,
            crate::config::InputSpec::HlsPull { .. } => true,
            crate::config::InputSpec::DashPull { .. } => true,
            crate::config::InputSpec::SmoothPull { .. } => true,
            crate::config::InputSpec::Rtmp { .. } => true,
            #[cfg(feature = "whip")]
            crate::config::InputSpec::Whip { .. } => true,
            crate::config::InputSpec::Custom { .. } => true,
        }
    }

    /// Issue #805 task 3's own regression net: **every** non-`Custom`
    /// [`crate::config::InputSpec`] variant — not just `TsUdp` (task 2's own
    /// bite, above) — must dispatch to a route that genuinely attempts
    /// ingest, not the pre-#805 combined stub arm that logged an error and
    /// spawned a no-op future for eight of nine variants while every gate
    /// (build/clippy/doc/5674 tests) stayed green.
    ///
    /// Every variant here is pointed at a deliberately dead/unreachable
    /// endpoint (a refused TCP connect, a UDP port nothing ever sends to, an
    /// SRT caller dialing nobody, or — for `Rtmp` — a listen port this test
    /// has already bound out from under it) so no real server is needed
    /// anywhere. A genuinely-wired route's supervisor (`supervise_driver`)
    /// completes one full attempt-and-fail cycle and transitions
    /// [`RouteHandle::health`] `Connecting` -> `Reconnecting`;
    /// the old combined stub arm never touched `route_handle` at all, so a
    /// regressed variant would still read `Connecting` at the hang guard.
    ///
    /// `Custom` is deliberately excluded from the dead-endpoint loop below —
    /// it resolves through [`SchemeRegistry`], not a built-in `run_*` entry
    /// point, and is already covered by
    /// `registered_custom_input_factory_runs_against_a_real_input_ctx` and
    /// the `serve_with_registry_unregistered_custom_*_errors_not_panics`
    /// tests above — but it is still named in
    /// [`every_input_spec_variant_is_named_here`]'s match, so the compile-time
    /// net covers the whole enum, not just the nine `run_*`-backed variants.
    ///
    /// MUTATION VERIFIED: replacing `spawn_ingest`'s `InputSpec::TsUdp` arm
    /// with the pre-#805 combined stub (`InputSpec::TsUdp { .. } =>
    /// tokio::spawn(async move { tracing::error!("ingest kind not yet
    /// wired"); })`) makes this test fail on the `TsUdp` iteration
    /// specifically: the 10 s hang-guard `reached_reconnecting` poll loop
    /// times out (`is_ok()` is `false`, since the stub never calls
    /// `route_handle.set_health` at all) and the very next
    /// `assert!(reached_reconnecting, ...)` — not the later `assert_eq!`,
    /// which the panic short-circuits before ever reaching — fires with:
    /// `variant TsUdp { addr: "127.0.0.1:58600", multicast_group: None }
    /// never left HealthState::Connecting within the hang guard (still
    /// Connecting) -- looks like a stubbed dispatch arm`. Rebuilt and re-ran
    /// to confirm this exact message, then reverted (confirmed green again
    /// afterwards).
    #[tokio::test]
    async fn every_input_spec_variant_dispatches_to_real_ingest_not_a_stub() {
        // Shrunk so the UDP-bind-but-no-data-arrives path (Rtp/TsUdp) and the
        // SRT-caller-dialing-nobody path resolve in well under a second
        // instead of waiting the production 30 s/10 s defaults.
        let config = crate::config::Config {
            ingest_connect_timeout_secs: 0.2,
            ingest_read_timeout_secs: 0.2,
            ..crate::config::Config::default()
        };

        // A TCP port nothing listens on: every HTTP/RTSP-based puller gets a
        // connection refused practically instantly.
        let refused = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
        let refused_addr = refused.local_addr().expect("local addr");
        drop(refused);

        let reserve_udp = || {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").expect("reserve udp port");
            let addr = s.local_addr().expect("local addr");
            drop(s);
            addr
        };
        let quiet_rtp_addr = reserve_udp();
        let quiet_ts_udp_addr = reserve_udp();
        let quiet_srt_addr = reserve_udp();

        // Steal an RTMP listen port so `AsyncRtmpServer::bind` itself fails
        // on every attempt -- no client connection needed at all, since
        // `RtmpRoute`'s accept-pump task (unlike every other variant here)
        // has no timeout of its own on `server.accept().await` and would
        // otherwise hang forever waiting for a publisher that never arrives.
        let rtmp_thief =
            std::net::TcpListener::bind("127.0.0.1:0").expect("steal an rtmp listen port");
        let rtmp_addr = rtmp_thief.local_addr().expect("local addr");

        let variants: Vec<crate::config::InputSpec> = vec![
            crate::config::InputSpec::Rtsp {
                url: format!("rtsp://{refused_addr}/x"),
                auth: None,
            },
            crate::config::InputSpec::Rtp {
                addr: quiet_rtp_addr.to_string(),
                sdp: minimal_rtp_sdp(),
                multicast_group: None,
            },
            crate::config::InputSpec::TsUdp {
                addr: quiet_ts_udp_addr.to_string(),
                multicast_group: None,
            },
            crate::config::InputSpec::TsHttp {
                url: format!("http://{refused_addr}/x"),
                auth: None,
            },
            crate::config::InputSpec::Srt {
                listen: None,
                remote: Some(quiet_srt_addr.to_string()),
                stream_id: None,
                latency_ms: None,
            },
            crate::config::InputSpec::HlsPull {
                url: format!("http://{refused_addr}/x"),
                auth: None,
            },
            crate::config::InputSpec::DashPull {
                url: format!("http://{refused_addr}/x"),
                auth: None,
            },
            crate::config::InputSpec::SmoothPull {
                url: format!("http://{refused_addr}/x"),
                auth: None,
            },
            crate::config::InputSpec::Rtmp {
                listen: rtmp_addr.to_string(),
                app: None,
                stream_key: None,
            },
        ];
        assert_eq!(
            variants.len(),
            9,
            "every non-Custom InputSpec kind must be represented exactly once"
        );

        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let registry = SchemeRegistry::new();

        for spec in variants {
            assert!(
                every_input_spec_variant_is_named_here(&spec),
                "compile-time net: every match arm returns true"
            );

            let route = crate::config::Route {
                name: "cam".into(),
                input: spec.clone(),
                outputs: vec![crate::output::OutputKind::LlHls],
                dvr: DvrConfig::default(),
            };
            let store = Arc::new(RouteHandle::new(
                config.target_duration_secs,
                config.part_target_ms,
                config.window_segments,
            ));

            let handle = spawn_ingest(
                &route,
                store.clone(),
                &config,
                &registry,
                shutdown_rx.clone(),
            )
            .unwrap_or_else(|e| panic!("spawn_ingest must accept {spec:?}, got {e}"));

            // No `.await` has happened on this task yet since `store` was
            // constructed, so the spawned supervisor task has not had a
            // chance to run a single instruction -- this is a deterministic
            // check of `RouteHandle::new`'s own initial value, not a race.
            assert_eq!(
                store.health(),
                HealthState::Connecting,
                "variant {spec:?} must start Connecting"
            );

            // HANG GUARD (issue #807): every input variant here fails fast
            // (unreachable/invalid target) so this normally resolves in low
            // tens of ms. Raised for load tolerance -- only job is to fail
            // "never reaches Reconnecting" rather than hang.
            let reached_reconnecting = tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    if store.health() == HealthState::Reconnecting {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .is_ok();
            assert!(
                reached_reconnecting,
                "variant {spec:?} never left HealthState::Connecting within the hang guard \
                 (still {:?}) -- looks like a stubbed dispatch arm",
                store.health()
            );
            assert_eq!(
                store.health(),
                HealthState::Reconnecting,
                "variant {spec:?} must be Reconnecting after its first failed attempt"
            );

            handle.abort();
        }

        drop(rtmp_thief);
    }

    /// Issue #934: `rtmp://host/app/streamkey` must split into
    /// `app="app"`, `stream_key="streamkey"` — before this fix, `app` was
    /// derived from the *whole* path (`"app/streamkey"`) and `stream_key`
    /// was always `""`, so the publish target was wrong end to end.
    #[test]
    fn rtmp_url_splits_app_and_stream_key() {
        assert_eq!(
            rtmp_app_and_stream_key("rtmp://host/app/streamkey"),
            ("app".to_string(), "streamkey".to_string()),
            "two path segments: app + stream key"
        );
        assert_eq!(
            rtmp_app_and_stream_key("rtmp://host:1935/live/mystream"),
            ("live".to_string(), "mystream".to_string()),
            "with an explicit port"
        );
        assert_eq!(
            rtmp_app_and_stream_key("rtmp://host/app/instance/streamkey"),
            ("app/instance".to_string(), "streamkey".to_string()),
            "multi-segment app: everything but the last segment"
        );
        assert_eq!(
            rtmp_app_and_stream_key("rtmp://host/app"),
            ("app".to_string(), String::new()),
            "one segment only: no stream key given"
        );
        assert_eq!(
            rtmp_app_and_stream_key("rtmp://host"),
            ("live".to_string(), String::new()),
            "no path at all: falls back to the pre-existing \"live\" default"
        );
        assert_eq!(
            rtmp_app_and_stream_key("not a url"),
            ("live".to_string(), String::new()),
            "unparseable URL: same fallback, never panics"
        );
    }
}
