//! `LlHlsOutput`: the LL-HLS [`crate::output::Output`] implementation — a
//! thin tokio+axum adapter over the sans-IO LL-HLS origin engine
//! ([`ll_hls_runtime::server`], issue #663/#717 Stage 2): axum routes for the
//! master/media playlists and the init/segment/part byte ranges they
//! reference, translating [`ll_hls_runtime::server::MediaStore::resolve_playlist`]/
//! [`ll_hls_runtime::server::MediaStore::resolve_resource`]'s `Ready`/
//! `WouldBlock`/`BadRequest`/`NotFound` outcomes into real HTTP responses —
//! including the actual bounded `.await` on a `WouldBlock`, which is the one
//! thing the sans-IO engine can't do itself.
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`, rendered by
//! [`ll_hls_runtime::server::media_playlist_m3u8`]); the blocking reload
//! query parameters (`_HLS_msn`/`_HLS_part`) are the Blocking Playlist Reload
//! mechanism of RFC 8216bis §6.2.5.2 — the client asks the origin to hold the
//! response open until the requested Media Sequence Number/part is
//! available, bounded by a 5 s timeout (`BLOCKING_RELOAD_TIMEOUT`) so the
//! origin never hangs indefinitely.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ll_hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, PlaylistOutcome, ResourceOutcome, master_playlist_m3u8,
};
use serde::Deserialize;

use crate::output::Output;
use crate::store::MediaStore;

// Re-exported so existing `crate::output::llhls::media_playlist_m3u8(..)` call
// sites (e.g. `crate::pipeline`'s own tests) keep working unchanged — the
// renderer itself now lives in `ll_hls_runtime::server` alongside the
// `MediaStore` it renders from.
pub use ll_hls_runtime::server::media_playlist_m3u8;

/// Upper bound on how long a blocking `media.m3u8`/dynamic-file request
/// (`_HLS_msn`/`_HLS_part`, or a preload-hinted part) waits for the requested
/// data before falling back (playlist: render whatever is currently
/// available; resource: `404`). RFC 8216bis §6.2.5.2 requires the origin to
/// eventually respond either way — this cap keeps a stalled/slow source from
/// hanging the HTTP response forever. This is the one clock the sans-IO
/// engine ([`ll_hls_runtime::server`]) doesn't have — it lives here, in the
/// adapter.
const BLOCKING_RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";
const MP4_CONTENT_TYPE: &str = "video/mp4";

/// `Cache-Control` for playlists (`master.m3u8`/`media.m3u8`): they must
/// always be re-fetched for liveness, never served stale from a cache.
const CACHE_CONTROL_PLAYLIST: &str = "no-cache";

/// `Cache-Control` for init/segment/part byte ranges: once produced, a given
/// URI's bytes never change (each segment/part is generated exactly once
/// under a unique filename), so these are safe to cache indefinitely. Mirrors
/// [`ll_hls_runtime::server::CachePolicy::Immutable`], which every
/// [`ResourceOutcome::Ready`] this adapter serves carries.
const CACHE_CONTROL_IMMUTABLE: &str = "max-age=31536000, immutable";

/// The LL-HLS [`Output`]: master/media playlists + init/segment/part byte
/// ranges, over a shared [`MediaStore`].
pub struct LlHlsOutput;

impl Output for LlHlsOutput {
    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /master.m3u8` — minimal single-variant master playlist.
    /// - `GET /media.m3u8` — LL-HLS media playlist, blocking-reload aware.
    /// - `GET /:file` — catch-all serving `init-*.mp4`/`seg-*.m4s`/
    ///   `part-*.m4s` byte ranges (see [`dynamic_file`] for why a single
    ///   catch-all is used instead of per-filename routes).
    fn router(&self, store: Arc<MediaStore>) -> Router {
        Router::new()
            .route("/master.m3u8", get(master_playlist).options(cors_preflight))
            .route("/media.m3u8", get(media_playlist).options(cors_preflight))
            .route("/:file", get(dynamic_file).options(cors_preflight))
            .with_state(store)
            .layer(middleware::from_fn(add_response_headers))
    }
}

