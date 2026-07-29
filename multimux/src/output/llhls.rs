//! `LlHlsOutput`: the LL-HLS [`crate::output::Output`] implementation — a
//! thin tokio+axum adapter over the sans-IO LL-HLS origin engine
//! ([`ll_hls_runtime::server::LlHlsOrigin`], plan step 4): axum routes for
//! the master/media playlists, resolved through the **one** shared adapter
//! (`crate::http::resolve_blocking`/`crate::http::into_response`) —
//! including the actual bounded `.await` on an
//! [`media_plane::egress::EgressResponse::Await`], which the sans-IO engine
//! can't do itself. The init/segment/part byte ranges these playlists
//! reference are served by the origin's *shared* resource route
//! (`crate::origin::resource`), not here — issue #663 P4 moved that out of
//! this per-output module since DASH references the exact same bytes (see
//! `crate::output` module docs for why).
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`, rendered by
//! [`ll_hls_runtime::server::master_playlist_m3u8`]); the blocking reload
//! query parameters (`_HLS_msn`/`_HLS_part`) are the Blocking Playlist Reload
//! mechanism of RFC 8216bis §6.2.5.2 — the client asks the origin to hold the
//! response open until the requested Media Sequence Number/part is
//! available, bounded by `crate::http::BLOCKING_RELOAD_TIMEOUT` so the
//! origin never hangs indefinitely.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ll_hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, LlHlsBody, LlHlsRequest, master_playlist_m3u8,
};
use serde::Deserialize;

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::origin::resource::{BlockingRequestGuard, cors_preflight};
use crate::output::{Output, OutputKind};
use crate::route::RouteHandle;

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";

/// Default media-playlist filename (issue #663 "configurable `playlist_name`")
/// — every pre-existing `LlHlsOutput::default()`/`OutputKind::build()` call
/// site keeps serving `/media.m3u8` unchanged.
pub const DEFAULT_PLAYLIST_NAME: &str = "media.m3u8";

/// The LL-HLS [`Output`]: master/media playlists over a shared
/// [`RouteHandle`]. Init/segment/part byte ranges are the origin's shared
/// resource route, not this one — see the module docs.
///
/// [`Self::new`] serves the media playlist under a caller-chosen filename
/// (`crate::config::Config::playlist_name`) — `master.m3u8` always points at
/// whichever name this instance was built with. [`Default`] (and therefore
/// [`OutputKind::build`]) uses [`DEFAULT_PLAYLIST_NAME`].
pub struct LlHlsOutput {
    playlist_name: String,
}

impl Default for LlHlsOutput {
    fn default() -> Self {
        LlHlsOutput::new(DEFAULT_PLAYLIST_NAME)
    }
}

impl LlHlsOutput {
    /// Serves the media playlist at `/{playlist_name}` instead of the
    /// default `/media.m3u8` (`master.m3u8`'s `#EXT-X-STREAM-INF` reference
    /// follows suit — see `master_playlist`).
    pub fn new(playlist_name: impl Into<String>) -> Self {
        LlHlsOutput {
            playlist_name: playlist_name.into(),
        }
    }
}

/// The axum state for [`LlHlsOutput`]'s manifest routes: the shared `route`
/// plus this instance's configured media-playlist filename (needed by
/// [`master_playlist`] to render the correct `#EXT-X-STREAM-INF` reference,
/// and by [`LlHlsOutput::manifest_routes`] to mount [`media_playlist`] under
/// the right path).
#[derive(Clone)]
pub(crate) struct LlHlsState {
    route: Arc<RouteHandle>,
    playlist_name: String,
}

