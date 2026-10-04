//! Three-budget concurrent-request classification (audit run 7, W4/B; SP2.5).
//!
//! [`tower::limit::ConcurrencyLimitLayer`] cannot express a **global** bound:
//! `Router::layer` calls `Layer::layer` once per *route*, so each route would
//! build its own `Semaphore` and the documented bound ("across every route",
//! issue #663 P5) would really be `bound × routes × methods × streams`. This
//! module therefore owns the pools itself, in [`BudgetLimitLayer`], and hands
//! every clone the same `Arc<Semaphore>`s.
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
//! - **Ordinary** requests share one pool.
//!
//! A request that cannot acquire a permit within the queue timeout is
//! answered `503 Service Unavailable` (RFC 9110 §15.6.4, "temporary
//! overloading") with a `Retry-After` header (RFC 9110 §10.2.3) rather than
//! waiting forever. The permit is pinned to the **response body**
//! ([`PermitBody`]), so it bounds the entire response — a streamed body
//! included, not merely the headers.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody};
use axum::http::{Request, Response, Uri};
use http_body::{Frame, SizeHint};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt as _;

pub(crate) use crate::prometheus::HTTP_SHED_TOTAL;

/// Default queue wait before a request with no free permit is shed —
/// deliberately short: the point of the bound is to shed load a slow-loris is
/// trying to accumulate, so waiting long enough to defeat that defeats the
/// limit too.
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
const RETRY_AFTER: axum::http::HeaderValue = axum::http::HeaderValue::from_static("1");

/// The ops/supervision endpoints an orchestrator probes. These answer even
/// while every data permit is held — a `503` here means a container restart,
/// which turns a load spike into an outage.
const OPS_PATHS: [&str; 3] = ["/healthz", "/readyz", "/metrics"];

/// Which pool a request draws from.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Budget {
    /// Not limited at all: the ops/supervision endpoints.
    Exempt,
    /// An LL-HLS blocking-reload long-poll.
    BlockingReload,
    /// Everything else.
    Ordinary,
}

impl Budget {
    /// A stable label for the `kind` metric label.
    pub fn label(self) -> &'static str {
        match self {
            Budget::Exempt => "exempt",
            Budget::BlockingReload => "blocking_reload",
            Budget::Ordinary => "ordinary",
        }
    }
}

/// The 3-budget classification (the one thing this module keeps): ops routes
/// are exempt; a request carrying an `_HLS_msn`/`_HLS_part` query KEY (typed
/// form parse, never a substring scan) is a blocking reload; the rest are
/// ordinary. Allocation-free when there is no query string.
#[doc(hidden)]
pub fn classify(uri: &Uri) -> Budget {
    if OPS_PATHS.contains(&uri.path()) {
        return Budget::Exempt;
    }
    let blocking = uri.query().is_some_and(|q| {
        url::form_urlencoded::parse(q.as_bytes()).any(|(k, _)| k == "_HLS_msn" || k == "_HLS_part")
    });
    if blocking {
        Budget::BlockingReload
    } else {
        Budget::Ordinary
    }
}