/// `OPTIONS` preflight handler for every LL-HLS route: browsers (hls.js and
/// friends, per-origin from the API) send a CORS preflight before the real
/// `GET` for cross-origin requests with custom headers. Returns `204 No
/// Content` with no body; [`add_response_headers`] (mounted below as a
/// router-wide layer) adds the actual `Access-Control-Allow-*` headers to
/// this response the same as every other response this router serves.
async fn cors_preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Router-wide middleware (mounted via `.layer` in [`LlHlsOutput::router`],
/// so it wraps every route including the `:file` catch-all's 404 fallback):
/// adds `Access-Control-Allow-*` (permissive CORS — LL-HLS players are
/// commonly browsers on a different origin than the API, e.g. hls.js) and a
/// `Cache-Control` appropriate to the resource kind — `no-cache` for
/// playlists (must always be re-fetched for liveness), `max-age=31536000,
/// immutable` for init/segment/part byte ranges (a produced segment/part
/// never changes) — to every response this router serves. This is the
/// adapter's application of [`ll_hls_runtime::server::CachePolicy`]: every
/// non-`.m3u8` route this router serves is a resolved [`ResourceOutcome::Ready`]
/// carrying `CachePolicy::Immutable`, and every `.m3u8` route is a rendered
/// playlist (implicitly `NoCache` — playlists never carry a cache policy of
/// their own since they're always liveness-sensitive).
async fn add_response_headers(req: Request, next: Next) -> Response {
    let is_playlist = req.uri().path().ends_with(".m3u8");
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
        HeaderValue::from_static(if is_playlist {
            CACHE_CONTROL_PLAYLIST
        } else {
            CACHE_CONTROL_IMMUTABLE
        }),
    );
    resp
}

/// `GET /master.m3u8` — a minimal single-variant master playlist pointing at
/// `media.m3u8`.
pub async fn master_playlist(State(_store): State<Arc<MediaStore>>) -> Response {
    (
        [(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)],
        master_playlist_m3u8(),
    )
        .into_response()
}

/// Blocking playlist reload query parameters (RFC 8216bis §6.2.5.2), as
/// deserialized from the HTTP query string — the wire-format counterpart of
/// [`ll_hls_runtime::server::BlockingQuery`], which [`media_playlist`] maps
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

/// RAII guard bumping/dropping [`crate::prometheus::ACTIVE_BLOCKING_REQUESTS`]
/// for the lifetime of a blocking LL-HLS wait ([`media_playlist_blocking`]/
/// [`resource_blocking`]) — incremented on construction, decremented on drop,
/// so the gauge stays accurate even if the awaited future is itself dropped
/// (e.g. the client disconnects mid-wait), not just on a normal return.
struct BlockingRequestGuard;

impl BlockingRequestGuard {
    fn new() -> Self {
        metrics::gauge!(crate::prometheus::ACTIVE_BLOCKING_REQUESTS).increment(1.0);
        BlockingRequestGuard
    }
}

impl Drop for BlockingRequestGuard {
    fn drop(&mut self) {
        metrics::gauge!(crate::prometheus::ACTIVE_BLOCKING_REQUESTS).decrement(1.0);
    }
}

