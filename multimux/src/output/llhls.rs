//! `LlHlsOutput`: the LL-HLS [`crate::output::Output`] implementation — a
//! thin tokio+axum adapter over the sans-IO LL-HLS origin engine
//! ([`ll_hls_runtime::server`], issue #663/#717 Stage 2): axum routes for the
//! master/media playlists, translating
//! [`ll_hls_runtime::server::MediaStore::resolve_playlist`]'s `Ready`/
//! `WouldBlock`/`BadRequest` outcomes into real HTTP responses — including
//! the actual bounded `.await` on a `WouldBlock`, which is the one thing the
//! sans-IO engine can't do itself. The init/segment/part byte ranges these
//! playlists reference are served by the origin's *shared* resource route
//! (`crate::origin::resource`), not here — issue #663 P4 moved that out of
//! this per-output module since DASH references the exact same bytes (see
//! `crate::output` module docs for why).
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
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ll_hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, PlaylistOutcome, master_playlist_m3u8,
};
use serde::Deserialize;

use crate::origin::resource::{BlockingRequestGuard, cors_preflight};
use crate::output::{Output, OutputKind};
use crate::store::MediaStore;

// Re-exported so existing `crate::output::llhls::media_playlist_m3u8(..)` call
// sites (e.g. `crate::pipeline`'s own tests) keep working unchanged — the
// renderer itself now lives in `ll_hls_runtime::server` alongside the
// `MediaStore` it renders from.
pub use ll_hls_runtime::server::media_playlist_m3u8;

/// Upper bound on how long a blocking `media.m3u8` request (`_HLS_msn`/
/// `_HLS_part`) waits for the requested segment/part before falling back to
/// rendering the playlist as it currently is. RFC 8216bis §6.2.5.2 requires
/// the origin to eventually respond either way — this cap keeps a stalled/
/// slow source from hanging the HTTP response forever. This is the one clock
/// the sans-IO engine ([`ll_hls_runtime::server`]) doesn't have — it lives
/// here, in the adapter. Mirrors `crate::origin::resource`'s own
/// `BLOCKING_RELOAD_TIMEOUT` for the resource-byte-serving wait.
const BLOCKING_RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";

/// Default media-playlist filename (issue #663 "configurable `playlist_name`")
/// — every pre-existing `LlHlsOutput::default()`/`OutputKind::build()` call
/// site keeps serving `/media.m3u8` unchanged.
pub const DEFAULT_PLAYLIST_NAME: &str = "media.m3u8";

/// The LL-HLS [`Output`]: master/media playlists over a shared [`MediaStore`].
/// Init/segment/part byte ranges are the origin's shared resource route, not
/// this one — see the module docs.
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

/// The axum state for [`LlHlsOutput`]'s manifest routes: the shared
/// [`MediaStore`] plus this instance's configured media-playlist filename
/// (needed by [`master_playlist`] to render the correct `#EXT-X-STREAM-INF`
/// reference, and by [`LlHlsOutput::manifest_routes`] to mount
/// [`media_playlist`] under the right path).
#[derive(Clone)]
pub(crate) struct LlHlsState {
    store: Arc<MediaStore>,
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
    fn manifest_routes(&self, store: Arc<MediaStore>) -> Router {
        let state = LlHlsState {
            store,
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
///
/// `pub(crate)` (narrowed from `pub`, issue #663 "configurable
/// `playlist_name`"): its `State` type is now the crate-private
/// [`LlHlsState`] (store + playlist name) rather than the previously bare
/// `Arc<MediaStore>`, and nothing outside this crate called the handler
/// directly (only through the router `LlHlsOutput::manifest_routes` builds).
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
        // `PlaylistOutcome` is `#[non_exhaustive]` — treat any future variant
        // this adapter doesn't yet know how to render as a bad request
        // rather than blocking forever or fabricating a playlist body.
        _ => return Err(()),
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
                // `PlaylistOutcome` is `#[non_exhaustive]` — an unrecognized
                // future variant is treated the same as `BadRequest` (give up
                // on this wait rather than looping/blocking on a condition
                // this adapter cannot evaluate).
                _ => return None,
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
pub(crate) async fn media_playlist(
    State(state): State<LlHlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    match media_playlist_blocking(&state.store, DEFAULT_TRACK_ID, q.into()).await {
        Ok(body) => ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response(),
        Err(()) => StatusCode::BAD_REQUEST.into_response(),
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

    /// Wraps `store` into an [`LlHlsState`] using the default playlist name —
    /// the shape the handlers under test actually receive as `State`.
    fn state(store: Arc<MediaStore>) -> LlHlsState {
        LlHlsState {
            store,
            playlist_name: DEFAULT_PLAYLIST_NAME.to_string(),
        }
    }

    #[tokio::test]
    async fn master_playlist_ok() {
        let store = make_store();
        let resp = master_playlist(State(state(store))).await;
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
        let store = make_store();
        let resp = master_playlist(State(LlHlsState {
            store,
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
        let store = make_store();
        let resp = media_playlist(State(state(store)), Query(BlockingReloadQuery::default())).await;
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
            State(state(store)),
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
            State(state(store)),
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
            State(state(store)),
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
            State(state(store)),
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
            State(state(store)),
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
            State(state(store)),
            Query(BlockingReloadQuery {
                hls_msn: None,
                hls_part: Some(0),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// `manifest_routes`' own `OPTIONS` preflight (the `Access-Control-*`
    /// header *values* are the origin's shared `add_response_headers`
    /// middleware's job now — see `crate::origin::mod`'s tests — but the
    /// route itself, and that it 204s rather than 404ing, is this output's
    /// responsibility).
    #[tokio::test]
    async fn options_preflight_returns_no_content() {
        let store = make_store();
        let router = LlHlsOutput::default().manifest_routes(store);
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
