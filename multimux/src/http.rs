//! The **one** axum adapter that turns a [`media_plane::egress::ServedEgress`]
//! resolution into a real HTTP response (plan step 5b) — every output/route
//! that resolves against a [`crate::route::RouteHandle`] goes through the two
//! functions here (`resolve_blocking` + `into_response`), rather than each
//! hand-rolling its own blocking-reload wait loop and status-code mapping.
//!
//! `ServedEgress` is sans-IO by design (`media_plane::egress`'s own module
//! doc): no `axum`, no `tokio`, no HTTP type appears in it. This module is
//! the one place that bridges it to axum — [`crate::output::llhls`]'s
//! playlist route, the shared init/segment/part resource route
//! ([`crate::origin::resource`]), and [`crate::output::dash`]/
//! [`crate::output::ll_dash`]'s manifest routes all call the same two
//! functions below rather than each reimplementing the wait loop or the
//! `EgressResponse` -> `Response` mapping. If a future output route is
//! tempted to hand-roll HTTP handling instead of calling these, that
//! reintroduces exactly the duplication this design exists to delete.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use broadcast_common::Timestamp;
use media_plane::Trunk;
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};

/// Upper bound on how long [`resolve_blocking`] parks a caller waiting for a
/// [`ServedEgress::resolve`] to stop answering [`EgressResponse::Await`] — no
/// call site may wait longer than this.
pub(crate) const BLOCKING_RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolves `route`'s single-program-route `Trunk`
/// ([`crate::route::SPTS_PROGRAM_ID`]) against its registry, handling the
/// three-way [`crate::route::ProgramResolution`] the same way at every
/// migrated egress call site (issue #805 tasks 3/4): `Found` hands back the
/// `Trunk` to resolve against; `NotYetAnnounced` (the route is connected but
/// no program has appeared yet — ingest may still be dialing/handshaking) is
/// a `503 Service Unavailable` "not ready", **not** a `404` — collapsing the
/// two would make an ordinary route mid-connect indistinguishable from a
/// client hitting a route that will never exist, exactly the failure mode
/// [`crate::route::ProgramResolution`]'s own doc exists to prevent;
/// `NotFound` (a genuinely different, permanent absence) is a real `404`.
///
/// Every caller (`crate::output::llhls::media_playlist`,
/// `crate::output::dash::manifest`, `crate::output::ll_dash::manifest`,
/// `crate::origin::resource::dynamic_file`/`fetch_part`) matches this
/// `Result` once, up front, before touching `resolve_blocking`/`ll_hls()` at
/// all — so neither of those needs its own "no program yet" branch.
pub(crate) fn resolve_route_trunk(
    route: &crate::route::RouteHandle,
) -> core::result::Result<Arc<Trunk>, Response> {
    match route.resolve_program(crate::route::SPTS_PROGRAM_ID) {
        crate::route::ProgramResolution::Found(trunk) => Ok(trunk),
        crate::route::ProgramResolution::NotYetAnnounced => {
            Err(StatusCode::SERVICE_UNAVAILABLE.into_response())
        }
        crate::route::ProgramResolution::NotFound => Err(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Fallback poll interval when [`Trunk::listen`] has no free waiter slot
/// (`media_plane::trunk::TrunkConfig::part_capacity` already saturated by
/// other parked requests) — bounded by the caller's own `timeout` regardless,
/// so a burst of blocking requests beyond the slot cap degrades to polling
/// rather than either busy-spinning or leaking an unbounded wait.
const NO_SLOT_BACKOFF: Duration = Duration::from_millis(20);

/// Resolve `request` against `egress`, holding a blocking-reload-style wait
/// (RFC 8216bis §6.2.5.2) up to `timeout` while [`ServedEgress::resolve`]
/// answers [`EgressResponse::Await`] — the caller-driven wait loop
/// `ll_hls_runtime::server`'s own module doc sketches, generalised to any
/// `ServedEgress` implementation (LL-HLS's [`ll_hls_runtime::server::LlHlsOrigin`],
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
/// [`Trunk::listen`]'s own waiter-slot cap (a burst of requests beyond it
/// falls back to a capped poll rather than parking unboundedly) — this is
/// the concrete mechanism satisfying "bound anything that accumulates,
/// including parked `Await` requests".
pub(crate) async fn resolve_blocking<E, G>(
    trunk: &Arc<Trunk>,
    egress: &E,
    request: E::Request,
    timeout: Duration,
    on_enter_wait: impl FnOnce() -> G,
) -> EgressResponse<E::Body>
where
    E: ServedEgress,
    E::Request: Clone,
{
    let base = Instant::now();
    let deadline_instant = base + timeout;
    let deadline = Timestamp::from_instant(base, deadline_instant);
    let policy = AwaitPolicy::new(deadline);

    let mut on_enter_wait = Some(on_enter_wait);
    let mut _wait_guard: Option<G> = None;

    loop {
        let now = Timestamp::from_instant(base, Instant::now());
        match egress.resolve(request.clone(), now, policy) {
            EgressResponse::Await { .. } => {}
            other => return other,
        }

        // Genuinely not ready on that check: register a wake-up BEFORE
        // re-checking (no missed-wakeup race), then re-check once more in
        // case the answer changed between the first `resolve` above and this
        // registration landing.
        let listener = trunk.listen();
        let now = Timestamp::from_instant(base, Instant::now());
        match egress.resolve(request.clone(), now, policy) {
            EgressResponse::Await { .. } => {}
            other => return other,
        }

        let remaining = deadline_instant.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Out of patience: one final resolve to get the honest
            // "gave up waiting" answer (`EgressResponse::pending`'s own
            // now-past-deadline path), rather than looping forever.
            let now = Timestamp::from_instant(base, Instant::now());
            return egress.resolve(request.clone(), now, policy);
        }

        if _wait_guard.is_none() {
            if let Some(f) = on_enter_wait.take() {
                _wait_guard = Some(f());
            }
        }

        match listener {
            Some(listener) => {
                let _ = tokio::time::timeout(remaining, listener).await;
            }
            None => {
                tokio::time::sleep(remaining.min(NO_SLOT_BACKOFF)).await;
            }
        }
    }
}

/// Turn one [`EgressResponse`] into an HTTP [`Response`] — the one place in
/// this crate that does so. `render_ready` supplies the content-type/body
/// for [`EgressResponse::Ready`] (protocol-specific: LL-HLS's `LlHlsBody`'s
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
    match resp {
        EgressResponse::Ready { body, .. } => render_ready(body),
        EgressResponse::NotFound => not_found_status.into_response(),
        EgressResponse::BadRequest { .. } => StatusCode::BAD_REQUEST.into_response(),
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
