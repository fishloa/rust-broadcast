//! A **global** concurrent-request bound with a bounded queue wait (audit
//! run 7, W4).
//!
//! [`tower::limit::ConcurrencyLimitLayer`] cannot express this: `Router::layer`
//! calls `endpoint.layer(layer.clone())` per *route* (axum 0.7.9
//! `routing/path_router.rs`), so each endpoint builds its own `Semaphore`.
//! The documented bound ("across every route", issue #663 P5) was really
//! `bound × routes × methods × streams`. Worse, the per-route limit sat
//! *inside* `TimeoutLayer`, so a request waiting for a permit was never
//! subject to the request timeout — a slow-loris could park an unbounded
//! number of tasks indefinitely.
//!
//! [`GlobalLimit`] owns one `Arc<Semaphore>` for the whole server and is
//! applied once, around the finished service. A request that cannot acquire
//! a permit within the queue timeout is answered `503 Service Unavailable`
//! (RFC 9110 §15.6.4, "temporary overloading") with a `Retry-After` header
//! (RFC 9110 §10.2.3) rather than waiting forever — the timeout applies to
//! the *queue* wait, exactly the case the old per-route limit left unbounded.
//!
//! # Three separate budgets (audit run 7, B)
//!
//! A single pool cannot serve an HTTP origin with long-polls in it:
//!
//! - **Ops routes** (`/healthz`, `/readyz`, `/metrics`) are *exempt*: they
//!   must answer `200` even while the pool is saturated, or an orchestrator's
//!   liveness probe fails and the process is restarted — turning a load spike
//!   into an outage. The admin API is a separate listener with its own router
//!   and never passes through here at all.
//! - **Blocking-reload** requests (LL-HLS `_HLS_msn`/`_HLS_part`,
//!   RFC 8216bis §6.2.5.2) get their own pool. A blocking reload legitimately
//!   holds its connection up to the LL-HLS cap, so a few thousand viewers can
//!   occupy every main permit; with a shared pool they would then `503` every
//!   ordinary manifest/segment request too. Separating the two means a reload
//!   flood starves only other reloads.
//! - **Ordinary** requests share [`GlobalLimit::max_concurrent`].

use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::{Layer, Service};

pub(crate) use crate::prometheus::HTTP_SHED_TOTAL;

/// Default queue wait before a request with no free permit is shed — see
/// [`GlobalLimit::with_queue_timeout`]. Deliberately short: the point of the
/// bound is to shed load a slow-loris is trying to accumulate, so waiting long
/// enough to defeat that defeats the limit too.
pub const DEFAULT_QUEUE_TIMEOUT: Duration = Duration::from_secs(5);

/// Default size of the blocking-reload budget, as a divisor of the main
/// bound. Blocking reloads hold a permit for up to the LL-HLS cap, so they
/// need a *smaller*, separately-bounded pool than the burst capacity ordinary
/// requests want.
pub const DEFAULT_BLOCKING_RELOAD_DIVISOR: usize = 4;

/// `Retry-After` (RFC 9110 §10.2.3) sent with a shed `503`, in seconds. One
/// second: retry soon, but not immediately, or the shed becomes a hot loop.
pub const RETRY_AFTER_SECS: u32 = 1;

/// The `Retry-After` value as a static header value.
const RETRY_AFTER: HeaderValue = HeaderValue::from_static("1");

/// Which pool a request draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Budget {
    /// Not limited at all: the ops/supervision endpoints.
    Exempt,
    /// An LL-HLS blocking-reload long-poll.
    BlockingReload,
    /// Everything else.
    Ordinary,
}

impl Budget {
    fn label(self) -> &'static str {
        match self {
            Budget::Exempt => "exempt",
            Budget::BlockingReload => "blocking_reload",
            Budget::Ordinary => "ordinary",
        }
    }
}

/// The ops/supervision endpoints an orchestrator probes. These answer even
/// while every data permit is held — a `503` here means a container restart,
/// which turns a load spike into an outage.
const OPS_PATHS: [&str; 3] = ["/healthz", "/readyz", "/metrics"];

/// Classify a request by path and query. Kept deliberately small and
/// allocation-free on the hot path (no query string → no scan).
fn classify(uri: &axum::http::Uri) -> Budget {
    if OPS_PATHS.contains(&uri.path()) {
        return Budget::Exempt;
    }
    // An LL-HLS blocking reload carries `_HLS_msn` and/or `_HLS_part`
    // (RFC 8216bis §6.2.5.2). Only a request that would actually block counts
    // — a plain manifest fetch has no such query and is ordinary.
    if let Some(q) = uri.query()
        && (q.contains("_HLS_msn") || q.contains("_HLS_part"))
    {
        return Budget::BlockingReload;
    }
    Budget::Ordinary
}