/// Resolve `store`'s media playlist for `track_id`/`query`, waiting
/// (bounded by [`BLOCKING_RELOAD_TIMEOUT`]) on a
/// [`PlaylistOutcome::WouldBlock`] rather than rendering immediately.
///
/// This is the caller-driven wait loop
/// [`ll_hls_runtime::server`]'s module docs describe: register a listener
/// *before* re-checking [`MediaStore::resolve_playlist`] (no missed-wakeup
/// race — see [`MediaStore::listen`]), so a notification that lands between
/// the initial `WouldBlock` and the listener's registration is never lost.
///
/// `Ok(body)` for a satisfied (or eventually-timed-out — RFC 8216bis
/// §6.2.5.2 requires the origin to eventually respond either way, so a
/// timeout still renders the playlist as it currently is, rather than
/// erroring) request; `Err(())` for [`PlaylistOutcome::BadRequest`], which
/// short-circuits immediately with no wait (a malformed/abusive request
/// should never be held open to the timeout).
async fn media_playlist_blocking(
    store: &MediaStore,
    track_id: u32,
    query: BlockingQuery,
) -> Result<String, ()> {
    match store.resolve_playlist(track_id, query) {
        PlaylistOutcome::Ready(body) => return Ok(body),
        PlaylistOutcome::BadRequest => return Err(()),
        PlaylistOutcome::WouldBlock => {}
    }
    let _guard = BlockingRequestGuard::new();
    let wait = async {
        loop {
            let listener = store.listen();
            match store.resolve_playlist(track_id, query) {
                PlaylistOutcome::Ready(body) => return Some(body),
                // Can't actually recur once the initial check above passed
                // (the abuse bound only ever grows less strict as the live
                // edge advances), but handled structurally rather than
                // assumed.
                PlaylistOutcome::BadRequest => return None,
                PlaylistOutcome::WouldBlock => {}
            }
            listener.await;
        }
    };
    let resolved = tokio::time::timeout(BLOCKING_RELOAD_TIMEOUT, wait)
        .await
        .ok()
        .flatten();
    Ok(resolved.unwrap_or_else(|| media_playlist_m3u8(store, track_id)))
}

/// Resolve `store`'s dynamic resource `name`, waiting (bounded by
/// [`BLOCKING_RELOAD_TIMEOUT`]) on a [`ResourceOutcome::WouldBlock`] (a
/// preload-hinted part not yet produced) rather than 404ing immediately.
/// Same caller-driven wait-loop shape as [`media_playlist_blocking`]. On
/// timeout, falls back to [`ResourceOutcome::NotFound`] (a `404`) — unlike
/// the playlist case, there is no "current" resource to serve instead.
async fn resource_blocking(store: &MediaStore, name: &str) -> ResourceOutcome {
    match store.resolve_resource(name) {
        ResourceOutcome::WouldBlock => {}
        terminal => return terminal,
    }
    let _guard = BlockingRequestGuard::new();
    let wait = async {
        loop {
            let listener = store.listen();
            match store.resolve_resource(name) {
                ResourceOutcome::WouldBlock => {}
                terminal => return terminal,
            }
            listener.await;
        }
    };
    tokio::time::timeout(BLOCKING_RELOAD_TIMEOUT, wait)
        .await
        .unwrap_or(ResourceOutcome::NotFound)
}

