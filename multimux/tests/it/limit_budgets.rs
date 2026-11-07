//! SP2.5: three budgets (ops-exempt / blocking-reload / ordinary) enforced by
//! one classify service over shared pools; a `_HLS_*` request is classified
//! by a typed query parse, not a substring scan. The permit lives in the
//! RESPONSE BODY, so a test HOLDS a permit by keeping a response whose body
//! it neither polls nor drops.

// Drives the production routers through the `whip_router_for_test`/
// `whep_router_for_test` seams, which are behind `test-hooks`.
#![cfg(all(feature = "whip", feature = "whep", feature = "test-hooks"))]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

#[test]
fn a_query_that_merely_contains_the_marker_is_not_a_blocking_reload() {
    use multimux::origin::limit::{Budget, classify};
    let u = |s: &str| s.parse::<axum::http::Uri>().unwrap();
    assert_eq!(
        classify(&u("/cam1/media.m3u8?_HLS_msnx=1")),
        Budget::Ordinary
    );
    assert_eq!(
        classify(&u("/cam1/media.m3u8?file=_HLS_msn")),
        Budget::Ordinary
    );
    assert_eq!(
        classify(&u("/cam1/media.m3u8?_HLS_msn=5")),
        Budget::BlockingReload
    );
    assert_eq!(
        classify(&u("/cam1/media.m3u8?_HLS_part=2")),
        Budget::BlockingReload
    );
    assert_eq!(classify(&u("/metrics")), Budget::Exempt);
}

/// One ordinary permit, held by an unread response body: a second ordinary
/// request — on a DIFFERENT route, proving the pool is shared across routes —
/// waits `queue_timeout`, then is shed `503` + `Retry-After`; an ops route
/// still answers; dropping the held response frees the permit.
///
/// Revert-checks (each run and observed to FAIL): (a) release the permit when
/// `call` returns (`drop(permit); Ok(resp)` instead of `pin_permit`) — the
/// second request is not shed (`200`, not `503`); (b) build the pools inside
/// `Layer::layer` instead of in `BudgetLimitLayer::new` — `Router::layer`
/// then makes one pool per route and the cross-route request gets `200`.
#[tokio::test(start_paused = true)]
async fn an_exhausted_ordinary_budget_sheds_and_ops_routes_still_answer() {
    let app = multimux::origin::limit_budget_test_app(1, Duration::from_millis(50));
    let hold = app.clone().oneshot(get("/cam1/master.m3u8")).await.unwrap();

    let shed = app.clone().oneshot(get("/cam1/media.m3u8")).await.unwrap();
    assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(shed.headers()["retry-after"], "1");

    let ops = app.clone().oneshot(get("/healthz")).await.unwrap();
    assert_eq!(
        ops.status(),
        StatusCode::OK,
        "ops routes are exempt from every budget"
    );

    drop(hold);
    let again = app.clone().oneshot(get("/cam1/media.m3u8")).await.unwrap();
    assert_ne!(
        again.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "the permit is freed when the body drops"
    );
}

/// I4: the WHIP/WHEP routers use ONE shared concurrency pool across routes
/// (`GlobalConcurrencyLimitLayer`, not the per-route `ConcurrencyLimitLayer`).
/// This drives the PRODUCTION `whep_router` (built by `whep_router_for_test`)
/// with its real routes and a low cap: a request held on `/whep` must block a
/// DIFFERENT route (`/whep/session`) on the SAME router — proof the two routes
/// share one semaphore. Reverting to `ConcurrencyLimitLayer` would give each
/// route its own cap-1 pool, so the second route would NOT block and this
/// test would fail.
#[cfg(feature = "whep")]
#[tokio::test]
async fn a_shared_concurrency_pool_is_one_across_routes() {
    use axum::body::Body;
    use axum::http::Request;
    use std::time::Duration;
    use tower::ServiceExt as _;

    // The real production WHEP router, cap 1 (dialled low to observe sharing).
    let router = multimux::output::whep::whep_router_for_test(None, 1);

    // Hold the single shared permit by streaming a body that never finishes:
    // axum's `Bytes` extractor inside the concurrency layer keeps the request
    // (and thus the permit) pending until the body completes.
    let hold = tokio::spawn({
        let router = router.clone();
        async move {
            let body = Body::from_stream(futures_util::stream::pending::<
                Result<axum::body::Bytes, std::io::Error>,
            >());
            router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/whep")
                        .header("content-type", "application/sdp")
                        .body(body)
                        .unwrap(),
                )
                .await
        }
    });

    // Let the first request acquire the permit (it blocks in the body
    // extractor holding the permit), then test the second route. Bounded
    // yields, not a sleep.
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }

    // A DIFFERENT route on the SAME router (`/whep/session` PATCH) must NOT
    // complete while the permit is held: it waits on the shared pool.
    let second = tokio::spawn({
        let router = router.clone();
        async move {
            tokio::time::timeout(
                Duration::from_millis(200),
                router.oneshot(
                    Request::builder()
                        .method("PATCH")
                        .uri("/whep/session")
                        .body(Body::empty())
                        .unwrap(),
                ),
            )
            .await
        }
    });

    // While the permit is held, the second route must be blocked (it waits for
    // the shared semaphore and times out rather than completing).
    assert!(
        second.await.unwrap().is_err(),
        "a different route must be blocked by the shared pool while a request holds the permit"
    );

    hold.abort();
}

/// I4 (WHIP counterpart): the WHIP router also uses ONE shared concurrency
/// pool across its routes, driven through the production `whip_router`
/// (built by `whip_router_for_test`) — the symmetric companion to
/// `a_shared_concurrency_pool_is_one_across_routes`. A request held on
/// `/whip` must block `/whip/session` PATCH on the same router.
#[cfg(feature = "whip")]
#[tokio::test]
async fn the_whip_router_uses_one_shared_pool_across_routes() {
    // The real production WHIP router, cap 1 (dialled low to observe sharing).
    let router = multimux::source::whip::whip_router_for_test(1, 1);

    // Hold the single shared permit by streaming a body that never finishes.
    let hold = tokio::spawn({
        let router = router.clone();
        async move {
            let body = Body::from_stream(futures_util::stream::pending::<
                Result<axum::body::Bytes, std::io::Error>,
            >());
            router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/whip")
                        .header("content-type", "application/sdp")
                        .body(body)
                        .unwrap(),
                )
                .await
        }
    });

    // Let the first request acquire the permit (bounded yields, not a sleep).
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }

    // A DIFFERENT route on the SAME router (`/whip/session` PATCH) must NOT
    // complete while the permit is held.
    let second = tokio::spawn({
        let router = router.clone();
        async move {
            tokio::time::timeout(
                Duration::from_millis(200),
                router.oneshot(
                    Request::builder()
                        .method("PATCH")
                        .uri("/whip/session")
                        .body(Body::empty())
                        .unwrap(),
                ),
            )
            .await
        }
    });

    assert!(
        second.await.unwrap().is_err(),
        "a different route must be blocked by the shared pool while a request holds the permit"
    );

    hold.abort();
}
