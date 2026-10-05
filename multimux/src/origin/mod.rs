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
pub mod limit;
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
    /// How long a request may wait for a concurrency permit before it is shed
    /// with `503` (issue #1083, B) — see [`limit::BudgetLimitLayer::new`].
    /// Config-surfaced so an operator can tighten or loosen it.
    pub queue_timeout: Duration,
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

/// Default concurrency-queue wait — re-exported from [`limit`] so a config
/// reader and the limit itself cannot drift.
pub const DEFAULT_QUEUE_TIMEOUT: Duration = limit::DEFAULT_QUEUE_TIMEOUT;

impl Default for HttpLimits {
    fn default() -> Self {
        HttpLimits {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_concurrent_requests: DEFAULT_MAX_CONCURRENT_REQUESTS,
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
            queue_timeout: DEFAULT_QUEUE_TIMEOUT,
        }
    }
}

impl From<&crate::config::Config> for HttpLimits {
    fn from(cfg: &crate::config::Config) -> Self {
        // `Duration::from_secs_f64` PANICS on a negative, NaN or overflowing
        // value. A `Config` reaching here is normally validated, but
        // `HttpLimits::from` is also reachable from an unvalidated one
        // (a caller-constructed `Config`), so convert defensively (item 7):
        // clamp into a sane range instead of panicking.
        HttpLimits {
            request_timeout: secs_or_default(cfg.request_timeout_secs, DEFAULT_REQUEST_TIMEOUT),
            max_concurrent_requests: cfg.max_concurrent_requests,
            max_request_body_bytes: cfg.max_request_body_bytes,
            queue_timeout: secs_or_default(
                cfg.concurrency_queue_timeout_secs,
                DEFAULT_QUEUE_TIMEOUT,
            ),
        }
    }
}

/// A finite, positive `Duration` from `secs`, or `fallback` for a negative,
/// NaN or overflowing value — never a panic (item 7).
fn secs_or_default(secs: f64, fallback: Duration) -> Duration {
    if secs.is_finite() && secs > 0.0 {
        Duration::try_from_secs_f64(secs).unwrap_or(fallback)
    } else {
        fallback
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
/// [`HttpLimits::request_timeout`] (so the timeout clock only runs once a
/// request holds a concurrency slot) — see [`HttpLimits`] (issue #663 P5,
/// audit-concurrency #3).
///
/// [`HttpLimits::max_concurrent_requests`] is applied here, via
/// [`limit::BudgetLimitLayer`], whose semaphores live behind `Arc`s and are
/// therefore shared by every per-endpoint clone (`Router::layer` clones a
/// layer once per route) — that is exactly why it can be a `Router::layer`
/// where [`tower::limit::ConcurrencyLimitLayer`] could not (it owns its
/// `Semaphore`, so each clone built a fresh one — audit run 7, W4). Applying
/// it here (rather than only at a serve boundary) means a library caller that
/// drives `router()` directly — a test, or an embedder's own server — gets
/// the same bound a `serve*` entry point does.
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
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            limits.request_timeout,
        ))
        .layer(RequestBodyLimitLayer::new(limits.max_request_body_bytes))
        .layer(middleware::from_fn_with_state(state.clone(), track_http))
        // The concurrency bound is outermost: it must see every request
        // including its own `503` (which `track_http` never observes, being
        // layered inside it), and it must be *outside* the timeout layer so
        // the queue wait is bounded by the limit's own timeout, not the
        // request timeout. The layer is built ONCE here, because the pools
        // live in the layer value: building it per route would give one pool
        // per route.
        .layer(limit::BudgetLimitLayer::new(
            limits.max_concurrent_requests,
            (limits.max_concurrent_requests / limit::DEFAULT_BLOCKING_RELOAD_DIVISOR).max(1),
            limits.queue_timeout,
        ))
}