impl Output for LlHlsOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::LlHls
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /master.m3u8` — minimal single-variant master playlist.
    /// - `GET /{playlist_name}` — LL-HLS media playlist, blocking-reload
    ///   aware (`/media.m3u8` unless [`LlHlsOutput::new`] configured a
    ///   different name).
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        let state = LlHlsState {
            route,
            playlist_name: self.playlist_name.clone(),
        };
        Router::new()
            .route("/master.m3u8", get(master_playlist).options(cors_preflight))
            .route(
                &format!("/{}", self.playlist_name),
                get(media_playlist).options(cors_preflight),
            )
            .with_state(state)
    }
}

/// `GET /master.m3u8` — a minimal single-variant master playlist pointing at
/// this route's configured media-playlist filename.
pub(crate) async fn master_playlist(State(state): State<LlHlsState>) -> Response {
    (
        [(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)],
        master_playlist_m3u8(&state.playlist_name),
    )
        .into_response()
}

/// Blocking playlist reload query parameters (RFC 8216bis §6.2.5.2), as
/// deserialized from the HTTP query string — the wire-format counterpart of
/// [`ll_hls_runtime::server::BlockingQuery`], which `media_playlist` maps
/// this into before handing off to the sans-IO engine.
#[derive(Debug, Default, Deserialize)]
pub struct BlockingReloadQuery {
    /// The Media Sequence Number the client already has, plus one — the
    /// origin should not respond until a segment/part beyond this is ready.
    #[serde(rename = "_HLS_msn")]
    pub hls_msn: Option<u64>,
    /// The part index (within `_HLS_msn`) the client is waiting for.
    #[serde(rename = "_HLS_part")]
    pub hls_part: Option<u32>,
}

impl From<BlockingReloadQuery> for BlockingQuery {
    fn from(q: BlockingReloadQuery) -> Self {
        BlockingQuery {
            hls_msn: q.hls_msn,
            hls_part: q.hls_part,
        }
    }
}

/// `GET /media.m3u8` — the LL-HLS media playlist for [`DEFAULT_TRACK_ID`],
/// blocking on `_HLS_msn`/`_HLS_part` when present (via
/// [`crate::http::resolve_blocking`]).
///
/// RFC 8216bis §6.2.5.2 abuse-prevention (SHOULD/MUST): `_HLS_part` without
/// `_HLS_msn` is meaningless (a part is only addressable relative to a
/// segment) and a `_HLS_msn` unreasonably far beyond the current live edge is
/// either a broken client or abuse — both are rejected with `400 Bad
/// Request` immediately (`LlHlsOrigin::resolve` answers `BadRequest`, which
/// [`http::into_response`] maps to `400` with no wait at all), rather than
/// blocking to the blocking-reload timeout and returning `200` regardless.
pub(crate) async fn media_playlist(
    State(state): State<LlHlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    let serving = match http::resolve_route_program(&state.route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let ll_hls = serving.ll_hls();
    let request = LlHlsRequest::Playlist {
        track_id: DEFAULT_TRACK_ID,
        query: q.into(),
    };
    let resp = http::resolve_blocking(
        &trunk,
        ll_hls.as_ref(),
        request,
        BLOCKING_RELOAD_TIMEOUT,
        BlockingRequestGuard::new,
    )
    .await;
    http::into_response(resp, StatusCode::NOT_FOUND, |body| match body {
        LlHlsBody::Playlist(m) => {
            ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], m).into_response()
        }
        LlHlsBody::Resource(_) => StatusCode::NOT_FOUND.into_response(),
        // `LlHlsBody` is `#[non_exhaustive]`; a future body variant this
        // playlist route doesn't understand is treated the same as a
        // resource body -- not found here.
        _ => StatusCode::NOT_FOUND.into_response(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteHandle;
    use std::time::Duration;
    use transmux::ll_hls::{PartInfo, SegmentInfo};

    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![0x10 + idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    fn seg(seq: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![0x20 + seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: 2,
        }
    }

    /// A populated route: a closed segment 1, plus two live parts of
    /// in-progress segment 2 -- so the live edge is `(2, 2)`. Publishes
    /// `SPTS_PROGRAM_ID` into the registry first (`publish_new_program`) so
    /// `media_playlist`'s `resolve_route_program` lookup (issue #805 tasks
    /// 3/6) sees `Found`, exactly as a real driver-backed route's
    /// `crate::source::report_driver_progress` call does by the time it
    /// serves anything -- and so there is a `ProgramServing` bundle for
    /// `set_init`/`add_segment`/`add_part` to write into at all.
    fn make_route() -> Arc<RouteHandle> {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 8]);
        route.add_segment(crate::route::SPTS_PROGRAM_ID, seg(1));
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 0));
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 1));
        route
    }

    async fn body_string(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Wraps `route` into an [`LlHlsState`] using the default playlist name —
    /// the shape the handlers under test actually receive as `State`.
    fn state(route: Arc<RouteHandle>) -> LlHlsState {
        LlHlsState {
            route,
            playlist_name: DEFAULT_PLAYLIST_NAME.to_string(),
        }
    }

    #[tokio::test]
    async fn master_playlist_ok() {
        let route = make_route();
        let resp = master_playlist(State(state(route))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXTM3U"));
        assert!(body.contains("#EXT-X-STREAM-INF"));
        assert!(body.contains("media.m3u8"));
    }

    /// Biting test (issue #663 "configurable `playlist_name`"): a
    /// non-default playlist name must appear in the master playlist's
    /// `#EXT-X-STREAM-INF` reference, and the default name must not.
    #[tokio::test]
    async fn master_playlist_points_at_configured_playlist_name() {
        let route = make_route();
        let resp = master_playlist(State(LlHlsState {
            route,
            playlist_name: "index.m3u8".to_string(),
        }))
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("index.m3u8"), "body: {body}");
        assert!(!body.contains("media.m3u8"), "body: {body}");
    }

    #[tokio::test]
    async fn media_playlist_no_query_renders_now() {
        let route = make_route();
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXT-X-PART"), "body: {body}");
    }

    #[tokio::test]
    async fn media_playlist_already_satisfied_blocking_request_resolves_immediately() {
        // Live edge is (2, 2): asking for msn=1 (an earlier segment) is
        // already satisfied and must not wait.
        let route = make_route();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(1),
                hls_part: Some(0),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_playlist_already_satisfied_same_msn_lower_part() {
        // in_progress_seg_seq == msn and live parts(2) > part(1): satisfied.
        let route = make_route();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: Some(1),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_playlist_msn_only_waits_for_closed_segment_not_just_open_parts() {
        // make_route()'s segment 2 is OPEN with 2 live parts but not yet
        // CLOSED. RFC 8216bis §6.2.5.2: a bare `_HLS_msn=2` (no `_HLS_part`)
        // must wait for segment 2 to actually close, not resolve merely
        // because it has live parts.
        let route = make_route();
        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            route_for_task.add_segment(crate::route::SPTS_PROGRAM_ID, seg(2)); // closes segment 2
        });

        let started = std::time::Instant::now();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            started.elapsed() >= Duration::from_millis(70),
            "must have waited for segment 2 to close, not returned as soon as \
             it had live parts: elapsed {:?}",
            started.elapsed()
        );
        let body = body_string(resp).await;
        assert!(
            body.contains("seg-1-2.m4s"),
            "resolved playlist must show segment 2 as a closed, fetchable segment: {body}"
        );
    }

    #[tokio::test]
    async fn media_playlist_far_future_msn_rejected_400_fast() {
        // Live edge is (2, 2). A `_HLS_msn` 1000 ahead of it is not a
        // legitimate blocking-reload request (RFC 8216bis §6.2.5.2 abuse
        // prevention) — it must 400 immediately, not consume the full
        // BLOCKING_RELOAD_TIMEOUT before giving up.
        let route = make_route();
        let started = std::time::Instant::now();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(1002),
                hls_part: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "must reject promptly, not block out the 5s timeout: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn media_playlist_msn_within_bound_still_blocks_normally() {
        // Sanity check for the abuse-bound change: a legitimate
        // just-ahead-of-live-edge msn must still work as before (block, then
        // resolve), not get swept up by the new bound check.
        let route = make_route(); // (2, 2)
        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            route_for_task.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 2));
        });
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: Some(2),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_playlist_part_without_msn_rejected_400() {
        // RFC 8216bis §6.2.5.2: `_HLS_part` without `_HLS_msn` is
        // meaningless (a part is only addressable relative to a segment).
        let route = make_route();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: None,
                hls_part: Some(0),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// MUTATION VERIFIED (issue #805 task 4): a route with no program
    /// announced yet (never called `publish_new_program`/`publish_program`)
    /// must answer `503 Service Unavailable` — a wait/not-ready signal — not
    /// `404 Not Found`. Changing `http::resolve_route_program`'s
    /// `ProgramResolution::NotYetAnnounced` arm to `Err(StatusCode::NOT_FOUND.into_response())`
    /// makes this test fail: `assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE)`
    /// fails, comparing actual `404 Not Found` against expected
    /// `503 Service Unavailable` — collapsing "not yet announced" into "gone"
    /// would make a route mid-connect indistinguishable from one that will
    /// never exist. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[tokio::test]
    async fn media_playlist_not_yet_announced_is_503_not_404() {
        // A bare route: never `publish_new_program`/`publish_program`d, so
        // its registry is empty -- exactly the window between a driver-backed
        // route connecting and its first `SessionEvent::NewProgram`.
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a route with no program announced yet must be 503 (not ready), not 404 (gone)"
        );
    }

    /// `manifest_routes`' own `OPTIONS` preflight (the `Access-Control-*`
    /// header *values* are the origin's shared `add_response_headers`
    /// middleware's job now — see `crate::origin::mod`'s tests — but the
    /// route itself, and that it 204s rather than 404ing, is this output's
    /// responsibility).
    #[tokio::test]
    async fn options_preflight_returns_no_content() {
        let route = make_route();
        let router = LlHlsOutput::default().manifest_routes(route);
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
