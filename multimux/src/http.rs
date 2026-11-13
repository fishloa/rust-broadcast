//! The **one** axum adapter that turns a [`media_plane::egress::ServedEgress`]
//! resolution into a real HTTP response (plan step 5b) — every output/route
//! that resolves against a [`crate::route::RouteHandle`] goes through the two
//! functions here (`resolve_blocking` + `into_response`), rather than each
//! hand-rolling its own blocking-reload wait loop and status-code mapping.
//!
//! `ServedEgress` is sans-IO by design (`media_plane::egress`'s own module
//! doc): no `axum`, no `tokio`, no HTTP type appears in it. This module is
//! the one place that bridges it to axum — [`crate::output::hls`]'s
//! playlist route, the shared init/segment/part resource route
//! ([`crate::origin::resource`]), and [`crate::output::dash`]/
//! [`crate::output::ll_dash`]'s manifest routes all call the same two
//! functions below rather than each reimplementing the wait loop or the
//! `EgressResponse` -> `Response` mapping. If a future output route is
//! tempted to hand-roll HTTP handling instead of calling these, that
//! reintroduces exactly the duplication this design exists to delete.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use broadcast_common::Timestamp;
use bytes::Bytes;
use media_plane::Trunk;
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use tokio::sync::Semaphore;

use crate::route::ProgramServing;

/// Upper bound on how long [`resolve_blocking`] parks a caller waiting for a
/// [`ServedEgress::resolve`] to stop answering [`EgressResponse::Await`] — no
/// call site may wait longer than this.
pub(crate) const BLOCKING_RELOAD_TIMEOUT: Duration = hls_runtime::server::BLOCKING_RELOAD_TIMEOUT;

/// Resolves `route`'s single-program-route [`ProgramServing`] bundle
/// ([`crate::route::SPTS_PROGRAM_ID`]) against its registry, handling the
/// three-way [`crate::route::ProgramResolution`] the same way at every
/// migrated egress call site (issue #805 tasks 3/4/6): `Found` hands back the
/// whole bundle (`Trunk` + `HlsOrigin` + `DashState`, resolved together from
/// one registry read, so a caller never risks reading one program's `Trunk`
/// against a different program's `HlsOrigin`); `NotYetAnnounced` (the route
/// is connected but no program has appeared yet — ingest may still be
/// dialing/handshaking) is a `503 Service Unavailable` "not ready", **not** a
/// `404` — collapsing the two would make an ordinary route mid-connect
/// indistinguishable from a client hitting a route that will never exist,
/// exactly the failure mode [`crate::route::ProgramResolution`]'s own doc
/// exists to prevent; `NotFound` (a genuinely different, permanent absence)
/// is a real `404`.
///
/// Every caller (`crate::output::hls::media_playlist`,
/// `crate::output::dash::manifest`, `crate::output::ll_dash::manifest`,
/// `crate::origin::resource::dynamic_file`/`fetch_part`) matches this
/// `Result` once, up front, before touching `resolve_blocking`/`ll_hls()` at
/// all — so neither of those needs its own "no program yet" branch.
pub(crate) fn resolve_route_program(
    route: &crate::route::RouteHandle,
) -> core::result::Result<Arc<ProgramServing>, Box<Response>> {
    // `Box<Response>`, not a bare `Response`: an `axum::Response` is >=128
    // bytes, so returning it unboxed makes every `Result` in this signature
    // that large on the success path too (clippy::result_large_err, which
    // the newer-stable canary enforces). The error path here is the rare one
    // -- a route mid-connect or a bad program id -- so one allocation there
    // is strictly better than widening the common case.
    match route.resolve_program(crate::route::SPTS_PROGRAM_ID) {
        crate::route::ProgramResolution::Found(serving) => Ok(serving),
        crate::route::ProgramResolution::NotYetAnnounced => {
            Err(Box::new(StatusCode::SERVICE_UNAVAILABLE.into_response()))
        }
        crate::route::ProgramResolution::NotFound => {
            Err(Box::new(StatusCode::NOT_FOUND.into_response()))
        }
    }
}