/// A real [`router`] over a one-stream (`cam1`) state with `ordinary`
/// concurrent ordinary-request permits and a `queue` wait — the harness
/// `tests/limit_budgets.rs` drives to hold a permit in an unread response
/// body and prove the shared pool sheds a cross-route request.
#[doc(hidden)]
pub fn limit_budget_test_app(ordinary: usize, queue: Duration) -> Router {
    let store = Arc::new(RouteHandle::new(4.0, 500, 4));
    store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
    store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
    let mut streams = HashMap::new();
    streams.insert(
        "cam1".to_string(),
        (
            store,
            vec![Arc::new(crate::output::llhls::LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let state = AppState::new(streams).with_limits(HttpLimits {
        max_concurrent_requests: ordinary,
        queue_timeout: queue,
        ..HttpLimits::default()
    });
    router(Arc::new(state))
}

/// The local address of the accepted connection, injected per connection by
/// the server (`serve_hyper_util`; a handler reachable only through an axum
/// `Router` has no stream to ask).
#[derive(Clone, Copy, Debug)]
pub struct LocalAddr(pub std::net::SocketAddr);

/// How long a client may take to finish its request header before the
/// connection is closed (SP2.1). `axum::serve` sets none.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Hard cap on concurrent live connections (accepted and being served). A
/// connection accepted past this cap is closed immediately, at `accept`,
/// rather than parked in the kernel backlog.
pub const MAX_CONNECTIONS: usize = 1024;

/// How long, once shutdown begins, an already-streaming connection is given
/// to finish before it is force-closed. Only applies to connections in
/// flight at shutdown; a long-lived media response is never truncated before
/// this bound, and never at all if it finishes inside it.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(30);

/// Back-off between `accept()` attempts after a transient error (EMFILE,
/// ECONNABORTED, ENFILE). An accept error must never end the listener.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Serve `app` on `listener` through hyper-util's auto builder with a Tokio
/// timer, [`HEADER_READ_TIMEOUT`], a [`MAX_CONNECTIONS`] connection cap
/// enforced at `accept`, and graceful shutdown on `token` that drains
/// in-flight responses up to [`DRAIN_DEADLINE`] before force-closing. There
/// is deliberately NO total per-connection deadline: the origin serves
/// long-lived responses (LL-HLS blocking reload, DVR catch-up, TS streams)
/// whose bodies a hard cap would truncate mid-stream.
///
/// `ConnectInfo<SocketAddr>` (read by `output_auth_gate`) is injected by
/// wrapping the router in `axum::Extension` per connection — the same
/// mechanism axum's own `IntoMakeServiceWithConnectInfo` uses, built here
/// because `axum::serve::IncomingStream` cannot be constructed outside axum
/// (private fields, axum 0.8.9 `serve/mod.rs:424`).
///
/// The origin's listeners are plain HTTP/1 (no h2c/TLS-ALPN upgrade exists in
/// any `serve*` path), so [`serve_impl`] uses `hyper`'s http1 connection
/// directly — no HTTP/2 prior-knowledge preface is served, and no h2/fnv
/// dependency is pulled in.
pub(crate) async fn serve_hyper_util(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
) -> std::io::Result<()> {
    serve_hyper_util_with_timeout(listener, app, token, HEADER_READ_TIMEOUT, DRAIN_DEADLINE).await
}

/// [`serve_hyper_util`] with explicit header-read + drain timeouts (test-only).
#[doc(hidden)]
pub async fn serve_hyper_util_with_timeout(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    drain: Duration,
) -> std::io::Result<()> {
    serve_hyper_util_with_limits(listener, app, token, header_read, drain, MAX_CONNECTIONS).await
}

/// [`serve_hyper_util`] with explicit header-read + drain timeouts AND a
/// connection cap (test-only): the cap is what the connection-cap test dials
/// low so it can observe over-cap sockets being closed at `accept`.
#[doc(hidden)]
pub async fn serve_hyper_util_with_limits(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    drain: Duration,
    max_connections: usize,
) -> std::io::Result<()> {
    serve_impl(
        listener,
        app,
        token,
        header_read,
        drain,
        max_connections,
        |l: Arc<tokio::net::TcpListener>| Box::pin(async move { l.accept().await }),
    )
    .await
}

/// Serve `app` with an injectable accept source (test-only): `accept_source`
/// is called for every connection and must return the next connection's
/// `(stream, remote_addr)`, allowing a test to inject a transient `accept()`
/// error (EMFILE) and prove the listener survives it.
#[doc(hidden)]
pub async fn serve_hyper_util_with_accept_source<A, Fut>(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    drain: Duration,
    accept_source: A,
) -> std::io::Result<()>
where
    A: Fn(Arc<tokio::net::TcpListener>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)>>
        + Send
        + 'static,
{
    serve_impl(
        listener,
        app,
        token,
        header_read,
        drain,
        MAX_CONNECTIONS,
        accept_source,
    )
    .await
}

/// [`serve_hyper_util_with_limits`] plus a live-task gauge (test-only): `gauge`
/// tracks the number of connection tasks currently spawned but not yet
/// reaped, so a test can assert the finished-task set stays bounded across
/// many sequential connections (finding N1).
#[doc(hidden)]
pub async fn serve_hyper_util_with_task_gauge(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    drain: Duration,
    max_connections: usize,
    live_tasks: Arc<std::sync::atomic::AtomicUsize>,
) -> std::io::Result<()> {
    serve_impl_with_task_gauge(
        listener,
        app,
        token,
        header_read,
        drain,
        max_connections,
        Some(live_tasks),
        |l: Arc<tokio::net::TcpListener>| Box::pin(async move { l.accept().await }),
    )
    .await
}

/// The core accept/drain loop shared by [`serve_hyper_util`] (a `Router`) and
/// [`serve_hyper_util_service`] (a per-connection service factory): every
/// connection is wrapped in `hyper_util::server::graceful::GracefulShutdown`
/// so shutdown drains in-flight responses up to `drain` before force-closing.
async fn serve_impl<A, Fut>(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    drain: Duration,
    max_connections: usize,
    accept_source: A,
) -> std::io::Result<()>
where
    A: Fn(Arc<tokio::net::TcpListener>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)>>
        + Send
        + 'static,
{
    serve_impl_with_task_gauge(
        listener,
        app,
        token,
        header_read,
        drain,
        max_connections,
        None,
        accept_source,
    )
    .await
}

/// [`serve_impl`] with an optional live-task gauge (test-only): the gauge is
/// incremented when a connection task is spawned and decremented when it
/// finishes, so a test can assert the finished-task set is reaped (bounded)
/// rather than accumulating one entry per connection.
#[allow(clippy::too_many_arguments)]
async fn serve_impl_with_task_gauge<A, Fut>(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    drain: Duration,
    max_connections: usize,
    live_tasks: Option<Arc<std::sync::atomic::AtomicUsize>>,
    accept_source: A,
) -> std::io::Result<()>
where
    A: Fn(Arc<tokio::net::TcpListener>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)>>
        + Send
        + 'static,
{
    use std::net::SocketAddr;
    use std::sync::Arc;

    use hyper::server::conn::http1;
    use hyper_util::rt::{TokioIo, TokioTimer};
    use hyper_util::server::graceful::GracefulShutdown;
    use hyper_util::service::TowerToHyperService;

    let listener = Arc::new(listener);
    let conns = Arc::new(tokio::sync::Semaphore::new(max_connections));
    let graceful = GracefulShutdown::new();
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        // Reap finished connection tasks continuously (never `join_next` only
        // after the loop exits): a long-running listener would otherwise retain
        // one finished entry per connection ever accepted. The gauge (when
        // present) decrements by the number reaped so it reflects the LIVE
        // (unreaped) set exactly.
        if let Some(gauge) = &live_tasks {
            let mut reaped = 0usize;
            while tasks.try_join_next().is_some() {
                reaped += 1;
            }
            gauge.fetch_sub(reaped, std::sync::atomic::Ordering::Relaxed);
        } else {
            while tasks.try_join_next().is_some() {}
        }

        let accepted = tokio::select! {
            () = token.cancelled() => break,
            accepted = accept_source(Arc::clone(&listener)) => accepted,
        };
        let (stream, remote_addr) = match accepted {
            Ok(ok) => ok,
            Err(e) => {
                tracing::warn!(error = %e, "origin: accept error; backing off and continuing");
                // Back off (bounded) so a transient EMFILE/ENFILE storm does
                // not spin; the listener itself is never torn down.
                tokio::select! {
                    () = token.cancelled() => break,
                    () = tokio::time::sleep(ACCEPT_BACKOFF) => {}
                }
                continue;
            }
        };

        // Enforce the connection cap AT accept: a connection past the cap is
        // closed here (dropped), not left unaccepted in the kernel backlog.
        let permit = match conns.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!("origin: connection cap reached; closing the accepted socket");
                drop(stream);
                continue;
            }
        };

        let app = app.clone();
        let watcher = graceful.watcher();
        if let Some(gauge) = &live_tasks {
            gauge.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        tasks.spawn(async move {
            let _permit = permit;
            use axum::extract::connect_info::ConnectInfo;
            use tower::Layer as _;
            let local_addr = stream.local_addr().ok();
            let svc = axum::Extension(ConnectInfo(remote_addr as SocketAddr))
                .layer(axum::Extension(local_addr.map(LocalAddr)).layer(app));
            let mut builder = http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(Some(header_read));
            let io = TokioIo::new(stream);
            let conn = builder.serve_connection(io, TowerToHyperService::new(svc));
            // `watch` signals `graceful_shutdown` on the connection and keeps
            // awaiting it, so an in-flight response drains to completion
            // rather than being cut when the watcher fires.
            let _ = watcher.watch(conn).await;
        });
    }

    // Stop accepting (loop exited). Signal every watched connection to
    // graceful-shutdown and drain them up to `DRAIN_DEADLINE`; force-close
    // any straggler that is still open past that bound.
    if tokio::time::timeout(drain, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("origin: draining connections past the deadline; force-closing");
        tasks.abort_all();
    }
    while tasks.join_next().await.is_some() {}
    drop(conns);
    Ok(())
}

/// [`serve_hyper_util`] for a caller-supplied per-connection service factory —
/// the admin media listener's `DynamicMediaService` shape. `build` receives
/// only the PEER address: the service it builds is a per-request dispatcher
/// over shared state (`DynamicMediaService` holds an `Arc<RouteRegistry>`, not
/// the stream), so the stream is never moved into the closure and stays owned
/// by the connection task for the `TokioIo` wrap below. All timeouts/bounds
/// are identical to [`serve_hyper_util`].
pub(crate) async fn serve_hyper_util_service<S, F, Fut>(
    listener: tokio::net::TcpListener,
    build: F,
    token: tokio_util::sync::CancellationToken,
) -> std::io::Result<()>
where
    F: Fn(std::net::SocketAddr) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = S> + Send + 'static,
    S: tower::Service<
            hyper::Request<hyper::body::Incoming>,
            Response = axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    use std::sync::Arc;

    use hyper::server::conn::http1;
    use hyper_util::rt::{TokioIo, TokioTimer};
    use hyper_util::server::graceful::GracefulShutdown;
    use hyper_util::service::TowerToHyperService;

    let conns = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let graceful = GracefulShutdown::new();
    let mut tasks = tokio::task::JoinSet::new();
    let build = Arc::new(build);

    loop {
        // Reap finished connection tasks continuously (see `serve_impl`).
        while tasks.try_join_next().is_some() {}

        let accepted = tokio::select! {
            () = token.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, remote_addr) = match accepted {
            Ok(ok) => ok,
            Err(e) => {
                tracing::warn!(error = %e, "origin: accept error; backing off and continuing");
                tokio::select! {
                    () = token.cancelled() => break,
                    () = tokio::time::sleep(ACCEPT_BACKOFF) => {}
                }
                continue;
            }
        };
        let permit = match conns.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!("origin: connection cap reached; closing the accepted socket");
                drop(stream);
                continue;
            }
        };

        let build = Arc::clone(&build);
        let watcher = graceful.watcher();
        tasks.spawn(async move {
            let _permit = permit;
            let svc = build(remote_addr).await;
            let mut builder = http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(Some(HEADER_READ_TIMEOUT));
            let io = TokioIo::new(stream);
            let conn = builder.serve_connection(io, TowerToHyperService::new(svc));
            let _ = watcher.watch(conn).await;
        });
    }

    if tokio::time::timeout(DRAIN_DEADLINE, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("origin: draining connections past the deadline; force-closing");
        tasks.abort_all();
    }
    while tasks.join_next().await.is_some() {}
    drop(conns);
    Ok(())
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
pub(crate) async fn output_auth_gate(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(verifier) = &state.output_auth else {
        return next.run(req).await;
    };
    match check_output_auth(verifier, &req) {
        None => next.run(req).await,
        Some(resp) => resp,
    }
}