/// `503` + `Retry-After` (RFC 9110 §15.6.4 / §10.2.3) + the existing
/// `HTTP_SHED_TOTAL` counter (label `kind`). The mapping lives here because
/// `BudgetLimit::call` must return `Infallible`.
fn shed_into_response(budget: Budget) -> Response<Body> {
    metrics::counter!(HTTP_SHED_TOTAL, "kind" => budget.label()).increment(1);
    let mut resp =
        axum::response::IntoResponse::into_response(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    resp.headers_mut()
        .insert(axum::http::header::RETRY_AFTER, RETRY_AFTER);
    resp
}

/// The layer OWNS the two pools. `Router::layer` calls `Layer::layer` once
/// per route, so pools created inside `layer()` would be one pool per route
/// (no shared budget at all); the layer holds the `Arc<Semaphore>`s and each
/// produced service clones them — the same shape as tower's
/// `GlobalConcurrencyLimitLayer::with_semaphore`.
#[derive(Clone)]
pub struct BudgetLimitLayer {
    ordinary: Arc<Semaphore>,
    reload: Arc<Semaphore>,
    queue_timeout: Duration,
}

impl BudgetLimitLayer {
    /// A bound admitting `ordinary` ordinary requests and `reload` blocking
    /// reloads at once, with `queue_timeout` before either pool sheds.
    /// `0` is meaningless for `Semaphore` (every request would be rejected)
    /// so both are clamped up to `1`.
    pub fn new(ordinary: usize, reload: usize, queue_timeout: Duration) -> Self {
        Self {
            ordinary: Arc::new(Semaphore::new(ordinary.max(1))),
            reload: Arc::new(Semaphore::new(reload.max(1))),
            queue_timeout,
        }
    }
}

impl<S> tower::Layer<S> for BudgetLimitLayer {
    type Service = BudgetLimit<S>;
    fn layer(&self, inner: S) -> BudgetLimit<S> {
        BudgetLimit {
            inner,
            ordinary: Arc::clone(&self.ordinary),
            reload: Arc::clone(&self.reload),
            queue_timeout: self.queue_timeout,
        }
    }
}

/// The service [`BudgetLimitLayer`] produces: a classify + acquire wrapper
/// over `inner` sharing the layer's pools.
#[derive(Clone)]
pub struct BudgetLimit<S> {
    inner: S,
    ordinary: Arc<Semaphore>,
    reload: Arc<Semaphore>,
    queue_timeout: Duration,
}

/// A response body that owns a pool permit.
pub struct PermitBody {
    inner: Body,
    _permit: OwnedSemaphorePermit,
}

/// Marker extension: this response's body holds a permit.
#[derive(Clone)]
pub struct PermitHeld;

impl PermitBody {
    /// Wrap `resp`'s body in a [`PermitBody`] carrying `permit`, and record
    /// the [`PermitHeld`] marker in the response extensions.
    fn pin_permit(resp: Response<Body>, permit: OwnedSemaphorePermit) -> Response<Body> {
        let (mut parts, body) = resp.into_parts();
        parts.extensions.insert(PermitHeld);
        Response::from_parts(
            parts,
            Body::new(PermitBody {
                inner: body,
                _permit: permit,
            }),
        )
    }
}

impl HttpBody for PermitBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        // `axum::body::Body` is `Unpin`, so `PermitBody` is too.
        Pin::new(&mut self.get_mut().inner).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<S, B> tower::Service<Request<B>> for BudgetLimit<S>
where
    B: Send + 'static,
    S: tower::Service<Request<B>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let budget = classify(req.uri());
        let queue_timeout = self.queue_timeout;
        let pool = match budget {
            Budget::Exempt => {
                let mut svc = self.inner.clone();
                return Box::pin(async move { svc.ready().await?.call(req).await });
            }
            Budget::BlockingReload => Arc::clone(&self.reload),
            Budget::Ordinary => Arc::clone(&self.ordinary),
        };
        let mut svc = self.inner.clone();
        Box::pin(async move {
            let permit = match tokio::time::timeout(queue_timeout, pool.acquire_owned()).await {
                Ok(Ok(p)) => p,
                Ok(Err(_)) | Err(_) => return Ok(shed_into_response(budget)),
            };
            let resp = svc.ready().await?.call(req).await?;
            Ok(PermitBody::pin_permit(resp, permit))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::Service as _;

    fn uri(s: &str) -> Uri {
        s.parse().expect("uri")
    }

    /// Ops routes classify as `Exempt`; a stream named `healthz` is not.
    #[test]
    fn ops_paths_are_exempt_from_the_bound() {
        assert_eq!(classify(&uri("/healthz")), Budget::Exempt);
        assert_eq!(classify(&uri("/readyz")), Budget::Exempt);
        assert_eq!(classify(&uri("/metrics")), Budget::Exempt);
        assert_eq!(classify(&uri("/cam1/healthz")), Budget::Ordinary);
    }

    /// A blocking-reload request is classified by its `_HLS_msn`/`_HLS_part`
    /// query KEY, not a substring anywhere in the query.
    #[test]
    fn blocking_reload_query_classifies_as_blocking_by_key() {
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?_HLS_msn=5")),
            Budget::BlockingReload
        );
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?_HLS_msn=5&_HLS_part=2")),
            Budget::BlockingReload
        );
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?_HLS_part=2")),
            Budget::BlockingReload
        );
        assert_eq!(classify(&uri("/cam1/media.m3u8")), Budget::Ordinary);
        // A value that merely CONTAINS the marker is not a key.
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?_HLS_msnx=1")),
            Budget::Ordinary
        );
        assert_eq!(
            classify(&uri("/cam1/media.m3u8?file=_HLS_msn")),
            Budget::Ordinary
        );
    }

    /// The shed response carries a `Retry-After` header (RFC 9110 §10.2.3).
    #[test]
    fn a_shed_response_carries_retry_after() {
        let resp = shed_into_response(Budget::Ordinary);
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
    }

    /// The service sheds with `503` once the ordinary pool is exhausted, the
    /// shed request never reaches the inner service, and a freed permit lets
    /// the next request through.
    #[tokio::test]
    async fn service_sheds_with_503_and_skips_the_inner_service() {
        const BOUND: usize = 1;
        const QUEUE_WAIT: Duration = Duration::from_millis(20);
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = tower::service_fn({
            let calls = Arc::clone(&calls);
            move |_req: Request<Body>| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok::<_, Infallible>(Response::new(Body::empty())))
            }
        });
        let layer = BudgetLimitLayer::new(BOUND, 1, QUEUE_WAIT);
        let mut svc = tower::Layer::layer(&layer, inner);
        let req = || {
            Request::builder()
                .uri("/cam1/media.m3u8")
                .body(Body::empty())
                .unwrap()
        };

        // Hold the only permit by a response whose body is never polled.
        let held = svc.call(req()).await.expect("infallible");
        let resp = svc.call(req()).await.expect("infallible");
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a shed request never reaches inner"
        );

        drop(held);
        let resp = svc.call(req()).await.expect("infallible");
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// A bound of N admits exactly N ordinary requests; the N+1th is shed with
    /// `503` (never parked indefinitely), and a freed permit is reused.
    #[tokio::test]
    async fn a_full_queue_is_rejected_after_the_timeout() {
        const BOUND: usize = 2;
        const QUEUE_WAIT: Duration = Duration::from_millis(50);
        let inner = tower::service_fn(|_req: Request<Body>| {
            std::future::ready(Ok::<_, Infallible>(Response::new(Body::empty())))
        });
        let mut svc = tower::Layer::layer(&BudgetLimitLayer::new(BOUND, 1, QUEUE_WAIT), inner);
        let req = || {
            Request::builder()
                .uri("/cam1/media.m3u8")
                .body(Body::empty())
                .unwrap()
        };

        // Two permits are free and both are admitted (their bodies held).
        let first = svc.call(req()).await.expect("infallible");
        let second = svc.call(req()).await.expect("infallible");
        assert_eq!(first.status(), axum::http::StatusCode::OK);
        assert_eq!(second.status(), axum::http::StatusCode::OK);

        // The third has no permit free: it waits the queue timeout, then shed
        // (never parked indefinitely).
        let started = std::time::Instant::now();
        let third = svc.call(req()).await.expect("infallible");
        assert_eq!(
            third.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "the 3rd request must be shed, not queued"
        );
        assert!(
            started.elapsed() >= QUEUE_WAIT,
            "the shed must have waited the full queue timeout, waited {:?}",
            started.elapsed()
        );

        // A freed permit is reused.
        drop(first);
        let fourth = svc.call(req()).await.expect("infallible");
        assert_eq!(fourth.status(), axum::http::StatusCode::OK);
        drop(second);
        drop(fourth);
    }

    /// A blocking reload draws from its own pool: saturating the ordinary
    /// pool does not shed a reload, and saturating the reload pool does not
    /// touch the ordinary pool.
    #[tokio::test]
    async fn blocking_reloads_have_their_own_pool() {
        let inner = tower::service_fn(|_req: Request<Body>| {
            std::future::ready(Ok::<_, Infallible>(Response::new(Body::empty())))
        });
        // ordinary bound 2, reload bound 1.
        let mut svc = tower::Layer::layer(
            &BudgetLimitLayer::new(2, 1, Duration::from_millis(20)),
            inner,
        );
        let ordinary = || {
            Request::builder()
                .uri("/cam1/media.m3u8")
                .body(Body::empty())
                .unwrap()
        };
        let reload = || {
            Request::builder()
                .uri("/cam1/media.m3u8?_HLS_msn=5")
                .body(Body::empty())
                .unwrap()
        };

        // Exhaust the ordinary pool entirely (2 held).
        let a = svc.call(ordinary()).await.expect("infallible");
        let b = svc.call(ordinary()).await.expect("infallible");
        // A reload is admitted even with the ordinary pool exhausted (separate).
        let r1 = svc.call(reload()).await.expect("infallible");
        assert_eq!(
            r1.status(),
            axum::http::StatusCode::OK,
            "a reload must be admitted even with the ordinary pool exhausted"
        );
        // The reload pool is its own bound: a second reload is shed.
        let r2 = svc.call(reload()).await.expect("infallible");
        assert_eq!(
            r2.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "the reload pool is its own bound"
        );
        drop(a);
        drop(b);
        drop(r1);
        drop(r2);
    }

    /// The permit is held until the inner service has produced its response
    /// (which, in this origin, includes collecting the body): a second request
    /// issued while the first is still being produced is shed.
    #[tokio::test]
    async fn the_permit_is_held_until_the_response_is_produced() {
        const QUEUE_WAIT: Duration = Duration::from_millis(20);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let entered_tx = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
        let inner = tower::service_fn(move |_req: Request<Body>| {
            let entered_tx = Arc::clone(&entered_tx);
            async move {
                if let Some(tx) = entered_tx.lock().expect("tx").take() {
                    let _ = tx.send(());
                }
                // A response that never resolves (still "producing").
                std::future::pending::<()>().await;
                #[allow(unreachable_code)]
                Ok::<_, Infallible>(Response::new(Body::empty()))
            }
        });
        let mut svc = tower::Layer::layer(&BudgetLimitLayer::new(1, 1, QUEUE_WAIT), inner);
        let req = || {
            Request::builder()
                .uri("/cam1/media.m3u8")
                .body(Body::empty())
                .unwrap()
        };

        let first = tokio::spawn({
            let mut svc = svc.clone();
            async move { svc.call(req()).await }
        });
        // HANDSHAKE: the first request entered the inner service (holds the
        // permit).
        tokio::time::timeout(Duration::from_secs(10), entered_rx)
            .await
            .expect("the first request must reach the inner service")
            .expect("the handshake sender must not be dropped");

        let resp = tokio::time::timeout(Duration::from_secs(10), svc.call(req()))
            .await
            .expect("the second request must not hang")
            .expect("infallible");
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "a second request must be shed while the first still holds its permit"
        );
        first.abort();
    }

    /// The permit covers the whole response, body included: while a streamed
    /// response is still open its permit stays taken (a second request is
    /// shed); once the body ends it is released.
    #[tokio::test]
    async fn the_permit_is_held_until_the_response_body_ends() {
        use axum::Router;
        use axum::routing::get;
        use tower::ServiceExt;

        let (tx, rx) = tokio::sync::mpsc::channel::<axum::body::Bytes>(4);
        let rx = Arc::new(std::sync::Mutex::new(Some(rx)));
        let app = Router::new()
            .route(
                "/stream",
                get(move || {
                    let rx = rx.lock().unwrap().take().expect("one streaming request");
                    async move {
                        Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move {
                            rx.recv().await.map(|c| (Ok::<_, Infallible>(c), rx))
                        }))
                    }
                }),
            )
            .route("/fast", get(|| async { "ok" }))
            .layer(BudgetLimitLayer::new(1, 1, Duration::from_millis(50)));
        let req = |path: &str| Request::builder().uri(path).body(Body::empty()).unwrap();

        tx.send(axum::body::Bytes::from_static(b"chunk"))
            .await
            .unwrap();
        let streaming = app.clone().oneshot(req("/stream")).await.unwrap();
        assert_eq!(streaming.status(), axum::http::StatusCode::OK);

        let shed = app.clone().oneshot(req("/fast")).await.unwrap();
        assert_eq!(
            shed.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "the streaming response still holds the only permit"
        );

        drop(tx);
        let body = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(streaming.into_body(), usize::MAX),
        )
        .await
        .expect("body ends once the producer is done")
        .unwrap();
        assert_eq!(&body[..], b"chunk");

        let after = app.oneshot(req("/fast")).await.unwrap();
        assert_eq!(
            after.status(),
            axum::http::StatusCode::OK,
            "permit released at body end"
        );
    }

    /// A client that disconnects mid-stream drops the response body: the
    /// permit it held must come back (the producer is still open, so the body
    /// never ends by itself).
    #[tokio::test]
    async fn dropping_the_response_body_mid_stream_releases_the_permit() {
        use axum::Router;
        use axum::routing::get;
        use tower::ServiceExt;

        let (tx, rx) = tokio::sync::mpsc::channel::<axum::body::Bytes>(4);
        let rx = Arc::new(std::sync::Mutex::new(Some(rx)));
        let app = Router::new()
            .route(
                "/stream",
                get(move || {
                    let rx = rx.lock().unwrap().take().expect("one streaming request");
                    async move {
                        Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move {
                            rx.recv().await.map(|c| (Ok::<_, Infallible>(c), rx))
                        }))
                    }
                }),
            )
            .route("/fast", get(|| async { "ok" }))
            .layer(BudgetLimitLayer::new(1, 1, Duration::from_millis(50)));
        let req = |path: &str| Request::builder().uri(path).body(Body::empty()).unwrap();
        tx.send(axum::body::Bytes::from_static(b"chunk"))
            .await
            .unwrap();
        let streaming = app.clone().oneshot(req("/stream")).await.unwrap();
        let shed = app.clone().oneshot(req("/fast")).await.unwrap();
        assert_eq!(
            shed.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "permit held"
        );

        // The client goes away with the stream still open (`tx` alive).
        let mut stream = streaming.into_body().into_data_stream();
        let first = futures_util::StreamExt::next(&mut stream)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&first[..], b"chunk");
        drop(stream);

        let after = app.oneshot(req("/fast")).await.unwrap();
        assert_eq!(
            after.status(),
            axum::http::StatusCode::OK,
            "permit returned on drop"
        );
        drop(tx);
    }
}