/// `GET /media.m3u8` — the LL-HLS media playlist for [`DEFAULT_TRACK_ID`],
/// blocking on `_HLS_msn`/`_HLS_part` when present.
///
/// RFC 8216bis §6.2.5.2 abuse-prevention (SHOULD/MUST): `_HLS_part` without
/// `_HLS_msn` is meaningless (a part is only addressable relative to a
/// segment) and a `_HLS_msn` unreasonably far beyond the current live edge is
/// either a broken client or abuse — both are rejected with `400 Bad
/// Request` immediately, rather than blocking to the blocking-reload timeout
/// and returning `200` regardless (which gives a misbehaving client no
/// signal to back off). See [`ll_hls_runtime::server::MediaStore::resolve_playlist`]
/// for the decision logic itself.
pub async fn media_playlist(
    State(store): State<Arc<MediaStore>>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    match media_playlist_blocking(&store, DEFAULT_TRACK_ID, q.into()).await {
        Ok(body) => ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response(),
        Err(()) => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// `GET /:file` — catch-all for the dynamic init/segment/part filenames
/// [`media_playlist_m3u8`] emits.
///
/// A single catch-all (rather than three routes with per-filename literals)
/// because axum 0.7's `matchit`-based router cannot mix multiple params with
/// literal text in one path segment (e.g. `seg-:track-:seq.m4s`) — only one
/// param per segment is supported, capturing the whole segment. Parsing
/// `file` into a segment/part/init lookup — including the "block until a
/// preload-hinted part is produced" behaviour (RFC 8216bis §6.2.2, §6.3.1) —
/// is [`ll_hls_runtime::server::MediaStore::resolve_resource`]'s job; this
/// handler only drives the wait (`resource_blocking`) and maps the outcome
/// to an HTTP response.
pub async fn dynamic_file(
    State(store): State<Arc<MediaStore>>,
    Path(file): Path<String>,
) -> Response {
    match resource_blocking(&store, &file).await {
        ResourceOutcome::Ready { bytes, .. } => {
            ([(header::CONTENT_TYPE, MP4_CONTENT_TYPE)], bytes).into_response()
        }
        ResourceOutcome::NotFound | ResourceOutcome::WouldBlock => {
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// A populated store: a closed segment 1, plus two live parts of
    /// in-progress segment 2 -- so `latest_progress()` (via `resolve_playlist`)
    /// treats the store as `(2, 2)`.
    fn make_store() -> Arc<MediaStore> {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 8]);
        store.add_segment(seg(1));
        store.add_part(part(2, 0));
        store.add_part(part(2, 1));
        store
    }

    async fn body_string(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn master_playlist_ok() {
        let store = make_store();
        let resp = master_playlist(State(store)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXTM3U"));
        assert!(body.contains("#EXT-X-STREAM-INF"));
        assert!(body.contains("media.m3u8"));
    }

    #[tokio::test]
    async fn media_playlist_no_query_renders_now() {
        let store = make_store();
        let resp = media_playlist(State(store), Query(BlockingReloadQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXT-X-PART"), "body: {body}");
    }

    #[tokio::test]
    async fn media_playlist_already_satisfied_blocking_request_resolves_immediately() {
        // Store state is (2, 2): asking for msn=1 (an earlier segment) is
        // already satisfied and must not wait.
        let store = make_store();
        let resp = media_playlist(
            State(store),
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
        // in_progress_seg_seq == msn and part_count(2) > part(1): satisfied.
        let store = make_store();
        let resp = media_playlist(
            State(store),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: Some(1),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn dynamic_file_init_present() {
        let store = make_store();
        let resp = dynamic_file(State(store), Path("init-1.mp4".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0xAA; 8]);
    }

    #[tokio::test]
    async fn dynamic_file_segment_present_and_absent() {
        let store = make_store();
        let ok = dynamic_file(State(store.clone()), Path("seg-1-1.m4s".to_string())).await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(body_bytes(ok).await, vec![0x21; 8]);

        let missing = dynamic_file(State(store), Path("seg-1-99.m4s".to_string())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_part_present() {
        let store = make_store();
        let resp = dynamic_file(State(store), Path("part-1-2.0.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0x10; 4]);
    }

    #[tokio::test]
    async fn dynamic_file_part_blocks_until_available_then_serves() {
        // part-1-2.2 is the preload-hinted next part of in-progress segment 2
        // (which currently has parts .0 and .1). The request must BLOCK until
        // the part is produced, not 404 immediately. Produce it after a short
        // delay from another task, then assert the handler returned its bytes.
        let store = make_store();
        let store_for_task = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_task.add_part(part(2, 2));
        });
        let resp = dynamic_file(State(store), Path("part-1-2.2.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "part request must block until the part is produced, not 404"
        );
        assert_eq!(body_bytes(resp).await, vec![0x12; 4]); // 0x10 + idx(2)
    }

    #[tokio::test]
    async fn dynamic_file_part_404_promptly_when_segment_closes_without_it() {
        // part-1-2.9 will never be produced. When segment 2 closes (advancing
        // the in-progress segment), the handler must 404 promptly — not hang
        // until the blocking timeout.
        let store = make_store();
        let store_for_task = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_task.add_segment(seg(2)); // closes segment 2
        });
        let started = std::time::Instant::now();
        let resp = dynamic_file(State(store), Path("part-1-2.9.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(
            started.elapsed() < BLOCKING_RELOAD_TIMEOUT,
            "must 404 promptly on segment close, not wait out the timeout"
        );
    }

    #[tokio::test]
    async fn dynamic_file_part_served_from_recent_after_close() {
        // Segment 2 has live parts .0 and .1; close it. Its final part must
        // still be served (from recent_parts) — the in-flight preload-hint
        // request that races the segment close must not 404.
        let store = make_store();
        store.add_segment(seg(2)); // close segment 2, moving its parts to recent_parts
        let resp = dynamic_file(State(store), Path("part-1-2.1.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a just-closed segment's part must still be served, not 404"
        );
        assert_eq!(body_bytes(resp).await, vec![0x11; 4]); // part(2,1): 0x10 + idx(1)
    }

    #[tokio::test]
    async fn dynamic_file_part_of_old_segment_404() {
        // Segment 1 closed in make_store() with no parts recorded and is old
        // enough to be past the recent-parts retention window, so a request for
        // one of its parts 404s without blocking (it will never be produced and
        // isn't individually addressable anymore).
        let store = make_store();
        let resp = dynamic_file(State(store), Path("part-1-1.0.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_unmatched_filename_404() {
        let store = make_store();
        let resp = dynamic_file(State(store), Path("not-a-thing.txt".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_playlist_msn_only_waits_for_closed_segment_not_just_open_parts() {
        // make_store()'s segment 2 is OPEN with 2 live parts (part_count == 2)
        // but not yet CLOSED. RFC 8216bis §6.2.5.2: a bare `_HLS_msn=2` (no
        // `_HLS_part`) must wait for segment 2 to actually close, not resolve
        // merely because it has live parts.
        let store = make_store();
        let store_for_task = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            store_for_task.add_segment(seg(2)); // closes segment 2
        });

        let started = std::time::Instant::now();
        let resp = media_playlist(
            State(store),
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
        // Store state is (2, 2). A `_HLS_msn` 1000 ahead of the live edge is
        // not a legitimate blocking-reload request (RFC 8216bis §6.2.5.2 abuse
        // prevention) — it must 400 immediately, not consume the full
        // BLOCKING_RELOAD_TIMEOUT before giving up.
        let store = make_store();
        let started = std::time::Instant::now();
        let resp = media_playlist(
            State(store),
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
        let store = make_store(); // (2, 2)
        let store_for_task = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_task.add_part(part(2, 2));
        });
        let resp = media_playlist(
            State(store),
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
        let store = make_store();
        let resp = media_playlist(
            State(store),
            Query(BlockingReloadQuery {
                hls_msn: None,
                hls_part: Some(0),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn segment_response_carries_immutable_cache_control_and_cors() {
        let store = make_store();
        let router = LlHlsOutput.router(store);
        let req = axum::http::Request::builder()
            .uri("/seg-1-1.m4s")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            CACHE_CONTROL_IMMUTABLE,
            "segment responses must be immutably cacheable"
        );
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*",
            "segment responses must carry permissive CORS"
        );
    }

    #[tokio::test]
    async fn media_playlist_response_carries_no_cache_and_cors() {
        let store = make_store();
        let router = LlHlsOutput.router(store);
        let req = axum::http::Request::builder()
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            CACHE_CONTROL_PLAYLIST,
            "playlist responses must always be re-fetched for liveness"
        );
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*",
            "playlist responses must carry permissive CORS"
        );
    }

    #[tokio::test]
    async fn options_preflight_returns_no_content_with_cors_headers() {
        let store = make_store();
        let router = LlHlsOutput.router(store);
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*"
        );
    }
}