/// A clonable, server-wide concurrency bound with three budgets (see the
/// module docs). Cheap to clone (`Arc`), so it can be installed on one shared
/// service and also handed to a per-connection factory that must share the
/// *same* pools.
#[derive(Clone)]
pub struct GlobalLimit {
    permits: Arc<Semaphore>,
    reload_permits: Arc<Semaphore>,
    queue_timeout: Duration,
    max_concurrent: usize,
}

impl GlobalLimit {
    /// A limit admitting `max_concurrent` ordinary requests at once, with the
    /// blocking-reload pool sized at a [`DEFAULT_BLOCKING_RELOAD_DIVISOR`] of
    /// that. Values `>= 1` are used as-is; `0` is meaningless for `Semaphore`
    /// (every request would be rejected) so it is clamped up to `1` — a config
    /// that asks for "no concurrency" still has to serve the request it is
    /// currently handling.
    pub fn new(max_concurrent: usize) -> Self {
        let max_concurrent = max_concurrent.max(1);
        GlobalLimit {
            permits: Arc::new(Semaphore::new(max_concurrent)),
            reload_permits: Arc::new(Semaphore::new(
                (max_concurrent / DEFAULT_BLOCKING_RELOAD_DIVISOR).max(1),
            )),
            queue_timeout: DEFAULT_QUEUE_TIMEOUT,
            max_concurrent,
        }
    }

    /// Override the blocking-reload pool size explicitly.
    pub fn with_blocking_reload_budget(mut self, budget: usize) -> Self {
        self.reload_permits = Arc::new(Semaphore::new(budget.max(1)));
        self
    }

    /// Override the queue wait (see [`DEFAULT_QUEUE_TIMEOUT`]).
    pub fn with_queue_timeout(mut self, timeout: Duration) -> Self {
        self.queue_timeout = timeout;
        self
    }

    /// The ordinary bound this limit was built with — for diagnostics.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// The layer form, for wrapping a whole finished service once.
    pub fn layer(self) -> GlobalLimitLayer {
        GlobalLimitLayer { limit: self }
    }

    /// A permit for `budget`, waiting at most `queue_timeout`. `None` means
    /// the wait elapsed — the caller answers `503` (`Budget::Exempt` is never
    /// limited at all, so it returns `Some(None)`).
    async fn acquire(&self, budget: Budget) -> Option<Option<Permit>> {
        let semaphore = match budget {
            Budget::Exempt => return Some(None),
            Budget::BlockingReload => &self.reload_permits,
            Budget::Ordinary => &self.permits,
        };
        match tokio::time::timeout(self.queue_timeout, Arc::clone(semaphore).acquire_owned()).await
        {
            Ok(Ok(permit)) => Some(Some(permit)),
            Ok(Err(_closed)) => None,
            Err(_elapsed) => None,
        }
    }
}

/// [`tower::Layer`] producing a [`GlobalLimitService`] that shares one set of
/// semaphores across every clone of the produced service.
///
/// This is what makes the bound **global** even though [`axum::Router::layer`]
/// clones the layer once per endpoint (`routing/path_router.rs:252-267`):
/// every clone built from one `GlobalLimitLayer` clones the same
/// `Arc<Semaphore>`, so the total in-flight count across every route, method
/// and stream is bounded by one semaphore — which is exactly what
/// [`tower::limit::ConcurrencyLimitLayer`] could not do (it owns the
/// `Semaphore` directly, so each per-endpoint clone built a fresh one).
#[derive(Clone)]
pub struct GlobalLimitLayer {
    limit: GlobalLimit,
}

impl GlobalLimitLayer {
    /// A global bound with the given limits.
    pub fn new(limit: GlobalLimit) -> Self {
        GlobalLimitLayer { limit }
    }
}

impl<S> Layer<S> for GlobalLimitLayer {
    type Service = GlobalLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GlobalLimitService {
            inner,
            limit: self.limit.clone(),
        }
    }
}

/// A service that admits at most `max_concurrent` in-flight ordinary requests
/// (plus a separate blocking-reload budget), queueing further ones for at
/// most the queue timeout, and never limiting the ops endpoints.
#[derive(Clone)]
pub struct GlobalLimitService<S> {
    inner: S,
    limit: GlobalLimit,
}

impl<S> GlobalLimitService<S> {
    /// Wrap `inner` with the given server-wide bound.
    pub fn new(inner: S, limit: GlobalLimit) -> Self {
        GlobalLimitService { inner, limit }
    }
}

/// A permit held for the duration of one in-flight request. Dropped once the
/// inner service has produced its response.
type Permit = OwnedSemaphorePermit;