/// [`resolve_route_program`] for a playlist request: a request made before the route announces
/// its program is held up to [`BLOCKING_RELOAD_TIMEOUT`] (RFC 8216bis section 6.2.5.2: a Server
/// that cannot provide the Playlist after blocking SHOULD answer 503) when the route holds cold
/// playlist requests, and a give-up is a `503` with `Retry-After`.
pub(crate) async fn resolve_route_program_held(
    route: &crate::route::RouteHandle,
) -> core::result::Result<Arc<ProgramServing>, Box<Response>> {
    match route
        .resolve_program_held(crate::route::SPTS_PROGRAM_ID, BLOCKING_RELOAD_TIMEOUT)
        .await
    {
        crate::route::ProgramResolution::Found(serving) => Ok(serving),
        crate::route::ProgramResolution::NotYetAnnounced if route.holds_cold_playlist() => {
            Err(Box::new(
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(axum::http::header::RETRY_AFTER, "1")],
                )
                    .into_response(),
            ))
        }
        crate::route::ProgramResolution::NotYetAnnounced => {
            Err(Box::new(StatusCode::SERVICE_UNAVAILABLE.into_response()))
        }
        crate::route::ProgramResolution::NotFound => {
            Err(Box::new(StatusCode::NOT_FOUND.into_response()))
        }
    }
}

pub(crate) const PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";