/// The shared output-auth decision: `None` when `req` is authorized,
/// `Some(401)` (with the challenge) when it is not, and `None` for an
/// `OPTIONS` preflight (which never carries the request's credentials). Used
/// by [`output_auth_gate`] and by `crate::output::whep`'s own router, so the
/// two share one implementation (SP2.4).
pub(crate) fn check_output_auth(verifier: &Verifier, req: &Request) -> Option<Response> {
    if req.method() == Method::OPTIONS {
        return None;
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
        AuthResult::Ok => None,
        AuthResult::Unauthorized => {
            let mut resp = StatusCode::UNAUTHORIZED.into_response();
            // `challenge_for` (not `challenge`): an expired nonce must be
            // answered with its own fresh nonce carrying `stale=true` (RFC
            // 7616 §3.3), which only `challenge_for` can tell from any other
            // Unauthorized cause — it re-inspects the caller's own `ctx`, the
            // same request `verify` just rejected.
            if let Ok(value) = HeaderValue::from_str(&verifier.challenge_for(&ctx)) {
                resp.headers_mut().insert(header::WWW_AUTHENTICATE, value);
            }
            Some(resp)
        }
        // `AuthResult` is `#[non_exhaustive]` (broadcast-auth may add finer-
        // grained outcomes later, e.g. a rate-limited variant) — default-deny
        // any variant this middleware doesn't yet know how to treat as
        // authenticated, rather than silently letting an unrecognized
        // outcome through.
        _ => Some(StatusCode::UNAUTHORIZED.into_response()),
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
    use headers::HeaderMapExt as _;

    let path = req.uri().path().to_string();
    let mut resp = next.run(req).await;
    let cache_control = cache_control_for(&path, resp.status());
    let headers = resp.headers_mut();
    headers.typed_insert(headers::AccessControlAllowOrigin::ANY);
    headers.typed_insert(
        [Method::GET, Method::HEAD, Method::OPTIONS]
            .into_iter()
            .collect::<headers::AccessControlAllowMethods>(),
    );
    // An explicit header list, not `*`: the Fetch spec's `*` wildcard never
    // covers `Authorization`, so a browser player sending Basic or Bearer
    // output auth cross-origin fails the preflight (audit run 7, W18). `Range`
    // covers byte-range fetches; `Content-Type` a preflight for a body.
    //
    // No `Access-Control-Allow-Credentials`: it is meaningless (and the Fetch
    // spec forbids it) with the `*` origin wildcard this origin sends, and
    // these are not credentialed requests — the browser sends the auth header
    // explicitly, not as an ambient cookie.
    headers.typed_insert(
        [header::AUTHORIZATION, header::RANGE, header::CONTENT_TYPE]
            .into_iter()
            .collect::<headers::AccessControlAllowHeaders>(),
    );
    // Let a browser read the response metadata a media client needs
    // (byte-range and cache validators), which CORS otherwise hides.
    headers.typed_insert(
        [
            header::CONTENT_LENGTH,
            header::CONTENT_RANGE,
            header::DATE,
            header::ETAG,
        ]
        .into_iter()
        .collect::<headers::AccessControlExposeHeaders>(),
    );
    // The `Access-Control-Allow-Origin: *` value does not vary by request, but
    // `Vary` is still set so a shared cache never serves a response whose CORS
    // headers were computed for a different `Origin`.
    // Append, not insert: the response may already carry a `Vary` (e.g. from a
    // compression layer), and `insert` would drop it.
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    headers.typed_insert(cache_control);
    resp
}

/// The `Cache-Control` for a response to `path` with `status`, as a typed
/// [`headers::CacheControl`].
///
/// | resource | success | otherwise |
/// |---|---|---|
/// | manifest / playlist (`.m3u8`, `.mpd`) | `no-cache` | `no-cache` |
/// | instance-named init/segment/part (`init-{t}-{instance}-{gen}.mp4`, `seg-{t}-{instance}-{msn}.*`, `part-{t}-{instance}-{msn}.{i}.*`) | `max-age=31536000, immutable` | `no-cache` |
/// | token-less `seg-{t}-{msn}.*`, `part-{t}-{msn}.{i}.*` (DASH/Smooth templates cannot carry the token), `catchup/seg-*` (archive *or* live tail) | `max-age=10` | `no-cache` |
/// | bare `init-{t}.mp4` (the *current* init) and anything else | `no-cache` | `no-cache` |
///
/// `immutable` only where the name carries the origin's instance token
/// (`hls_runtime::server::HlsOrigin::instance`), which differs for every
/// origin built — across reconnects and process restarts — so a name never
/// maps to other bytes (audit r09-C2, issue #1030). Where uniqueness cannot be
/// guaranteed the response gets a finite `max-age` without `immutable`. Every
/// non-success response (a `404` for a part that does not exist *yet*, `401`,
/// `503`) is `no-cache`: a year-long `immutable` on a transient error let a
/// CDN keep serving it after the resource appeared.
fn cache_control_for(path: &str, status: StatusCode) -> headers::CacheControl {
    use std::time::Duration;

    let file = path.rsplit('/').next().unwrap_or(path);
    let manifest = file.ends_with(".m3u8") || file.ends_with(".mpd") || !status.is_success();
    let kind = if manifest {
        MediaName::Other
    } else {
        media_name_kind(path)
    };
    match kind {
        MediaName::InstanceNamed => headers::CacheControl::new()
            .with_immutable()
            .with_max_age(Duration::from_secs(31_536_000)),
        MediaName::TokenLess => headers::CacheControl::new().with_max_age(Duration::from_secs(10)),
        MediaName::Other => headers::CacheControl::new().with_no_cache(),
    }
}

enum MediaName {
    InstanceNamed,
    TokenLess,
    Other,
}

/// Classify a media resource path by how many numeric fields its name has.
fn media_name_kind(path: &str) -> MediaName {
    let file = path.rsplit('/').next().unwrap_or(path);
    let numeric = |fields: &str, n: usize| {
        let parts: Vec<&str> = fields.split('-').collect();
        parts.len() == n
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    // `seg-{t}-{msn}` / `seg-{t}-{instance}-{msn}` (any extension).
    if let Some(rest) = file.strip_prefix("seg-") {
        let stem = rest.rsplit_once('.').map_or(rest, |(s, _)| s);
        return if numeric(stem, 3) {
            MediaName::InstanceNamed
        } else if numeric(stem, 2) || path.contains("/catchup/") {
            MediaName::TokenLess
        } else {
            MediaName::Other
        };
    }
    // `part-{t}-{msn}.{i}.ext` / `part-{t}-{instance}-{msn}.{i}.ext`.
    if let Some(rest) = file.strip_prefix("part-") {
        let mut it = rest.splitn(2, '.');
        let names = it.next().unwrap_or("");
        return if numeric(names, 3) {
            MediaName::InstanceNamed
        } else if numeric(names, 2) {
            MediaName::TokenLess
        } else {
            MediaName::Other
        };
    }
    if let Some(rest) = file.strip_prefix("init-")
        && let Some(stem) = rest.strip_suffix(".mp4")
        && numeric(stem, 3)
    {
        return MediaName::InstanceNamed;
    }
    MediaName::Other
}

/// `Cache-Control` for manifests (`master.m3u8`/`media.m3u8`/`manifest.mpd`):
/// they must always be re-fetched for liveness, never served stale from a
/// cache.
#[cfg(test)]
const CACHE_CONTROL_MANIFEST: &str = "no-cache";

/// `Cache-Control` for instance-named init/segment/part byte ranges: the name
/// carries the origin's instance token, so it maps to one origin's bytes for
/// ever (see [`cache_control_for`]).
#[cfg(test)]
const CACHE_CONTROL_IMMUTABLE: &str = "immutable, max-age=31536000";

/// `Cache-Control` for media whose name cannot be proven unique to its bytes
/// (see [`cache_control_for`]'s table): a short, finite `max-age`.
#[cfg(test)]
const CACHE_CONTROL_SHORT: &str = "max-age=10";

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
/// The response body is **never buffered** (audit r07-C10, issue #1083): the
/// byte count is taken from the frames as they pass through, and recorded
/// when the body ends or is dropped (a client that disconnects mid-stream
/// still counts the bytes it was sent). Collecting it first would hold an
/// in-progress LL-DASH segment (`stream_in_progress_segment`) back until it
/// had fully closed and defeat chunked low-latency delivery (#721).
///
/// A body with a known exact length but no `Content-Length` header gets one
/// here: re-wrapping the body as a stream hides its size from hyper, which
/// would otherwise switch every such response to chunked transfer coding.
async fn track_http(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let start = std::time::Instant::now();
    let resp = next.run(req).await;
    let elapsed = start.elapsed();

    let (route, kind) = classify_path(&state, &path);
    let status = resp.status().as_u16().to_string();

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

    let mut counter = BytesServedGuard {
        route,
        kind,
        bytes: 0,
    };
    // A method call, not a field assignment: closures capture disjoint
    // fields, so `counter.bytes += ..` would move a copy of the integer and
    // leave the guard (and its `Drop`) behind in this function.
    restream_body(resp, move |data| counter.add(data.len()))
}

/// Re-wrap `resp`'s body as a pass-through stream, calling `on_chunk` for each
/// data chunk as it goes by — never collecting the body, so a streaming
/// response keeps streaming. `on_chunk` (and anything it owns, e.g. a
/// concurrency permit or a metrics guard) is dropped when the body ends or
/// is abandoned.
///
/// A body with a known exact length but no `Content-Length` header gets one:
/// the stream wrapper hides the size from hyper, which would otherwise answer
/// every such response with chunked transfer coding.
pub(crate) fn restream_body(
    resp: Response,
    mut on_chunk: impl FnMut(&axum::body::Bytes) + Send + 'static,
) -> Response {
    use axum::body::HttpBody;
    use futures_util::StreamExt;

    let (mut parts, body) = resp.into_parts();
    if let Some(exact) = body.size_hint().exact()
        && !parts.headers.contains_key(header::CONTENT_LENGTH)
    {
        parts
            .headers
            .insert(header::CONTENT_LENGTH, HeaderValue::from(exact));
    }
    let stream = body.into_data_stream().map(move |chunk| {
        if let Ok(data) = &chunk {
            on_chunk(data);
        }
        chunk
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

/// Records [`crate::prometheus::BYTES_SERVED_TOTAL`] when dropped — i.e. when
/// the response body has been fully sent, or abandoned mid-stream.
struct BytesServedGuard {
    route: String,
    kind: &'static str,
    bytes: u64,
}

impl BytesServedGuard {
    fn add(&mut self, len: usize) {
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
    }
}

impl Drop for BytesServedGuard {
    fn drop(&mut self) {
        metrics::counter!(
            crate::prometheus::BYTES_SERVED_TOTAL,
            "route" => std::mem::take(&mut self.route),
            "path" => self.kind,
        )
        .increment(self.bytes);
    }
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
    serve_with_registry_impl(config, registry, Some(path), None).await
}

/// [`serve_config_file_with_registry`] over caller-supplied, already-bound
/// media AND admin listeners (SP7.1) — the admin-enabled shape a test uses to
/// keep `/admin/reload` working while binding port 0.
pub async fn serve_config_file_with_registry_on_admin(
    media: tokio::net::TcpListener,
    admin: tokio::net::TcpListener,
    path: impl AsRef<Path>,
    registry: SchemeRegistry,
) -> crate::Result<()> {
    let path = path.as_ref().to_path_buf();
    let config = crate::config::Config::from_json_file(&path)?;
    serve_with_registry_impl(
        config,
        registry,
        Some(path),
        Some(PreboundListeners {
            media,
            admin: Some(admin),
        }),
    )
    .await
}

/// [`serve_config_file_with_registry_on_admin`] plus caller-bound route
/// sockets (SP7.1, `test-seams` only). A config file cannot carry
/// [`crate::config::Config::prebound`] (it is `serde(skip)`), so a test that
/// serves a *file-defined* route and still wants to hand a pre-bound
/// listener/socket in uses this entry point: the binds are spliced into the
/// config loaded from `path` before it is served.
#[cfg(feature = "test-seams")]
#[doc(hidden)]
pub async fn serve_config_file_with_registry_on_admin_prebound(
    media: tokio::net::TcpListener,
    admin: tokio::net::TcpListener,
    path: impl AsRef<Path>,
    registry: SchemeRegistry,
    prebound: crate::config::PreboundBinds,
) -> crate::Result<()> {
    let path = path.as_ref().to_path_buf();
    let mut config = crate::config::Config::from_json_file(&path)?;
    config.prebound = prebound;
    serve_with_registry_impl(
        config,
        registry,
        Some(path),
        Some(PreboundListeners {
            media,
            admin: Some(admin),
        }),
    )
    .await
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
/// receipt, the server stops accepting new connections and drains in-flight
/// requests (including blocked LL-HLS long-poll reloads) up to
/// [`DRAIN_DEADLINE`] via the origin's `serve_hyper_util` `GracefulShutdown`, the same
/// signal breaks every route's supervise loop, and `serve` joins each
/// supervisor task (forcibly aborting one that doesn't return within a short
/// grace period) before returning `Ok(())`.
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
/// Pre-bound listeners for the `_on` entry points (SP7.1): a caller binds
/// `127.0.0.1:0`, reads the live address, and passes the listener in, so no
/// test reserves a port and then races to re-bind it.
#[doc(hidden)]
pub struct PreboundListeners {
    /// The media (shared-router) listener.
    pub media: tokio::net::TcpListener,
    /// The admin-API listener, when `config.admin` is set.
    pub admin: Option<tokio::net::TcpListener>,
}

pub async fn serve_with_registry(
    config: crate::config::Config,
    registry: SchemeRegistry,
) -> crate::Result<()> {
    serve_with_registry_impl(config, registry, None, None).await
}

/// [`serve_with_registry`] over a caller-supplied, already-bound media
/// listener (SP7.1). The admin path requires [`serve_with_registry_on_admin`].
pub async fn serve_with_registry_on(
    media: tokio::net::TcpListener,
    config: crate::config::Config,
    registry: SchemeRegistry,
) -> crate::Result<()> {
    serve_with_registry_impl(
        config,
        registry,
        None,
        Some(PreboundListeners { media, admin: None }),
    )
    .await
}

/// [`serve_with_registry`] over caller-supplied, already-bound media AND
/// admin listeners (SP7.1) — the admin-enabled shape.
pub async fn serve_with_registry_on_admin(
    media: tokio::net::TcpListener,
    admin: tokio::net::TcpListener,
    config: crate::config::Config,
    registry: SchemeRegistry,
) -> crate::Result<()> {
    serve_with_registry_impl(
        config,
        registry,
        None,
        Some(PreboundListeners {
            media,
            admin: Some(admin),
        }),
    )
    .await
}

async fn serve_with_registry_impl(
    config: crate::config::Config,
    registry: SchemeRegistry,
    config_path: Option<PathBuf>,
    prebound: Option<PreboundListeners>,
) -> crate::Result<()> {
    config.validate()?;

    if config.admin.is_some() {
        return admin::serve_with_admin(config, registry, config_path, prebound).await;
    }
    let prebound_media = prebound.map(|p| p.media);

    tracing::info!(
        bind = %config.bind,
        routes = config.routes.len(),
        "multimux origin starting"
    );

    let mut streams: HashMap<String, StreamRoute> = HashMap::new();
    let target_duration_secs = config.target_duration_secs;
    let part_target_ms = config.part_target_ms;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut supervisor_handles: Vec<(String, tokio::task::JoinHandle<()>)> = Vec::new();

    // Resolved once, ahead of the per-route loop, so `spawn_whep_outputs`
    // gates viewers with the same `Verifier` `AppState::output_auth` holds
    // (issue r07-C11) — previously this was only built after every route's
    // WHEP listener had already been spawned unauthenticated.
    let output_auth: Option<Arc<Verifier>> = config
        .output_auth
        .as_ref()
        .map(|spec| resolve_verifier(spec, OUTPUT_AUTH_REALM, &registry))
        .transpose()?
        .map(Arc::new);

    for route in &config.routes {
        // Segment numbers must stay above what the DVR archive already holds
        // (a previous process's), or the archive and any HTTP cache would
        // see numbers reused for different media (audit r07-C5, #1083).
        let archive_floor = admin::archive_floor_of(route).await;
        let store = Arc::new(
            RouteHandle::new(target_duration_secs, part_target_ms, config.window_segments)
                .with_archive_floor(archive_floor)
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
        let whep_handles = spawn_whep_outputs(
            route,
            Arc::clone(&store),
            &cancel,
            output_auth.clone(),
            &config,
        );
        for h in whep_handles {
            supervisor_handles.push((route.name.clone(), h));
        }

        let name = route.name.clone();
        let handle = spawn_ingest(route, store, &config, &registry, cancel.clone())?;
        supervisor_handles.push((name, handle));
    }

    let mut app_state = AppState::new(streams).with_limits(HttpLimits::from(&config));
    if let Some(verifier) = output_auth {
        app_state = app_state.with_output_auth(verifier);
    }
    let state = Arc::new(app_state);
    let listener = match prebound_media {
        Some(listener) => listener,
        None => tokio::net::TcpListener::bind(config.bind.as_str()).await?,
    };
    // The shutdown watcher races the server but is not part of it: firing
    // `cancel` makes `serve_hyper_util` stop accepting AND drain its in-flight
    // connections (up to `DRAIN_DEADLINE`) before it returns.
    let serve_cancel = cancel.clone();
    let shutdown_cancel = cancel.clone();
    let shutdown_future = async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        shutdown_cancel.cancel();
    };
    // The server runs on `serve_hyper_util` (SP2.1), which injects the
    // `ConnectInfo<SocketAddr>` extension `output_auth_gate` reads for
    // `RequestContext::peer_addr` (issue #663 extensibility wave part 1) —
    // without it, `peer_addr` would always be `None`, same as it is in tests
    // that `oneshot` the router directly.
    //
    // The concurrency bound is applied inside `router()` itself (audit run
    // 7, W4/B), so it is present whether this entry point or a library
    // caller's own server drives it.
    let shutdown_task = tokio::spawn(shutdown_future);
    let serve_result = serve_hyper_util(listener, router(state), serve_cancel).await;
    shutdown_task.abort();

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
/// How long an egress task is given to wind down after its `Trunk` is
/// replaced before [`follow_trunk`] moves on to the new one regardless.
const EGRESS_REBIND_GRACE: Duration = Duration::from_secs(5);

/// Run an egress (`run`) against the route's current `Trunk` and keep it
/// bound to the route's *current* one (audit r07-C4, issue #1083).
///
/// A source reconnect builds a fresh `Trunk` and publishes it over the old
/// program; an egress that subscribed to the first `Trunk` and never looked
/// again drains a dead ring forever, silently. Here the egress is cancelled
/// (via a child of `cancel`, so a route shutdown still stops it) as soon as
/// the route's first program is bound to a different `Trunk`, given
/// [`EGRESS_REBIND_GRACE`] to finish, and started again over the new one.
/// Returns when `cancel` fires, or when an egress run ends by itself
/// (permanently failed) — exactly as before.
async fn follow_trunk<F, Fut>(
    store: Arc<RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
    run: F,
) where
    F: Fn(Arc<media_plane::trunk::Trunk>, tokio_util::sync::CancellationToken) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut trunk = tokio::select! {
        trunk = store.await_first_trunk() => trunk,
        () = cancel.cancelled() => return,
    };
    loop {
        let child = cancel.child_token();
        let mut task = std::pin::pin!(run(Arc::clone(&trunk), child.clone()));
        tokio::select! {
            () = &mut task => return,
            () = cancel.cancelled() => {
                let _ = tokio::time::timeout(EGRESS_REBIND_GRACE, &mut task).await;
                return;
            }
            next = store.await_trunk_change(&trunk) => {
                tracing::info!("source Trunk replaced — rebinding egress to the new one");
                child.cancel();
                let _ = tokio::time::timeout(EGRESS_REBIND_GRACE, &mut task).await;
                trunk = next;
            }
        }
    }
}

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
                handles.push(spawn_following(&store, cancel, move |trunk, cancel| {
                    let (url, reconnect) = (url.clone(), reconnect.clone());
                    async move {
                        tracing::info!(url = %crate::redact::redact_destination(&url), "SRT push output starting");
                        let config = crate::push::SrtTransportConfig::default();
                        crate::push::drive_push::<crate::push::SrtTransport>(
                            trunk, url, config, format, reconnect, cancel,
                        )
                        .await;
                    }
                }));
            }
            crate::output::OutputKind::RtmpPush {
                url,
                format,
                reconnect,
            } => {
                let url = url.clone();
                let format = format.unwrap_or(crate::config::PushFormat::Ts);
                let reconnect = reconnect.clone().unwrap_or_default();
                handles.push(spawn_following(&store, cancel, move |trunk, cancel| {
                    let (url, reconnect) = (url.clone(), reconnect.clone());
                    async move {
                        tracing::info!(url = %crate::redact::redact_destination(&url), "RTMP push output starting");
                        let (app, stream_key) = rtmp_app_and_stream_key(&url);
                        let config = crate::push::RtmpTransportConfig {
                            app,
                            stream_key,
                            ..Default::default()
                        };
                        crate::push::drive_push::<crate::push::RtmpTransport>(
                            trunk, url, config, format, reconnect, cancel,
                        )
                        .await;
                    }
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
                handles.push(spawn_following(&store, cancel, move |trunk, cancel| {
                    let (url, reconnect) = (url.clone(), reconnect.clone());
                    async move {
                        tracing::info!(url = %crate::redact::redact_destination(&url), "RTSP push output starting");
                        let config = crate::push::RtspTransportConfig::default();
                        crate::push::drive_push::<crate::push::RtspTransport>(
                            trunk, url, config, format, reconnect, cancel,
                        )
                        .await;
                    }
                }));
            }
            _ => {}
        }
    }
    handles
}

/// Spawn [`follow_trunk`] for `run` on the runtime.
fn spawn_following<F, Fut>(
    store: &Arc<RouteHandle>,
    cancel: &tokio_util::sync::CancellationToken,
    run: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(Arc<media_plane::trunk::Trunk>, tokio_util::sync::CancellationToken) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(follow_trunk(Arc::clone(store), cancel.clone(), run))
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
    output_auth: Option<Arc<Verifier>>,
    config: &crate::config::Config,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    for kind in &route.outputs {
        if let crate::output::OutputKind::Whep { listen } = kind {
            // A `test-hooks` caller may have bound this exact listen address
            // itself and handed the live listener in (`Config::prebound`), so
            // the route consumes it rather than racing reserve-then-rebind.
            let route_cfg = Arc::new(match config.prebound_tcp(listen) {
                Some(listener) => crate::output::whep::WhepRoute::with_listener(
                    "whep",
                    listener,
                    crate::output::whep::DEFAULT_WHEP_MAX_SESSIONS,
                ),
                None => crate::output::whep::WhepRoute::new(listen.clone()),
            });
            let output_auth = output_auth.clone();
            handles.push(spawn_following(&store, cancel, move |trunk, cancel| {
                let (route_cfg, output_auth) = (Arc::clone(&route_cfg), output_auth.clone());
                async move {
                    tracing::info!(listen = %route_cfg.listen(), "WHEP egress starting");
                    crate::output::whep::run_whep(&route_cfg, trunk, cancel, output_auth).await;
                }
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
    output_auth: Option<Arc<Verifier>>,
    config: &crate::config::Config,
) -> Vec<tokio::task::JoinHandle<()>> {
    let _ = (route, store, cancel, output_auth, config);
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
    cancel: tokio_util::sync::CancellationToken,
) -> crate::Result<tokio::task::JoinHandle<()>> {
    let name = route.name.clone();
    let timeouts = crate::source::IngestTimeouts::from(config);
    let ctx = IngestSpawn {
        name: name.clone(),
        store,
        cancel,
        window_segments: config.window_segments,
        timeouts,
    };

    Ok(match &route.input {
        crate::config::InputSpec::Rtsp { url, auth } => {
            let route_cfg = crate::source::rtsp::RtspRoute::new(name, url.clone())
                .with_timeouts(timeouts)
                .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials));
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    Err(crate::source::rtsp::run_rtsp(&cfg, trunk_config, handshake, &handle).await)
                },
            )
        }
        crate::config::InputSpec::Rtp {
            addr,
            sdp,
            multicast_group,
            socket,
        } => {
            let route_cfg = crate::source::rtp_udp::RtpUdpRoute::new(
                name,
                addr.clone(),
                sdp.clone(),
                multicast_group.clone(),
            )
            .with_socket_options(socket.to_bind_options())
            .with_timeouts(timeouts);
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    Err(
                        crate::source::rtp_udp::run_rtp_udp(&cfg, trunk_config, handshake, &handle)
                            .await,
                    )
                },
            )
        }
        crate::config::InputSpec::TsUdp {
            addr,
            multicast_group,
            socket,
        } => {
            // A `test-hooks` caller may have bound this exact `addr` itself and
            // handed the live socket in (`Config::prebound`), so the route
            // consumes it rather than racing reserve-then-rebind. A configured
            // `multicast_group` cannot be joined on a caller-bound socket, so
            // carry it onto the route: `bind` then rejects the contradiction
            // instead of silently dropping the group.
            let route_cfg = match config.prebound_udp(addr) {
                Some(socket) => {
                    let mut route = crate::source::ts_udp::TsUdpRoute::with_socket(name, socket);
                    if let Some(group) = multicast_group.clone() {
                        route = route.with_multicast_group(group);
                    }
                    route.with_timeouts(timeouts)
                }
                None => crate::source::ts_udp::TsUdpRoute::new(
                    name,
                    addr.clone(),
                    multicast_group.clone(),
                )
                .with_socket_options(socket.to_bind_options())
                .with_timeouts(timeouts),
            };
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    Err(
                        crate::source::ts_udp::run_ts_udp(&cfg, trunk_config, handshake, &handle)
                            .await,
                    )
                },
            )
        }
        crate::config::InputSpec::TsHttp { url, auth } => {
            let route_cfg = crate::source::ts_http::TsHttpRoute::new(name, url.clone())
                .with_timeouts(timeouts)
                .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials));
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    crate::source::ts_http::run_ts_http(&cfg, trunk_config, handshake, &handle)
                        .await
                },
            )
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
            if is_listener {
                // See `InputSpec::Srt`'s own doc: no passphrase field exists
                // yet, so a listener-mode route accepts any Caller.
                tracing::warn!(
                    route = %name,
                    "SRT ingest listener is running with no passphrase/auth of any kind — \
                     any caller that can reach this port may publish"
                );
            }
            let srt_route = match (listen, remote) {
                (Some(l), None) => {
                    crate::source::srt::SrtRoute::new_listener(name.clone(), l.clone())
                }
                (None, Some(r)) => {
                    crate::source::srt::SrtRoute::new_caller(name.clone(), r.clone())
                }
                _ => unreachable!("config.validate() enforces exactly one of Srt listen/remote"),
            }
            .with_stream_id(stream_id.clone())
            .with_latency_ms(*latency_ms)
            .with_timeouts(timeouts);
            spawn_supervised(
                srt_route,
                ctx,
                move |cfg, trunk_config, handshake, handle| async move {
                    if is_listener {
                        crate::source::srt::run_srt_listener_once(
                            &cfg,
                            trunk_config,
                            handshake,
                            &handle,
                        )
                        .await
                    } else {
                        crate::source::srt::run_srt_caller(&cfg, trunk_config, handshake, &handle)
                            .await
                    }
                },
            )
        }
        crate::config::InputSpec::HlsPull { url, auth } => {
            let route_cfg = crate::source::hls_pull::HlsPullRoute::new(name, url.clone())
                .with_timeouts(timeouts)
                .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials));
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    crate::source::hls_pull::run_hls_pull(&cfg, trunk_config, handshake, &handle)
                        .await
                },
            )
        }
        crate::config::InputSpec::DashPull { url, auth } => {
            let route_cfg = crate::source::dash_pull::DashPullRoute::new(name, url.clone())
                .with_timeouts(timeouts)
                .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials));
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    crate::source::dash_pull::run_dash_pull(&cfg, trunk_config, handshake, &handle)
                        .await
                },
            )
        }
        crate::config::InputSpec::SmoothPull { url, auth } => {
            let route_cfg = crate::source::smooth_pull::SmoothPullRoute::new(name, url.clone())
                .with_timeouts(timeouts)
                .with_auth(auth.as_ref().map(crate::config::AuthSpec::to_credentials));
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    crate::source::smooth_pull::run_smooth_pull(
                        &cfg,
                        trunk_config,
                        handshake,
                        &handle,
                    )
                    .await
                },
            )
        }
        crate::config::InputSpec::Rtmp {
            listen,
            app,
            stream_key,
        } => {
            if stream_key.is_none() {
                // See `InputSpec::Rtmp`'s own doc: with no `stream_key`, this
                // listener authenticates nobody.
                tracing::warn!(
                    route = %name,
                    "RTMP ingest listener is running with no stream_key configured — \
                     any publisher that can reach this port may publish"
                );
            }
            let route_cfg = crate::source::rtmp::RtmpRoute::new(name, listen.clone())
                .with_app(app.clone())
                .with_stream_key(stream_key.clone())
                .with_timeouts(timeouts);
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    Err(crate::source::rtmp::run_rtmp(&cfg, trunk_config, handshake, &handle).await)
                },
            )
        }
        #[cfg(feature = "whip")]
        crate::config::InputSpec::Whip { listen } => {
            // See `InputSpec::Whip`'s own doc: this cut has no auth knob at
            // all, so every WHIP ingest listener is unconditionally open.
            tracing::warn!(
                route = %name,
                "WHIP ingest listener is running with no authentication of any kind — \
                 any publisher that can reach this endpoint may publish"
            );
            let route_cfg = match config.prebound_tcp(listen) {
                Some(listener) => crate::source::whip::WhipRoute::with_listener(
                    name,
                    listener,
                    crate::source::whip::DEFAULT_WHIP_MAX_SESSIONS,
                )
                .with_timeouts(timeouts),
                None => crate::source::whip::WhipRoute::new(name, listen.clone())
                    .with_timeouts(timeouts),
            };
            spawn_supervised(
                route_cfg,
                ctx,
                |cfg, trunk_config, handshake, handle| async move {
                    Err(crate::source::whip::run_whip(&cfg, trunk_config, handshake, &handle).await)
                },
            )
        }
        crate::config::InputSpec::File { path, loop_file } => {
            let window_segments = ctx.window_segments;
            let loop_file = *loop_file;
            spawn_supervised(
                path.clone(),
                ctx,
                move |path, _trunk_config, handshake, handle| async move {
                    Err(crate::source::file_reader::run_file_source(
                        &path,
                        loop_file,
                        window_segments,
                        handshake,
                        &handle,
                    )
                    .await)
                },
            )
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
                name,
                params: params.clone(),
                store: ctx.store,
                target_duration_secs: config.target_duration_secs,
                part_target_ms: config.part_target_ms,
                cancel: ctx.cancel,
            })?
        }
    })
}