impl<S, B> Service<axum::http::Request<B>> for GlobalLimitService<S>
where
    S: Service<axum::http::Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Response, S::Error>> + Send + 'static>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<B>) -> Self::Future {
        let limit = self.limit.clone();
        let budget = classify(req.uri());
        // A clone of `inner`, so `self` stays intact for the next call
        // (`Router` and every service this wraps is `Clone`).
        let mut inner = self.inner.clone();
        Box::pin(async move {
            let permit = match limit.acquire(budget).await {
                Some(permit) => permit,
                None => {
                    // Queue wait elapsed: shed the request rather than park
                    // it, and record it — a shed request never reaches
                    // `track_http` (layered inside this), so without this
                    // counter the `503`s are invisible.
                    metrics::counter!(HTTP_SHED_TOTAL, "kind" => budget.label()).increment(1);
                    return Ok(overloaded());
                }
            };
            let result = inner.call(req).await;
            // The permit is held until `inner` resolves. In this origin
            // `inner` is `track_http`, which itself collects the whole
            // response body (`axum::body::to_bytes`) before returning — so
            // the permit genuinely covers the *entire* response, body
            // included, not merely the headers. A future reordering that
            // moved body production outside this layer would need to carry
            // the permit into the body stream instead (see `origin::mod`'s
            // `router` doc).
            drop(permit);
            result
        })
    }
}