/// `GET /master.m3u8` for an HLS route: the origin's own master (measured peak
/// `BANDWIDTH` and the `CODECS` of its init segment, RFC 8216 §4.3.4.2) once the
/// program is published, 503 with `Retry-After` while there is nothing to measure.
pub(crate) fn master_playlist_response(
    route: &crate::route::RouteHandle,
    render: impl FnOnce(
        &hls_runtime::server::HlsOrigin,
    ) -> Result<String, hls_runtime::server::HlsMasterError>,
) -> Response {
    let retry = |body: &'static str| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [(axum::http::header::RETRY_AFTER, "1")],
            body,
        )
            .into_response()
    };
    let Ok(serving) = resolve_route_program(route) else {
        return retry("not serving yet\n");
    };
    match render(&serving.ll_hls()) {
        Ok(master) => (
            [(axum::http::header::CONTENT_TYPE, PLAYLIST_CONTENT_TYPE)],
            master,
        )
            .into_response(),
        Err(hls_runtime::server::HlsMasterError::BandwidthUnknown) => retry("bandwidth unknown\n"),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Resolve `request` against `egress`, holding a blocking-reload-style wait
/// (RFC 8216bis §6.2.5.2) up to `timeout` while [`ServedEgress::resolve`]
/// answers [`EgressResponse::Await`] — the caller-driven wait loop
/// `hls_runtime::server`'s own module doc sketches, generalised to any
/// `ServedEgress` implementation (LL-HLS's [`hls_runtime::server::HlsOrigin`],
/// or this crate's own DASH/LL-DASH manifest origins).
///
/// A `Trunk::listen()` wake-up is registered **before** re-checking
/// `resolve` (never after) — the same ordering [`Trunk::listen`]'s own docs
/// require, so a publish racing between the first `Await` and the
/// registration is never missed. The fast path (the overwhelmingly common
/// case: a request that resolves immediately, e.g. every DASH/LL-DASH
/// manifest call, which never answers `Await` at all) never touches
/// `Trunk::listen` — only a request that has genuinely seen `Await` twice in
/// a row (once before, once after registering) commits to an actual wait,
/// via `on_enter_wait` (invoked at most once, the first time this call
/// commits to parking) — callers that want an observability gauge around the
/// wait (e.g. `crate::origin::resource::BlockingRequestGuard`) construct it
/// there; callers that never genuinely wait (DASH/LL-DASH) pass a no-op.
///
/// Bounded twice over: by `timeout` (an absolute deadline, composed via
/// [`AwaitPolicy`] so `resolve` itself stops answering `Await` once it
/// passes — see [`EgressResponse::pending`]'s own doc), and by
/// [`Trunk::listen`]'s waiter cap (`TrunkConfig::waiter_capacity`). A
/// request beyond the cap parks on [`Trunk::waiter_slot_freed`], raced
/// against its own deadline, and never sleep-polls — this is the concrete
/// mechanism satisfying "bound anything that accumulates, including parked
/// `Await` requests".
pub(crate) async fn resolve_blocking<E, G>(
    trunk: &Arc<Trunk>,
    warms: &Semaphore,
    egress: &Arc<E>,
    request: E::Request,
    timeout: Duration,
    on_enter_wait: impl FnOnce() -> G,
) -> EgressResponse<E::Body>
where
    E: ServedEgress,
    E::Request: Clone + Send + 'static,
{
    resolve_blocking_timed(trunk, warms, egress, request, timeout, on_enter_wait)
        .await
        .0
}

/// [`resolve_blocking`] plus whether a `NotFound` answer is the engine giving
/// up on a wait (the request answered `Await` at least once) rather than a
/// resource that does not exist. A Playlist whose blocking reload times out
/// answers `503` (RFC 8216bis section 6.2.5.2), never `404`.
pub(crate) async fn resolve_blocking_timed<E, G>(
    trunk: &Arc<Trunk>,
    warms: &Semaphore,
    egress: &Arc<E>,
    request: E::Request,
    timeout: Duration,
    on_enter_wait: impl FnOnce() -> G,
) -> (EgressResponse<E::Body>, bool)
where
    E: ServedEgress,
    E::Request: Clone + Send + 'static,
{
    let mut waited = false;
    let resp = resolve_blocking_inner(
        trunk,
        warms,
        egress,
        request,
        timeout,
        on_enter_wait,
        &mut waited,
    )
    .await;
    let timed_out = waited && matches!(resp, EgressResponse::NotFound);
    (resp, timed_out)
}

/// Run `egress`'s [`ServedEgress::prepare`] for `request` on the blocking pool
/// when [`ServedEgress::is_warm`] says the resolve is not cheap, so a request
/// that has just woken never does a scan, demux or packaging on the async
/// worker that woke it. The caller's `warms` semaphore (one per route) bounds how many run at once; a request
/// queued on it re-checks warmth once admitted and skips its own if an
/// earlier one has finished. `prepare` must itself tolerate running
/// concurrently with itself.
async fn ensure_warm<E>(warms: &Semaphore, egress: &Arc<E>, request: &E::Request)
where
    E: ServedEgress,
    E::Request: Clone + Send + 'static,
{
    if egress.is_warm(request) {
        return;
    }
    let Ok(_permit) = warms.acquire().await else {
        return;
    };
    if egress.is_warm(request) {
        return;
    }
    let (egress, request) = (Arc::clone(egress), request.clone());
    if let Err(error) = tokio::task::spawn_blocking(move || egress.prepare(request)).await {
        tracing::warn!(%error, "egress warm-up task failed");
    }
}

async fn resolve_blocking_inner<E, G>(
    trunk: &Arc<Trunk>,
    warms: &Semaphore,
    egress: &Arc<E>,
    request: E::Request,
    timeout: Duration,
    on_enter_wait: impl FnOnce() -> G,
    waited: &mut bool,
) -> EgressResponse<E::Body>
where
    E: ServedEgress,
    E::Request: Clone + Send + 'static,
{
    let base = Instant::now();
    let deadline_instant = base + timeout;
    let deadline = Timestamp::from_instant(base.into_std(), deadline_instant.into_std());
    let policy = AwaitPolicy::new(deadline);

    let mut on_enter_wait = Some(on_enter_wait);
    let mut _wait_guard: Option<G> = None;

    loop {
        ensure_warm(warms, egress, &request).await;
        let now = Timestamp::from_instant(base.into_std(), Instant::now().into_std());
        match egress.resolve(request.clone(), now, policy) {
            EgressResponse::Await { .. } => *waited = true,
            other => return other,
        }

        // Genuinely not ready on that check: register a wake-up BEFORE
        // re-checking (no missed-wakeup race), then re-check once more in
        // case the answer changed between the first `resolve` above and this
        // registration landing.
        let listener = trunk.listen();
        if listener.is_some() {
            ensure_warm(warms, egress, &request).await;
            let now = Timestamp::from_instant(base.into_std(), Instant::now().into_std());
            match egress.resolve(request.clone(), now, policy) {
                EgressResponse::Await { .. } => *waited = true,
                other => return other,
            }
        }

        let remaining = deadline_instant.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Out of patience: one final resolve to get the honest
            // "gave up waiting" answer (`EgressResponse::pending`'s own
            // now-past-deadline path), rather than looping forever.
            ensure_warm(warms, egress, &request).await;
            let now = Timestamp::from_instant(base.into_std(), Instant::now().into_std());
            return egress.resolve(request.clone(), now, policy);
        }

        if _wait_guard.is_none()
            && let Some(f) = on_enter_wait.take()
        {
            _wait_guard = Some(f());
        }

        match listener {
            Some(listener) => {
                let _ = tokio::time::timeout(remaining, listener).await;
            }
            None => {
                if let Some(freed) = trunk.waiter_slot_freed() {
                    let _ = tokio::time::timeout(remaining, freed).await;
                }
            }
        }
    }
}