/// What [`spawn_supervised`] needs besides a route's own config: the route's
/// name, handle and shutdown signal, and the two settings every built-in
/// input derives its per-attempt `TrunkConfig` and handshake budget from.
struct IngestSpawn {
    name: String,
    store: Arc<RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
    window_segments: usize,
    timeouts: crate::source::IngestTimeouts,
}

/// Spawn one built-in input's [`supervisor::supervise_driver`] loop — the
/// wrapper every `InputSpec` arm of [`spawn_ingest`] used to spell out in full
/// (audit r07-O1, issue #1083). Each attempt builds a fresh `TrunkConfig` and
/// handshake budget and hands them, with the route's own config, to `run`.
fn spawn_supervised<R, F, Fut>(
    route_cfg: R,
    ctx: IngestSpawn,
    run: F,
) -> tokio::task::JoinHandle<()>
where
    R: Send + Sync + 'static,
    F: Fn(
            Arc<R>,
            media_plane::trunk::TrunkConfig,
            media_plane::ingress::HandshakePolicy,
            Arc<RouteHandle>,
        ) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = crate::Result<()>> + Send + 'static,
{
    let IngestSpawn {
        name,
        store,
        cancel,
        window_segments,
        timeouts,
    } = ctx;
    let route_cfg = Arc::new(route_cfg);
    tokio::spawn(supervisor::supervise_driver(
        move |route_handle| {
            let trunk_config = crate::source::driver_trunk_config(window_segments);
            let handshake = crate::source::handshake_policy(timeouts.connect);
            run(
                Arc::clone(&route_cfg),
                trunk_config,
                handshake,
                route_handle,
            )
        },
        store,
        Backoff::production_default(),
        name,
        cancel,
    ))
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

    /// A body whose chunks the test releases one at a time.
    fn channel_body() -> (tokio::sync::mpsc::Sender<axum::body::Bytes>, Body) {
        let (tx, rx) = tokio::sync::mpsc::channel::<axum::body::Bytes>(4);
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv()
                .await
                .map(|chunk| (Ok::<_, std::convert::Infallible>(chunk), rx))
        });
        (tx, Body::from_stream(stream))
    }

    /// The `multimux_bytes_served_total` value for `route`/`path`, from the
    /// process-wide Prometheus snapshot (`0` while the series is absent).
    fn bytes_served(state: &AppState, route: &str, kind: &str) -> u64 {
        let prefix = format!("{}{{", crate::prometheus::BYTES_SERVED_TOTAL);
        state
            .metrics_handle
            .render()
            .lines()
            .filter(|l| l.starts_with(&prefix))
            .filter(|l| l.contains(&format!("route=\"{route}\"")))
            .filter(|l| l.contains(&format!("path=\"{kind}\"")))
            .filter_map(|l| l.rsplit(' ').next()?.parse::<u64>().ok())
            .sum()
    }

    fn state_with_stream(name: &str) -> Arc<AppState> {
        let mut streams = HashMap::new();
        streams.insert(
            name.to_string(),
            (Arc::new(RouteHandle::new(4.0, 500, 4)), Vec::new()),
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

    /// Well-formedness check on a real `quick-xml` pull loop: every opening
    /// tag matches its closing tag (the reader's end-name check), entity
    /// references resolve, there is exactly one root element, and nothing is
    /// left open at the end. Genuinely biting: a mismatched/unclosed tag or a
    /// stray `&` panics.
    fn assert_well_formed_xml(xml: &str) {
        use quick_xml::Reader;
        use quick_xml::events::Event;

        let mut reader = Reader::from_str(xml);
        let mut depth = 0usize;
        let mut roots = 0usize;
        loop {
            match reader
                .read_event()
                .unwrap_or_else(|e| panic!("not well-formed XML: {e}"))
            {
                Event::Start(_) => {
                    if depth == 0 {
                        roots += 1;
                    }
                    depth += 1;
                }
                Event::Empty(_) if depth == 0 => roots += 1,
                Event::End(_) => depth -= 1,
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(depth, 0, "unclosed tags remain");
        assert_eq!(roots, 1, "exactly one root element");
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
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x33; 16],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 1,
                },
            )
            .expect("add_segment");

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
        assert!(
            hls_body
                .lines()
                .any(|l| l.starts_with("seg-1-") && l.ends_with("-1.m4s")),
            "hls body: {hls_body}"
        );

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
        // The LL-HLS playlist names the same segment (MSN 1) under its
        // instance-token form; both forms serve the same bytes through the
        // shared resource route.
        let hls_uri = hls_body
            .lines()
            .find(|l| l.starts_with("seg-1-") && l.ends_with("-1.m4s"))
            .unwrap_or_else(|| panic!("LL-HLS playlist must name segment 1: {hls_body}"))
            .to_string();
        for uri in [resolved_uri, hls_uri.as_str()] {
            let resp = app
                .clone()
                .oneshot(get(&format!("/cam1/{uri}")))
                .await
                .unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::OK, "{uri}");
            assert_eq!(body_bytes(resp).await, vec![0x33; 16], "{uri}");
        }
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
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x33; 16],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 2,
                },
            )
            .expect("add_segment");
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
            store_for_close
                .add_segment(
                    crate::route::SPTS_PROGRAM_ID,
                    transmux::ll_hls::SegmentInfo {
                        bytes: vec![0x99; 8], // distinct from the concatenated parts
                        duration: 1.0,
                        segment_seq: 2,
                        part_count: 2,
                    },
                )
                .expect("add_segment");
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
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x33; 16],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 1,
                },
            )
            .expect("add_segment");
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
            ("/cam1/seg-1-1.m4s", CACHE_CONTROL_SHORT),
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

    /// Byte-for-byte golden of the origin's response HEADERS (W2a Task 1
    /// Step 1): one line per probe, `path<TAB>status<TAB>name: value`, the
    /// header names sorted. Taken from `main` BEFORE the axum 0.8 /
    /// tower-http 0.7 bump, because `add_response_headers` and `CorsLayer`
    /// both write the wire (see `tests/golden/README.md`).
    /// `GOLDEN_BLESS=<dir>` writes instead of comparing.
    #[tokio::test]
    async fn origin_response_headers_match_golden() {
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
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x33; 16],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 1,
                },
            )
            .expect("add_segment");
        // Probe a real instance-named init resource (`init-{track}-{instance}-
        // {generation}.mp4`, the immutable-cache form) so the golden also
        // pins the instance-named branch of `cache_control_for`. The
        // instance token is a fresh wall-clock-seeded number per `HlsOrigin`
        // build, so it is captured here and normalised out of both the URI
        // and the golden file: the golden pins the header SET, not one
        // random token.
        let ll_hls = store
            .ll_hls(crate::route::SPTS_PROGRAM_ID)
            .expect("program published");
        let instance = ll_hls.instance().to_string();
        let init_uri = format!("/cam1/{}", ll_hls.init_name(1, 1));
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

        let probes = [
            "/cam1/master.m3u8",
            "/cam1/media.m3u8",
            init_uri.as_str(),
            "/cam1/seg-1-1.m4s",
            "/cam1/manifest.mpd",
            "/metrics",
            "/healthz",
        ];
        // The instance token is a fresh wall-clock-seeded number per
        // `HlsOrigin` build, so replace it with a stable placeholder before
        // comparing: the golden pins the header SET, not one random token.
        let mut actual = String::new();
        for uri in probes {
            let resp = app.clone().oneshot(get(uri)).await.unwrap();
            let status = resp.status().as_u16();
            let uri = uri.replace(&instance, "{instance}");
            actual.push_str(&format!("{uri}\t{status}\n"));
            let mut names: Vec<String> = resp
                .headers()
                .keys()
                .map(|n| n.as_str().to_string())
                .collect();
            names.sort();
            for name in names {
                // `/metrics`' body (and therefore its `Content-Length`) is a
                // process-global counter exposition that other tests in this
                // binary also increment, so it is not golden-able; every
                // other header of every probe is.
                if uri == "/metrics" && name == "content-length" {
                    continue;
                }
                let value = resp
                    .headers()
                    .get(name.as_str())
                    .map(|v| v.to_str().unwrap_or("<binary>"))
                    .unwrap_or("");
                actual.push_str(&format!("{uri}\t{status}\t{name}: {value}\n"));
            }
        }

        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(
                std::path::Path::new(&dir).join("origin_response_headers.golden"),
                &actual,
            )
            .expect("write golden");
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/origin_response_headers.golden");
        let expected = std::fs::read_to_string(&path).expect("read origin header golden");
        assert_eq!(
            actual, expected,
            "origin response headers differ from the golden; every wire-visible \
             change must be listed in the multimux CHANGELOG"
        );
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
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x20; 8],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 1,
                },
            )
            .expect("add_segment");
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

    /// Audit run 7, B: a flood of LL-HLS blocking reloads (which park for up
    /// to the LL-HLS cap) must not starve an ordinary request to the SAME
    /// route. Driven with REAL HTTP clients against a REAL listener.
    ///
    /// The blocking-reload pool is separate from the ordinary pool, so with
    /// the ordinary bound at 1 a request to the same route still succeeds —
    /// which the pre-change per-route `ConcurrencyLimitLayer` could not do
    /// (its single per-route permit would be held by a parked reload).
    ///
    /// The reloads are confirmed to be PARKED before the ordinary request is
    /// sent (a handshake on the process-wide `ACTIVE_BLOCKING_REQUESTS`
    /// gauge), not by sleeping.
    #[tokio::test]
    async fn a_blocking_reload_flood_does_not_starve_ordinary_requests() {
        const BOUND: usize = 1;
        const FLOOD: usize = 8;
        let handle = crate::prometheus::install();
        let mut streams = HashMap::new();
        let store = Arc::new(RouteHandle::new(4.0, 500, 8));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x20; 8],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 1,
                },
            )
            .expect("add_segment");
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
        );
        let limits = HttpLimits {
            max_concurrent_requests: BOUND,
            queue_timeout: std::time::Duration::from_millis(50),
            ..HttpLimits::default()
        };
        let app = router(Arc::new(AppState::new(streams).with_limits(limits)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        // msn=2 is within the future bound of the current max (1), so these
        // are genuine WouldBlock reloads that park until the LL-HLS cap.
        let reload_url = format!("http://{addr}/cam1/media.m3u8?_HLS_msn=2&_HLS_part=0");
        let floods = (0..FLOOD)
            .map(|_| {
                let c = client.clone();
                let u = reload_url.clone();
                tokio::spawn(async move { c.get(u).send().await.map(|r| r.status()) })
            })
            .collect::<Vec<_>>();

        // HANDSHAKE: wait until at least one reload is genuinely parked
        // (the gauge is bumped by the blocking-request guard), bounded so a
        // regression fails rather than hangs.
        let parked = |handle: &metrics_exporter_prometheus::PrometheusHandle| -> bool {
            handle.render().lines().any(|l| {
                l.starts_with(crate::prometheus::ACTIVE_BLOCKING_REQUESTS)
                    && l.rsplit(' ')
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                        .is_some_and(|v| v >= 1.0)
            })
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if parked(&handle) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no blocking reload ever parked (the handshake never completed)"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // The ordinary request to the SAME route must still be served.
        let ordinary = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.get(format!("http://{addr}/cam1/media.m3u8")).send(),
        )
        .await
        .expect("the ordinary request must not hang")
        .expect("ordinary request");
        assert_eq!(
            ordinary.status(),
            reqwest::StatusCode::OK,
            "an ordinary request to the same route must not be starved by a blocking-reload flood"
        );

        for h in floods {
            h.abort();
        }
        server.abort();
    }

    /// Audit run 7, B: a flood beyond a pool's bound must shed the excess with
    /// `503` (with `Retry-After`), over REAL HTTP. The blocking-reload pool is
    /// `bound / DEFAULT_BLOCKING_RELOAD_DIVISOR` (here 1), so a burst of
    /// concurrent reloads must produce at least one `503` while at least one
    /// request is still admitted.
    #[tokio::test]
    async fn a_reload_flood_sheds_the_excess_with_503() {
        const BOUND: usize = 4;
        let mut streams = HashMap::new();
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        store.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        store.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 4]);
        store
            .add_segment(
                crate::route::SPTS_PROGRAM_ID,
                transmux::ll_hls::SegmentInfo {
                    bytes: vec![0x20; 8],
                    duration: 4.0,
                    segment_seq: 1,
                    part_count: 1,
                },
            )
            .expect("add_segment");
        streams.insert(
            "cam1".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
            ),
        );
        let limits = HttpLimits {
            max_concurrent_requests: BOUND,
            queue_timeout: std::time::Duration::from_millis(50),
            ..HttpLimits::default()
        };
        let app = router(Arc::new(AppState::new(streams).with_limits(limits)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        let url = format!("http://{addr}/cam1/media.m3u8?_HLS_msn=2&_HLS_part=0");
        let floods = (0..12)
            .map(|_| {
                let c = client.clone();
                let u = url.clone();
                tokio::spawn(async move { c.get(u).send().await })
            })
            .collect::<Vec<_>>();

        let mut shed = 0usize;
        let mut ok = 0usize;
        for h in floods {
            // Bounded so a regression fails rather than hangs (a parked
            // reload still completes by the LL-HLS cap, so 10 s is generous).
            let resp = tokio::time::timeout(std::time::Duration::from_secs(10), h)
                .await
                .expect("every reload request must complete within the guard")
                .expect("join")
                .expect("request completes");
            match resp.status() {
                reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                    assert!(
                        resp.headers().contains_key(reqwest::header::RETRY_AFTER),
                        "a shed 503 must carry Retry-After"
                    );
                    shed += 1;
                }
                // A parked reload that never sees the requested segment
                // falls back to a normal response (a 404 here, or a 200 if
                // a segment arrived) — neither is a shed.
                reqwest::StatusCode::OK | reqwest::StatusCode::NOT_FOUND => ok += 1,
                other => panic!("unexpected status {other}"),
            }
        }
        assert!(
            shed >= 1,
            "a reload flood past the reload budget must shed at least one 503  \
            (ok={ok}, shed={shed})"
        );

        server.abort();
    }

    /// Item 7: `HttpLimits::from` must never panic on an unvalidated
    /// `Config` — a NaN/negative/overflowing timeout falls back to the
    /// default rather than `Duration::from_secs_f64`'s panic.
    ///
    /// Biting test: use `Duration::from_secs_f64` directly and a NaN
    /// `request_timeout_secs` panics.
    #[test]
    fn http_limits_from_an_unvalidated_config_does_not_panic() {
        for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0, f64::MAX] {
            let cfg = crate::config::Config {
                request_timeout_secs: bad,
                concurrency_queue_timeout_secs: bad,
                ..crate::config::Config::default()
            };
            let limits = HttpLimits::from(&cfg);
            assert!(
                limits.request_timeout > std::time::Duration::ZERO,
                "a bad request_timeout ({bad}) must fall back to a positive default"
            );
            assert!(
                limits.queue_timeout > std::time::Duration::ZERO,
                "a bad queue_timeout ({bad}) must fall back to a positive default"
            );
        }
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

    /// Biting test for `output_auth_gate` switching from `Verifier::challenge`
    /// to `Verifier::challenge_for`: a request that correctly answers a
    /// Digest nonce which has since expired must `401` with a
    /// `WWW-Authenticate` that carries `stale=true` (RFC 7616 §3.3), not a
    /// bare fresh challenge — using `challenge()` here (as this middleware
    /// did before) would drop `stale=true`, and a compliant client only
    /// retries silently (without re-prompting for credentials) when it sees
    /// that flag. Drives the real production `output_auth_gate` middleware
    /// with a `Verifier::with_clock`-controlled clock, exactly like
    /// `broadcast_auth::server`'s own
    /// `digest_expired_nonce_is_rejected_with_stale_challenge` unit test.
    #[tokio::test]
    async fn output_auth_digest_expired_nonce_gets_stale_challenge() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let now = Arc::new(AtomicU64::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ));
        let clock_now = Arc::clone(&now);
        let verifier = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "hunter2".into(),
            },
            OUTPUT_AUTH_REALM,
        )
        .with_clock(move || UNIX_EPOCH + Duration::from_secs(clock_now.load(Ordering::SeqCst)));

        let challenge = verifier.challenge();
        let authorization = respond(
            &challenge,
            &RequestContext::new("GET", "/cam1/master.m3u8"),
            Credentials::new("admin", "hunter2"),
        )
        .unwrap();

        // Advance the clock past the nonce lifetime before the client's
        // (otherwise-correct) answer ever reaches the server.
        now.fetch_add(
            broadcast_auth::DIGEST_NONCE_LIFETIME.as_secs() + 1,
            Ordering::SeqCst,
        );

        let app = app_with_output_auth(verifier);
        let resp = app
            .oneshot(get_with_auth("/cam1/master.m3u8", &authorization))
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        let challenge = resp
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .expect("401 must carry WWW-Authenticate")
            .to_str()
            .unwrap();
        assert!(
            challenge.ends_with(", stale=true"),
            "expired-but-correctly-answered nonce must be flagged stale: {challenge}"
        );
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

    /// Audit run 7, W18: a real cross-origin preflight for a request that
    /// sends `Authorization` must be answered with an
    /// `Access-Control-Allow-Headers` that names it — the Fetch spec's `*`
    /// wildcard never covers `Authorization`, so the pre-fix header failed the
    /// preflight for every browser player using output auth.
    #[tokio::test]
    async fn cors_preflight_allows_authorization_explicitly() {
        let app = app_with_output_auth(Verifier::new(
            Credentials::bearer("secrettoken"),
            OUTPUT_AUTH_REALM,
        ));
        let req = Request::builder()
            .method("OPTIONS")
            .uri("/cam1/master.m3u8")
            .header("Origin", "https://player.example")
            .header("Access-Control-Request-Method", "GET")
            .header("Access-Control-Request-Headers", "authorization")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let headers = resp.headers();
        let allow = headers
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            allow.to_ascii_lowercase().contains("authorization"),
            "the preflight must name Authorization: {allow:?}"
        );
        // `Vary: Origin` so a shared cache never reuses a CORS-wrong response.
        assert_eq!(
            headers
                .get(axum::http::header::VARY)
                .and_then(|v| v.to_str().ok()),
            Some("Origin")
        );
        // The metadata a media client needs is exposed. Header NAMES are
        // case-insensitive (RFC 9110 §5.1) and `headers` renders them
        // lowercased, so compare case-insensitively.
        let expose = headers
            .get(axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        for want in ["content-length", "content-range", "etag"] {
            assert!(expose.contains(want), "must expose {want}: {expose}");
        }
        // HEAD is allowed (a client may probe a segment's size).
        let methods = headers
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_METHODS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(methods.contains("HEAD"), "HEAD must be allowed: {methods}");
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
                    // Hold the shutdown token alive until told to stop,
                    // mirroring a real supervised connector task.
                    ctx.cancel.cancelled().await;
                }))
            }),
        );

        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        let cancel = tokio_util::sync::CancellationToken::new();
        let factory = registry.input("silence").expect("factory registered above");
        let handle = factory(crate::registry::InputCtx {
            name: "cam1".into(),
            params: serde_json::json!({"marker": "ok"}),
            store: store.clone(),
            target_duration_secs: 4.0,
            part_target_ms: 500,
            cancel,
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
                    socket: Default::default(),
                },
                outputs: vec![crate::output::OutputKind::LlHls],
                dvr: DvrConfig::default(),
            }],
            bind: "127.0.0.1:0".into(),
            ..crate::config::Config::default()
        };
        let store = Arc::new(RouteHandle::new(4.0, 500, 4));
        let cancel = tokio_util::sync::CancellationToken::new();
        let handle = spawn_ingest(
            &cfg.routes[0],
            store.clone(),
            &cfg,
            &SchemeRegistry::new(),
            cancel,
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
            crate::config::InputSpec::File { .. } => true,
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
                socket: Default::default(),
            },
            crate::config::InputSpec::TsUdp {
                addr: quiet_ts_udp_addr.to_string(),
                multicast_group: None,
                socket: Default::default(),
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

        let cancel = tokio_util::sync::CancellationToken::new();
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

            let handle = spawn_ingest(&route, store.clone(), &config, &registry, cancel.clone())
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

    /// Audit r07-C10 (#1083): `track_http` must not collect the response
    /// body. The first chunk of a streamed response reaches the client while
    /// the producer is still holding the rest back (an in-progress LL-DASH
    /// segment); the buffering middleware never returned the response at all
    /// until the stream closed, so this times out against the old code. The
    /// bytes counter still ends up exact.
    #[tokio::test]
    async fn track_http_streams_the_body_and_counts_every_byte() {
        let state = state_with_stream("c10-stream");
        let (tx, body) = channel_body();
        let body = Arc::new(std::sync::Mutex::new(Some(body)));
        let app = Router::new()
            .route(
                "/c10-stream/seg-1-1.m4s",
                axum::routing::get(move || {
                    let body = body.lock().unwrap().take().expect("one request");
                    async move { body }
                }),
            )
            .layer(middleware::from_fn_with_state(state.clone(), track_http));
        let before = bytes_served(&state, "c10-stream", "segment");

        tx.send(axum::body::Bytes::from_static(b"first"))
            .await
            .unwrap();
        let resp = tokio::time::timeout(
            Duration::from_secs(10),
            app.oneshot(get("/c10-stream/seg-1-1.m4s")),
        )
        .await
        .expect("the response must be returned while its body is still open")
        .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut stream = resp.into_body().into_data_stream();
        let first = tokio::time::timeout(
            Duration::from_secs(10),
            futures_util::StreamExt::next(&mut stream),
        )
        .await
        .expect("first chunk must arrive before the second is produced")
        .expect("a chunk")
        .unwrap();
        assert_eq!(&first[..], b"first");

        tx.send(axum::body::Bytes::from_static(b"-second"))
            .await
            .unwrap();
        drop(tx);
        let rest = tokio::time::timeout(Duration::from_secs(10), async {
            let mut all = Vec::new();
            while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                all.extend_from_slice(&chunk.unwrap());
            }
            all
        })
        .await
        .expect("body must end once the producer is done");
        assert_eq!(rest, b"-second");
        drop(stream);
        assert_eq!(
            bytes_served(&state, "c10-stream", "segment") - before,
            12,
            "5 + 7 bytes were sent"
        );
    }

    /// A fully buffered body keeps the `Content-Length` hyper used to derive
    /// from it, even though the body is now re-wrapped as a stream.
    #[tokio::test]
    async fn track_http_keeps_content_length_of_a_sized_body() {
        let state = state_with_stream("c10-sized");
        let app = Router::new()
            .route("/c10-sized/x", axum::routing::get(|| async { "hello" }))
            .layer(middleware::from_fn_with_state(state, track_http));
        let resp = app.oneshot(get("/c10-sized/x")).await.unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_LENGTH),
            Some(&HeaderValue::from_static("5"))
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"hello");
    }

    /// Audit r07-C4 (#1083): an egress bound to the route's first `Trunk`
    /// must follow a source reconnect to the replacement `Trunk` — cancelled
    /// on the old one (its token fires) and started again on the new one —
    /// instead of draining a dead ring forever. Against the old
    /// `await_first_trunk`-once wiring the second start never happens.
    #[tokio::test]
    async fn egress_follows_a_replaced_trunk() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        let cancel = tokio_util::sync::CancellationToken::new();
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stopped_tx, mut stopped_rx) = tokio::sync::mpsc::unbounded_channel();
        let follower = tokio::spawn(follow_trunk(Arc::clone(&route), cancel.clone(), {
            move |trunk, token| {
                let (started_tx, stopped_tx) = (started_tx.clone(), stopped_tx.clone());
                async move {
                    started_tx.send(trunk).unwrap();
                    token.cancelled().await;
                    stopped_tx.send(()).unwrap();
                }
            }
        }));
        let wait = |secs| Duration::from_secs(secs);

        // Nothing published yet: nothing started.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), started_rx.recv())
                .await
                .is_err()
        );

        let first = route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        let bound = tokio::time::timeout(wait(10), started_rx.recv())
            .await
            .expect("egress starts once a Trunk exists")
            .unwrap();
        assert!(Arc::ptr_eq(&bound, &first));

        // The source reconnects: its session is reaped (releasing the
        // publisher slot) and a fresh Trunk is published over the program.
        route.release_program(crate::route::SPTS_PROGRAM_ID, &first);
        let second = route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        assert!(!Arc::ptr_eq(&first, &second));

        tokio::time::timeout(wait(10), stopped_rx.recv())
            .await
            .expect("the egress on the dead Trunk is cancelled")
            .unwrap();
        let rebound = tokio::time::timeout(wait(10), started_rx.recv())
            .await
            .expect("egress restarts on the replacement Trunk")
            .unwrap();
        assert!(Arc::ptr_eq(&rebound, &second));

        // Route shutdown still ends the follower, and the running egress.
        cancel.cancel();
        tokio::time::timeout(wait(10), follower)
            .await
            .expect("follower ends on cancel")
            .unwrap();
        tokio::time::timeout(wait(10), stopped_rx.recv())
            .await
            .expect("the egress is cancelled with the route")
            .unwrap();
    }

    /// An egress that ends by itself ends the follower too (a permanently
    /// failed push must not be restarted in a loop), and cancelling before
    /// any `Trunk` exists returns without starting anything.
    #[tokio::test]
    async fn follower_ends_when_the_egress_ends_or_the_route_is_cancelled() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        tokio::time::timeout(
            Duration::from_secs(10),
            follow_trunk(
                Arc::clone(&route),
                tokio_util::sync::CancellationToken::new(),
                move |_, _| {
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async {}
                },
            ),
        )
        .await
        .expect("an egress that returns ends the follower");
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);

        let empty = Arc::new(RouteHandle::new(4.0, 500, 4));
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        tokio::time::timeout(
            Duration::from_secs(10),
            follow_trunk(empty, cancel, |_, _| async { panic!("must not start") }),
        )
        .await
        .expect("cancelled before any Trunk exists");
    }

    /// Audit r07-C2 (#1030): `immutable` is for URIs that can never map to
    /// other bytes — not the bare current-init name, and not an error. The
    /// expected value is the RENDERED header (the typed `headers::CacheControl`
    /// serialises the directives in its own canonical order).
    #[test]
    fn cache_control_is_immutable_only_for_names_that_cannot_change() {
        use axum::http::StatusCode as S;
        let ok = S::OK;
        let render = |path: &str, status| {
            let mut map = axum::http::HeaderMap::new();
            headers::HeaderMapExt::typed_insert(&mut map, cache_control_for(path, status));
            map.get(axum::http::header::CACHE_CONTROL)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        };
        for (path, status, expected) in [
            (
                "/cam1/seg-1-1790000000000-7.m4s",
                ok,
                CACHE_CONTROL_IMMUTABLE,
            ),
            (
                "/cam1/part-1-1790000000000-7.2.m4s",
                ok,
                CACHE_CONTROL_IMMUTABLE,
            ),
            (
                "/cam1/seg-1-1790000000000-7.ts",
                ok,
                CACHE_CONTROL_IMMUTABLE,
            ),
            (
                "/cam1/init-1-1790000000000-3.mp4",
                ok,
                CACHE_CONTROL_IMMUTABLE,
            ),
            ("/cam1/seg-1-7.m4s", ok, CACHE_CONTROL_SHORT),
            ("/cam1/part-1-7.2.m4s", ok, CACHE_CONTROL_SHORT),
            ("/cam1/catchup/seg-12.m4s", ok, CACHE_CONTROL_SHORT),
            ("/cam1/init-1-3.mp4", ok, CACHE_CONTROL_MANIFEST),
            ("/cam1/init-1.mp4", ok, CACHE_CONTROL_MANIFEST),
            ("/cam1/media.m3u8", ok, CACHE_CONTROL_MANIFEST),
            ("/cam1/manifest.mpd", ok, CACHE_CONTROL_MANIFEST),
            (
                "/cam1/seg-1-1790000000000-7.m4s",
                S::NOT_FOUND,
                CACHE_CONTROL_MANIFEST,
            ),
            (
                "/cam1/part-1-7.2.m4s",
                S::SERVICE_UNAVAILABLE,
                CACHE_CONTROL_MANIFEST,
            ),
            (
                "/cam1/seg-1-1790000000000-7.m4s",
                S::UNAUTHORIZED,
                CACHE_CONTROL_MANIFEST,
            ),
            ("/cam1/something-else.bin", ok, CACHE_CONTROL_MANIFEST),
        ] {
            assert_eq!(render(path, status), expected, "{path} {status}");
        }
    }

    /// A client that disconnects mid-stream: the bytes already sent are
    /// counted exactly once when the body is dropped, never lost and never
    /// counted twice.
    #[tokio::test]
    async fn dropping_the_body_mid_stream_counts_the_bytes_sent_exactly_once() {
        let state = state_with_stream("c10-drop");
        let (tx, body) = channel_body();
        let body = Arc::new(std::sync::Mutex::new(Some(body)));
        let app = Router::new()
            .route(
                "/c10-drop/seg-1-1.m4s",
                axum::routing::get(move || {
                    let body = body.lock().unwrap().take().expect("one request");
                    async move { body }
                }),
            )
            .layer(middleware::from_fn_with_state(state.clone(), track_http));
        let before = bytes_served(&state, "c10-drop", "segment");

        tx.send(axum::body::Bytes::from_static(b"seven-b"))
            .await
            .unwrap();
        let resp = app.oneshot(get("/c10-drop/seg-1-1.m4s")).await.unwrap();
        let mut stream = resp.into_body().into_data_stream();
        let first = tokio::time::timeout(
            Duration::from_secs(10),
            futures_util::StreamExt::next(&mut stream),
        )
        .await
        .expect("first chunk")
        .expect("a chunk")
        .unwrap();
        assert_eq!(first.len(), 7);
        // The producer is still open (`tx` alive): the client just leaves.
        drop(stream);
        assert_eq!(bytes_served(&state, "c10-drop", "segment") - before, 7);
        // Late data into a closed channel changes nothing.
        let _ = tx.send(axum::body::Bytes::from_static(b"late")).await;
        assert_eq!(bytes_served(&state, "c10-drop", "segment") - before, 7);
    }
}