/// `503` answer when the queue wait elapses with no permit free, carrying a
/// `Retry-After` (RFC 9110 §10.2.3).
fn overloaded() -> Response {
    let mut resp = (StatusCode::SERVICE_UNAVAILABLE, "server overloaded").into_response();
    resp.headers_mut().insert(header::RETRY_AFTER, RETRY_AFTER);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn uri(s: &str) -> axum::http::Uri {
        s.parse().expect("uri")
    }

    /// A bound of N admits exactly N concurrent requests; the N+1th waits,
    /// and (with a short queue timeout) is shed as `None`.
    #[tokio::test]
    async fn a_full_queue_is_rejected_after_the_timeout() {
        const BOUND: usize = 2;
        const QUEUE_WAIT: Duration = Duration::from_millis(50);
        let limit = GlobalLimit::new(BOUND).with_queue_timeout(QUEUE_WAIT);

        let first = limit.acquire(Budget::Ordinary).await;
        let second = limit.acquire(Budget::Ordinary).await;
        assert!(first.is_some(), "permit 1 of {BOUND} must be admitted");
        assert!(second.is_some(), "permit 2 of {BOUND} must be admitted");

        // The third has no permit free — it waits the full queue timeout and
        // is rejected with `None` (never parked indefinitely).
        let started = std::time::Instant::now();
        let third = limit.acquire(Budget::Ordinary).await;
        assert!(third.is_none(), "the 3rd request must be shed, not queued");
        assert!(
            started.elapsed() >= QUEUE_WAIT,
            "the shed must have waited the full queue timeout, waited {:?}",
            started.elapsed()
        );

        // Releasing one permit lets the next request through.
        drop(first);
        let fourth = limit.acquire(Budget::Ordinary).await;
        assert!(fourth.is_some(), "a freed permit must be reused");
    }

    /// Ops routes are never limited: they classify as `Exempt`.
    #[tokio::test]
    async fn ops_paths_are_exempt_from_the_bound() {
        assert_eq!(classify(&uri("/healthz")), Budget::Exempt);
        assert_eq!(classify(&uri("/readyz")), Budget::Exempt);
        assert_eq!(classify(&uri("/metrics")), Budget::Exempt);
        // A stream named `healthz` is NOT the ops endpoint — it is under a
        // `/{stream}/` prefix.
        assert_eq!(classify(&uri("/cam1/healthz")), Budget::Ordinary);
        // And an exempt classification never blocks, even with the pool full.
        let limit = GlobalLimit::new(1).with_queue_timeout(Duration::from_millis(1));
        let _held = limit.acquire(Budget::Ordinary).await.expect("free");
        let exempt = limit.acquire(Budget::Exempt).await;
        assert!(exempt.is_some(), "ops requests are never limited");
    }

    /// A blocking reload draws from its own pool, so saturating it does not
    /// touch the ordinary pool (and vice versa).
    #[tokio::test]
    async fn blocking_reloads_have_their_own_pool() {
        // Bound 2 => reload pool 1. Exhaust the ordinary pool entirely, then
        // show a reload still gets through (they are separate pools).
        let limit = GlobalLimit::new(2).with_blocking_reload_budget(1);
        let a = limit.acquire(Budget::Ordinary).await;
        let b = limit.acquire(Budget::Ordinary).await;
        assert!(a.is_some() && b.is_some(), "both ordinary permits are free");
        // Ordinary pool is now full.
        let reload = limit.acquire(Budget::BlockingReload).await;
        assert!(
            reload.is_some(),
            "a reload must be admitted even with the ordinary pool exhausted  \
            (separate pools)"
        );
        // And the reload budget is its own: a second reload is shed while the
        // first still holds it, without touching the ordinary pool.
        let limit2 = GlobalLimit::new(2).with_blocking_reload_budget(1);
        let r1 = limit2.acquire(Budget::BlockingReload).await;
        assert!(r1.is_some());
        let r2 = limit2
            .with_queue_timeout(Duration::from_millis(10))
            .acquire(Budget::BlockingReload)
            .await;
        assert!(r2.is_none(), "the reload pool is its own bound");
    }

    /// A blocking-reload request is classified by its `_HLS_msn`/`_HLS_part`
    /// query, not by path.
    #[test]
    fn blocking_reload_query_classifies_as_blocking() {
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?_HLS_msn=5")),
            Budget::BlockingReload
        );
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?_HLS_msn=5&_HLS_part=2")),
            Budget::BlockingReload
        );
        assert_eq!(classify(&uri("/cam1/media.m3u8")), Budget::Ordinary);
    }

    /// The shed response carries a `Retry-After` header (RFC 9110 §10.2.3).
    #[test]
    fn a_shed_response_carries_retry_after() {
        let resp = overloaded();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
    }

    /// The permit is held until the inner service has produced its response
    /// (which, in this origin, includes collecting the whole body). A second
    /// request issued while the first is still being produced must therefore
    /// be shed — proving the permit covers body production, not just headers.
    ///
    /// The first request signals (over a `oneshot`) that it has ENTERED the
    /// inner service — i.e. it holds the permit — so the second request is
    /// sent only after that handshake, never after a sleep.
    #[tokio::test]
    async fn the_permit_is_held_until_the_response_is_produced() {
        const QUEUE_WAIT: Duration = Duration::from_millis(20);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let entered_tx = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
        let inner = tower::service_fn(move |_req| {
            let entered_tx = Arc::clone(&entered_tx);
            async move {
                // Signal once, on the first request: it now holds the permit
                // and is "producing its body".
                if let Some(tx) = entered_tx.lock().expect("tx").take() {
                    let _ = tx.send(());
                }
                // A response that never resolves.
                std::future::pending::<()>().await;
                #[allow(unreachable_code)]
                Ok::<_, std::convert::Infallible>(Response::new(axum::body::Body::empty()))
            }
        });
        let shared = GlobalLimit::new(1).with_queue_timeout(QUEUE_WAIT);
        let mut svc = GlobalLimitService::new(inner, shared.clone());
        let req = || {
            axum::http::Request::builder()
                .uri("/cam1/media.m3u8")
                .body(axum::body::Body::empty())
                .unwrap()
        };

        let first = tokio::spawn({
            let mut svc = svc.clone();
            async move { svc.call(req()).await }
        });
        // HANDSHAKE: the first request has entered the inner service (holds
        // the permit). Bounded so a regression fails rather than hangs.
        tokio::time::timeout(Duration::from_secs(10), entered_rx)
            .await
            .expect("the first request must reach the inner service")
            .expect("the handshake sender must not be dropped");

        // Second request: must be shed.
        let resp = tokio::time::timeout(Duration::from_secs(10), svc.call(req()))
            .await
            .expect("the second request must not hang")
            .expect("infallible");
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a second request must be shed while the first still holds its permit"
        );
        first.abort();
    }

    /// The service sheds with `503` once the ordinary pool is exhausted, and
    /// the shed request never reaches the inner service.
    #[tokio::test]
    async fn service_sheds_with_503_and_skips_the_inner_service() {
        const BOUND: usize = 1;
        const QUEUE_WAIT: Duration = Duration::from_millis(20);
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = tower::service_fn({
            let calls = Arc::clone(&calls);
            move |_req: axum::http::Request<axum::body::Body>| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok::<_, std::convert::Infallible>(Response::new(
                    axum::body::Body::empty(),
                )))
            }
        });
        let shared = GlobalLimit::new(BOUND).with_queue_timeout(QUEUE_WAIT);
        let mut svc = GlobalLimitService::new(inner, shared.clone());
        let req = || {
            axum::http::Request::builder()
                .uri("/cam1/media.m3u8")
                .body(axum::body::Body::empty())
                .unwrap()
        };

        let held = shared.acquire(Budget::Ordinary).await.expect("free");
        let resp = svc.call(req()).await.expect("infallible");
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a shed request never reaches inner"
        );

        drop(held);
        let resp = svc.call(req()).await.expect("infallible");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