/// Status of a Playlist answer the engine gave up waiting for.
///
/// RFC 8216bis section 6.2.5.2: "A Server that cannot provide the requested
/// Playlist after blocking for more than three Target Durations SHOULD
/// return Service Unavailable, such as HTTP 503."
pub(crate) fn playlist_not_found_status(timed_out: bool) -> StatusCode {
    if timed_out {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::NOT_FOUND
    }
}

/// [`into_response`] for a Playlist: a wait the engine gave up on answers `503`
/// with `Retry-After`, an absent Playlist `404`.
pub(crate) fn into_playlist_response<B>(
    resp: EgressResponse<B>,
    timed_out: bool,
    render_ready: impl FnOnce(B) -> Response,
) -> Response {
    let mut response = into_response(resp, playlist_not_found_status(timed_out), render_ready);
    if response.status() == StatusCode::SERVICE_UNAVAILABLE {
        response.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            HeaderValue::from_static(PLAYLIST_RETRY_AFTER_SECS),
        );
    }
    response
}

const PLAYLIST_RETRY_AFTER_SECS: &str = "1";

/// Turn one [`EgressResponse`] into an HTTP [`Response`] — the one place in
/// this crate that does so. `render_ready` supplies the content-type/body
/// for [`EgressResponse::Ready`] (protocol-specific: LL-HLS's `HlsBody`'s
/// two shapes, or a plain MPD `String`); `not_found_status` is the one
/// per-route status choice this adapter still leaves to its caller — a
/// resource byte-range that will never exist is a `404` (gone forever), but
/// a DASH/LL-DASH manifest that cannot yet be described at all (issue #776:
/// no track with a derivable codec string, not just "no segment has closed
/// yet") is a `503` (may become available once a usable track lands) — see
/// each call site.
pub(crate) fn into_response<B>(
    resp: EgressResponse<B>,
    not_found_status: StatusCode,
    render_ready: impl FnOnce(B) -> Response,
) -> Response {
    into_response_with_message(
        resp,
        not_found_status,
        StatusCode::BAD_REQUEST,
        render_ready,
    )
}

/// [`into_response`] for a route whose `BadRequest` answer is *not* the
/// client's fault: an egress that returns `BadRequest` because it cannot
/// describe what it holds (rather than because the request is malformed)
/// reports it as the caller's chosen `refused_status`.
pub(crate) fn into_response_with_message<B>(
    resp: EgressResponse<B>,
    not_found_status: StatusCode,
    refused_status: StatusCode,
    render_ready: impl FnOnce(B) -> Response,
) -> Response {
    match resp {
        EgressResponse::Ready { body, .. } => render_ready(body),
        EgressResponse::NotFound => not_found_status.into_response(),
        EgressResponse::BadRequest { .. } => refused_status.into_response(),
        // `EgressResponse` is `#[non_exhaustive]`, and `Await` should never
        // reach here (`resolve_blocking` only ever returns once `resolve`
        // stops answering it, or its own deadline has passed and
        // `EgressResponse::pending` has already downgraded it to
        // `NotFound`) -- treat any such variant the same defensive way the
        // rest of this crate treats an unrecognized future variant: the
        // `not_found_status` fallback, never a panic or a fabricated body.
        _ => not_found_status.into_response(),
    }
}

use std::ops::Bound;

/// Serve a final resource body with HTTP Range support (RFC 9110 §14.2 /
/// §14.4 / §15.3.7 / §15.5.17) — the single shared helper used by every
/// handler that returns a sized resource body (init/segment/part). Given the
/// request headers and the final bytes, produce:
///
/// - no `Range` (or a `Range` the helper must ignore) -> `200 OK` with the
///   full body + `Accept-Ranges: bytes`;
/// - a single satisfiable `bytes=` range -> `206 Partial Content` with
///   `Content-Range: bytes first-last/total`, `Content-Length` of the slice,
///   the same content type, and `Accept-Ranges: bytes`;
/// - an unsatisfiable range (first byte position >= total) -> `416 Range Not
///   Satisfiable` with `Content-Range: bytes */total`;
/// - a suffix (`bytes=-N`) or open-ended (`bytes=a-`) range per §14.1.2;
/// - multiple ranges -> `200 OK` with the full body (multipart/byteranges is
///   deliberately not implemented, §14.2 permits answering `200`);
/// - a non-`bytes` unit or a syntactically invalid `Range` -> ignored, `200`.
///
/// `If-Range` is honoured in the safe, spec-correct way for these
/// immutable-by-name resources, which carry no validator: the `Range` is
/// ignored and a `200` with the full body is returned (§13.1.5).
pub(crate) fn range_response(headers: &HeaderMap, bytes: Bytes, content_type: &str) -> Response {
    use headers::HeaderMapExt as _;

    let ct = HeaderValue::from_str(content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));

    // Full-body (200) response carrying Content-Type + Accept-Ranges: bytes.
    // Shared by every path that ignores the Range (If-Range, no Range,
    // unparseable Range, multi-range).
    let full_response = |bytes: Bytes| -> Response {
        let mut r = Response::new(Body::from(bytes));
        *r.status_mut() = StatusCode::OK;
        r.headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, ct.clone());
        headers::HeaderMapExt::typed_insert(r.headers_mut(), headers::AcceptRanges::bytes());
        r
    };

    // If-Range present: these resources carry no validator, so the safe,
    // spec-correct behaviour is to ignore the Range and answer 200 (§13.1.5).
    if headers.get(axum::http::header::IF_RANGE).is_some() {
        return full_response(bytes);
    }

    let total = bytes.len() as u64;

    let range = headers.typed_get::<headers::Range>();
    let Some(range) = range else {
        // No (typed) Range header -> full body.
        return full_response(bytes);
    };

    let collected: Vec<(Bound<u64>, Bound<u64>)> = range.satisfiable_ranges(total).collect();

    // Syntactically invalid / non-bytes unit already filtered by typed_get;
    // an empty satisfiable set means the Range could not be parsed into a
    // byte spec at all -> ignore and answer 200 (§14.2).
    if collected.is_empty() {
        return full_response(bytes);
    }

    // Multiple ranges -> answer 200 with the full body (§14.2; multipart/
    // byteranges deliberately not implemented).
    if collected.len() >= 2 {
        return full_response(bytes);
    }

    let (start_bound, end_bound) = collected[0];

    // An invalid byte-range-spec (last-pos < first-pos, or neither bound) is
    // ignored, not unsatisfiable (RFC 9110 §14.1.1, §14.2).
    let invalid_spec = match (start_bound, end_bound) {
        (Bound::Included(a), Bound::Included(b)) => a > b,
        (Bound::Unbounded, Bound::Unbounded) => true,
        _ => false,
    };
    if invalid_spec {
        return full_response(bytes);
    }

    let first = match start_bound {
        Bound::Included(x) => x,
        Bound::Excluded(x) => x.saturating_add(1),
        Bound::Unbounded => 0,
    };
    let mut last = match end_bound {
        Bound::Included(x) => x,
        Bound::Excluded(x) => x.saturating_sub(1),
        Bound::Unbounded => total.saturating_sub(1),
    };

    // Unsatisfiable: first byte position >= total (§14.1.2).
    if total == 0 || first >= total || first > last {
        let mut resp = Response::new(Body::empty());
        *resp.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
        resp.headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, ct);
        headers::HeaderMapExt::typed_insert(
            resp.headers_mut(),
            headers::ContentRange::unsatisfied_bytes(total),
        );
        headers::HeaderMapExt::typed_insert(resp.headers_mut(), headers::AcceptRanges::bytes());
        return resp;
    }

    // Clamp a last-pos beyond the length to the final byte (§14.1.2).
    if total > 0 {
        last = last.min(total - 1);
    }

    let slice_len = (last - first + 1) as usize;
    let slice = bytes.slice(first as usize..(first as usize + slice_len));

    let cr = headers::ContentRange::bytes(first..=last, total)
        .unwrap_or_else(|_| headers::ContentRange::unsatisfied_bytes(total));

    let mut resp = Response::new(Body::from(slice));
    *resp.status_mut() = StatusCode::PARTIAL_CONTENT;
    resp.headers_mut()
        .insert(axum::http::header::CONTENT_TYPE, ct);
    headers::HeaderMapExt::typed_insert(resp.headers_mut(), cr);
    headers::HeaderMapExt::typed_insert(
        resp.headers_mut(),
        headers::ContentLength(slice_len as u64),
    );
    headers::HeaderMapExt::typed_insert(resp.headers_mut(), headers::AcceptRanges::bytes());
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::TrunkConfig;
    use media_plane::egress::CachePolicy;
    use media_plane::trunk::PartEntry;
    use std::num::NonZeroUsize;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    struct GatedEgress {
        ready: AtomicBool,
        resolves: AtomicUsize,
    }

    impl ServedEgress for GatedEgress {
        type Request = ();
        type Body = ();

        fn resolve(&self, (): (), now: Timestamp, policy: AwaitPolicy) -> EgressResponse<()> {
            self.resolves.fetch_add(1, Ordering::SeqCst);
            if self.ready.load(Ordering::SeqCst) {
                EgressResponse::Ready {
                    body: (),
                    cache: CachePolicy::NoCache,
                }
            } else {
                EgressResponse::pending(policy, now, now)
            }
        }
    }

    struct ColdEgress {
        warm: AtomicBool,
        prepares: AtomicUsize,
    }

    const WARM_PERMITS: usize = 8;
    const PREPARE_DURATION: Duration = Duration::from_millis(300);
    const TICK: Duration = Duration::from_millis(10);

    impl ServedEgress for ColdEgress {
        type Request = ();
        type Body = ();

        fn resolve(&self, (): (), _now: Timestamp, _policy: AwaitPolicy) -> EgressResponse<()> {
            EgressResponse::Ready {
                body: (),
                cache: CachePolicy::NoCache,
            }
        }

        fn is_warm(&self, (): &()) -> bool {
            self.warm.load(Ordering::SeqCst)
        }

        fn prepare(&self, (): ()) {
            self.prepares.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(PREPARE_DURATION);
            self.warm.store(true, Ordering::SeqCst);
        }
    }

    /// #2538: a cold egress is prepared on the blocking pool. On a
    /// single-threaded runtime the one worker keeps ticking while the
    /// 300 ms `prepare` runs; inline it would starve the ticker. Concurrent
    /// requests start at most `WARM_PERMITS` prepares, never one each.
    #[tokio::test(flavor = "current_thread")]
    async fn a_cold_egress_is_prepared_off_the_async_worker() {
        let trunk = Trunk::new(TrunkConfig::new(nz(8), nz(8), nz(4), nz(8), nz(8)));
        let warms = Arc::new(Semaphore::new(WARM_PERMITS));
        let warms = Arc::new(Semaphore::new(WARM_PERMITS));
        let egress = Arc::new(ColdEgress {
            warm: AtomicBool::new(false),
            prepares: AtomicUsize::new(0),
        });
        let requests: Vec<_> = (0..16)
            .map(|_| {
                let (trunk, egress, warms) =
                    (Arc::clone(&trunk), Arc::clone(&egress), Arc::clone(&warms));
                tokio::spawn(async move {
                    resolve_blocking(&trunk, &warms, &egress, (), BLOCKING_TIMEOUT, || ()).await
                })
            })
            .collect();

        let mut ticks = 0u32;
        let started = tokio::time::Instant::now();
        while started.elapsed() < PREPARE_DURATION {
            tokio::time::sleep(TICK).await;
            ticks += 1;
        }
        assert!(
            ticks >= 10,
            "the worker was blocked while preparing: {ticks} ticks"
        );
        for request in requests {
            assert!(matches!(
                request.await.expect("request task"),
                EgressResponse::Ready { .. }
            ));
        }
        let prepares = egress.prepares.load(Ordering::SeqCst);
        assert!((1..=WARM_PERMITS).contains(&prepares), "{prepares}");
    }

    /// One route's slow `prepare` holding every one of its warm permits does
    /// not delay another route's: each route owns its own semaphore.
    #[tokio::test(flavor = "multi_thread")]
    async fn one_routes_slow_prepare_does_not_delay_another_route() {
        let trunk = Trunk::new(TrunkConfig::new(nz(8), nz(8), nz(4), nz(8), nz(8)));
        let cold = || {
            Arc::new(ColdEgress {
                warm: AtomicBool::new(false),
                prepares: AtomicUsize::new(0),
            })
        };
        let (slow_route, other_route) = (Arc::new(Semaphore::new(1)), Arc::new(Semaphore::new(1)));
        let slow_egress = cold();
        let slow: Vec<_> = (0..4)
            .map(|_| {
                let (trunk, warms, egress) = (
                    Arc::clone(&trunk),
                    Arc::clone(&slow_route),
                    Arc::clone(&slow_egress),
                );
                tokio::spawn(async move {
                    resolve_blocking(&trunk, &warms, &egress, (), BLOCKING_TIMEOUT, || ()).await
                })
            })
            .collect();
        while slow_egress.prepares.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        let started = std::time::Instant::now();
        let other = cold();
        let resp =
            resolve_blocking(&trunk, &other_route, &other, (), BLOCKING_TIMEOUT, || ()).await;
        assert!(matches!(resp, EgressResponse::Ready { .. }));
        assert!(
            started.elapsed() < PREPARE_DURATION * 3 / 2,
            "the other route waited behind the slow route's prepare: {:?}",
            started.elapsed()
        );
        for task in slow {
            task.await.expect("request task");
        }
    }

    const PARKED_REQUESTS: usize = 200;
    const IDLE_WINDOW: Duration = Duration::from_millis(300);
    const BLOCKING_TIMEOUT: Duration = Duration::from_secs(20);
    const RESOLVES_PER_REQUEST_BEFORE_PARKING: usize = 2;

    /// Park `PARKED_REQUESTS` blocking requests on one trunk, stay idle, then
    /// publish a part: every request is answered, and none of them re-resolved
    /// (re-rendered) while parked.
    async fn assert_all_parked_requests_answer_without_polling(config: TrunkConfig) {
        let trunk = Trunk::new(config);
        let part_writer = trunk.segment_writer().expect("segment writer");
        let egress = Arc::new(GatedEgress {
            ready: AtomicBool::new(false),
            resolves: AtomicUsize::new(0),
        });
        let warms = Arc::new(Semaphore::new(WARM_PERMITS));
        let tasks: Vec<_> = (0..PARKED_REQUESTS)
            .map(|_| {
                let (trunk, egress, warms) =
                    (Arc::clone(&trunk), Arc::clone(&egress), Arc::clone(&warms));
                tokio::spawn(async move {
                    resolve_blocking(&trunk, &warms, &egress, (), BLOCKING_TIMEOUT, || ()).await
                })
            })
            .collect();

        tokio::time::sleep(IDLE_WINDOW).await;
        let resolves_while_idle = egress.resolves.load(Ordering::SeqCst);
        assert!(
            resolves_while_idle <= PARKED_REQUESTS * RESOLVES_PER_REQUEST_BEFORE_PARKING,
            "parked requests re-resolved while idle: {resolves_while_idle}"
        );

        egress.ready.store(true, Ordering::SeqCst);
        part_writer.publish_part(PartEntry::new(
            Bytes::from_static(b"p"),
            media_plane::ids::SegmentSeq::new(1),
            media_plane::ids::PartIndex::new(0),
            Duration::from_millis(500),
            true,
        ));

        for task in tasks {
            let resp = tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .expect("a parked request must be answered after the publish")
                .expect("request task");
            assert!(matches!(resp, EgressResponse::Ready { .. }));
        }
    }

    /// #2523: more concurrent blocking requests than the part ring holds all
    /// park on the trunk's wake instead of sleep-polling.
    #[tokio::test(flavor = "multi_thread")]
    async fn more_blocking_requests_than_part_capacity_all_park_and_answer() {
        assert_all_parked_requests_answer_without_polling(TrunkConfig::new(
            nz(8),
            nz(8),
            nz(4),
            nz(8),
            nz(64),
        ))
        .await;
    }

    /// #2523: a request with no waiter slot parks on `waiter_slot_freed`, not
    /// a sleep-poll, and is answered once slots cycle.
    #[tokio::test(flavor = "multi_thread")]
    async fn requests_beyond_the_waiter_cap_park_and_answer() {
        assert_all_parked_requests_answer_without_polling(
            TrunkConfig::new(nz(8), nz(8), nz(4), nz(8), nz(64)).with_waiter_capacity(nz(2)),
        )
        .await;
    }

    fn status_for(range: &str) -> StatusCode {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::RANGE, range.parse().unwrap());
        range_response(&h, Bytes::from_static(b"0123456789"), "video/mp4").status()
    }

    #[test]
    fn invalid_byte_range_specs_are_ignored_not_416() {
        assert_eq!(status_for("bytes=5-3"), StatusCode::OK);
        assert_eq!(status_for("bytes=-"), StatusCode::OK);
    }

    #[test]
    fn valid_and_unsatisfiable_ranges_keep_their_status() {
        assert_eq!(status_for("bytes=3-5"), StatusCode::PARTIAL_CONTENT);
        assert_eq!(status_for("bytes=-3"), StatusCode::PARTIAL_CONTENT);
        assert_eq!(status_for("bytes=20-30"), StatusCode::RANGE_NOT_SATISFIABLE);
    }
}
