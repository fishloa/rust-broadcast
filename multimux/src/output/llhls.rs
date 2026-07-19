//! `LlHlsOutput`: the LL-HLS [`crate::output::Output`] implementation —
//! media playlist rendering, request handlers for the master/media playlists
//! and the init/segment/part byte ranges they reference, and the axum routes
//! that serve them.
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`, rendered by [`media_playlist_m3u8`]);
//! the blocking reload query parameters (`_HLS_msn`/`_HLS_part`) are the
//! Blocking Playlist Reload mechanism of RFC 8216bis §6.2.5.2 — the client
//! asks the origin to hold the response open until the requested Media
//! Sequence Number/part is available, bounded so the origin never hangs
//! indefinitely.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, OpenSegment, PartSpec};

use crate::output::Output;
use crate::store::MediaStore;

/// Track id for the single rendition multimux currently serves per stream
/// (no multi-track/multi-rendition support yet).
pub const DEFAULT_TRACK_ID: u32 = 1;

/// Placeholder `BANDWIDTH` (bits/second) advertised in the master playlist's
/// `#EXT-X-STREAM-INF` — multimux does not yet measure actual encoded
/// bitrate, so a single fixed estimate is used for the single variant served.
const PLACEHOLDER_BANDWIDTH_BPS: u64 = 5_000_000;

/// Upper bound on how long a blocking `media.m3u8` request
/// (`_HLS_msn`/`_HLS_part`) waits for the requested part/segment before
/// falling back to rendering whatever is currently available. RFC 8216bis
/// §6.2.5.2 requires the origin to eventually respond either way — this cap
/// keeps a stalled/slow source from hanging the HTTP response forever.
const BLOCKING_RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// RFC 8216bis §6.2.5.2 (SHOULD): the origin should reject a `_HLS_msn` that
/// is unreasonably far in the future rather than always blocking to the
/// timeout — a legitimate client only ever asks for the segment/part right
/// after the one it already has, so anything more than a few segments beyond
/// the current live edge is either a malfunctioning client or abuse.
const ABUSE_MSN_FUTURE_BOUND: u64 = 4;

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";
const MP4_CONTENT_TYPE: &str = "video/mp4";

/// `Cache-Control` for playlists (`master.m3u8`/`media.m3u8`): they must
/// always be re-fetched for liveness, never served stale from a cache.
const CACHE_CONTROL_PLAYLIST: &str = "no-cache";

/// `Cache-Control` for init/segment/part byte ranges: once produced, a given
/// URI's bytes never change (each segment/part is generated exactly once
/// under a unique filename), so these are safe to cache indefinitely.
const CACHE_CONTROL_IMMUTABLE: &str = "max-age=31536000, immutable";

/// HLS requires HLS protocol version 9 (RFC 8216bis §4.4.3.7/§4.4.3.8: the
/// `#EXT-X-PART-INF`/`#EXT-X-PART` directives this renderer always emits
/// require it).
const LL_HLS_VERSION: u8 = 9;

/// RFC 8216bis / Apple LL-HLS §4.4.3.7: `#EXT-X-SERVER-CONTROL`'s
/// `PART-HOLD-BACK` attribute MUST be at least 3x the part target duration
/// (`#EXT-X-PART-INF`'s `PART-TARGET`).
const PART_HOLD_BACK_MULTIPLIER: f64 = 3.0;

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
/// never changes) — to every response this router serves.
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

/// Render the LL-HLS media playlist for `track_id` from `store`'s current
/// segments/live parts.
///
/// RFC 8216bis §4.4.4.9: an in-progress (not yet closed) segment MUST NOT
/// be advertised with an `#EXTINF`/URI pair — that segment has no fetchable
/// resource yet — it may only appear as trailing `#EXT-X-PART` lines.
/// `transmux::hls::MediaPlaylist::open_segment` is exactly this
/// representation: its parts render as trailing `#EXT-X-PART` lines with
/// no `#EXTINF`/URI, so the in-progress segment's parts and the
/// `#EXT-X-PRELOAD-HINT` for the next, not-yet-available part are both
/// rendered by `to_m3u8()` itself — multimux only supplies the URI scheme
/// (`part-<track>-<seq>.<idx>.m4s`) and the part metadata.
pub fn media_playlist_m3u8(store: &MediaStore, track_id: u32) -> String {
    // Read these *before* taking `with_segments_and_parts`'s lock below —
    // `MediaStore::max_segment_duration` takes the same `inner` mutex
    // itself, and `std::sync::Mutex` is not reentrant, so calling it from
    // inside the `with_segments_and_parts` closure (as a previous version of
    // this function did) self-deadlocks the calling thread the first time
    // this function is ever invoked with any segment present. Caught by a
    // real network round trip against a live `MediaStore` (issue #717 slice
    // 5's acceptance test) — the existing test suite only ever called this
    // function directly (never over HTTP with two concurrently-scheduled
    // tasks), which happened to never trip the deadlock detector but hung
    // just the same once actually exercised end-to-end.
    let target_duration_secs = store.target_duration_secs();
    let max_segment_duration = store.max_segment_duration();
    store.with_segments_and_parts(|store_segments, live_parts| {
        let media_sequence = store_segments
            .front()
            .map(|s| u64::from(s.segment_seq))
            .or_else(|| live_parts.first().map(|p| u64::from(p.segment_seq)))
            .unwrap_or(1);
        let segments: Vec<MediaSegment> = store_segments
            .iter()
            .map(|s| MediaSegment {
                uri: format!("seg-{track_id}-{}.m4s", s.segment_seq),
                duration: s.duration,
                discontinuous: false,
                parts: Vec::new(),
                ..Default::default()
            })
            .collect();
        let part_target = f64::from(store.part_target_ms()) / 1000.0;
        // The in-progress segment's live parts + the next (not yet available)
        // part's preload-hint URI.
        let open_seq = live_parts.first().map(|p| p.segment_seq);
        let open_segment = open_seq.map(|seq| {
            OpenSegment::new(
                live_parts
                    .iter()
                    .filter(|p| p.segment_seq == seq)
                    .map(|p| PartSpec {
                        uri: format!("part-{track_id}-{}.{}.m4s", p.segment_seq, p.part_index),
                        duration: p.duration,
                        independent: p.independent,
                        ..Default::default()
                    })
                    .collect(),
            )
        });
        let next_part_hint = open_seq.map(|seq| {
            let next_idx = live_parts
                .iter()
                .filter(|p| p.segment_seq == seq)
                .map(|p| p.part_index)
                .max()
                .map(|idx| idx + 1)
                .unwrap_or(0);
            format!("part-{track_id}-{seq}.{next_idx}.m4s")
        });
        // RFC 8216bis §4.4.3.1 (MUST): every Media Segment's EXTINF duration,
        // rounded to the nearest integer, MUST be <= TARGETDURATION. The
        // segmenter cuts on the next keyframe *after* the configured target,
        // so a real segment routinely exceeds it — advertising the
        // configured target alone can under-declare. Use whichever is
        // larger, rounded (not the configured value's `ceil()` alone).
        let target_duration = target_duration_secs.max(max_segment_duration).round() as u32;
        let playlist = MediaPlaylist {
            version: LL_HLS_VERSION,
            target_duration,
            media_sequence,
            discontinuity_sequence: 0,
            segments,
            open_segment,
            endlist: false,
            extra_tags: vec![format!("#EXT-X-MAP:URI=\"init-{track_id}.mp4\"")],
            low_latency: Some(LowLatencyConfig {
                part_target,
                part_hold_back: part_target * PART_HOLD_BACK_MULTIPLIER,
                preload_hint_part: next_part_hint,
                ..Default::default()
            }),
            iframes_only: false,
            ..Default::default()
        };
        playlist.to_m3u8()
    })
}

/// `GET /master.m3u8` — a minimal single-variant master playlist pointing at
/// `media.m3u8`.
pub async fn master_playlist(State(_store): State<Arc<MediaStore>>) -> Response {
    let body =
        format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={PLACEHOLDER_BANDWIDTH_BPS}\nmedia.m3u8\n");
    ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response()
}

/// Blocking playlist reload query parameters (RFC 8216bis §6.2.5.2).
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

/// RAII guard bumping/dropping [`crate::prometheus::ACTIVE_BLOCKING_REQUESTS`]
/// for the lifetime of a blocking LL-HLS wait ([`wait_for_progress`]/
/// [`wait_for_part`]) — incremented on construction, decremented on drop, so
/// the gauge stays accurate even if the awaited future is itself dropped
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

/// Block until `store`'s in-progress segment/part reaches at least
/// `(msn, part)`, or [`BLOCKING_RELOAD_TIMEOUT`] elapses. Never hangs
/// indefinitely and never errors — on timeout (or a closed watch channel) it
/// simply returns, and the caller renders the playlist as it currently is.
async fn wait_for_progress(store: &MediaStore, msn: u64, part: u32) {
    let _guard = BlockingRequestGuard::new();
    let mut rx = store.subscribe();
    let wait = async {
        loop {
            let (in_progress_seg_seq, part_count) = store.latest_progress();
            let satisfied = u64::from(in_progress_seg_seq) > msn
                || (u64::from(in_progress_seg_seq) == msn && part_count > part);
            if satisfied {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    };
    let _ = tokio::time::timeout(BLOCKING_RELOAD_TIMEOUT, wait).await;
}

/// Block until segment `msn` is a fully-present (closed) Media Segment, or
/// [`BLOCKING_RELOAD_TIMEOUT`] elapses.
///
/// RFC 8216bis §6.2.5.2: a blocking-reload request carrying `_HLS_msn` with
/// **no** `_HLS_part` must be held until segment `msn` is a closed Media
/// Segment in the Playlist — unlike the `_HLS_msn`+`_HLS_part` case
/// ([`wait_for_progress`]), an in-progress segment merely having live parts
/// does not satisfy a bare `_HLS_msn` request: that segment has no `#EXTINF`/
/// URI yet, so a client that doesn't understand parts would see no new
/// segment at all.
async fn wait_for_closed_segment(store: &MediaStore, msn: u64) {
    let _guard = BlockingRequestGuard::new();
    let mut rx = store.subscribe();
    let wait = async {
        loop {
            if u64::from(store.last_closed_segment_seq()) >= msn {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    };
    let _ = tokio::time::timeout(BLOCKING_RELOAD_TIMEOUT, wait).await;
}

/// Block until part `idx` of segment `seq` is available, returning its bytes —
/// or `None` if the part will never be produced or [`BLOCKING_RELOAD_TIMEOUT`]
/// elapses. This is the origin side of LL-HLS preload-hinted part delivery
/// (RFC 8216bis §6.2.2, §6.3.1): the client fetches the `#EXT-X-PRELOAD-HINT`
/// part before it exists, and the origin holds the request open until it does.
///
/// Returns `None` *promptly* (without waiting out the timeout) once the part
/// can no longer appear as a live part: its segment has closed (it is now only
/// addressable as a whole segment via `seg-…`), or the in-progress segment has
/// advanced past `seq`. That happens at a real segment boundary when the hinted
/// "next part" is never produced (the segment closed instead) — a legitimate
/// 404 the client answers by fetching the next segment/part.
async fn wait_for_part(store: &MediaStore, seq: u32, idx: u32) -> Option<Vec<u8>> {
    let _guard = BlockingRequestGuard::new();
    let mut rx = store.subscribe();
    let wait = async {
        loop {
            if let Some(bytes) = store.part_bytes(seq, idx) {
                return Some(bytes);
            }
            let (in_progress_seg_seq, _) = store.latest_progress();
            if in_progress_seg_seq > seq || store.segment_bytes(seq).is_some() {
                return None;
            }
            if rx.changed().await.is_err() {
                return None;
            }
        }
    };
    tokio::time::timeout(BLOCKING_RELOAD_TIMEOUT, wait)
        .await
        .ok()
        .flatten()
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
/// signal to back off).
pub async fn media_playlist(
    State(store): State<Arc<MediaStore>>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    if q.hls_part.is_some() && q.hls_msn.is_none() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let Some(msn) = q.hls_msn {
        let (current_max_msn, _) = store.latest_progress();
        if msn > u64::from(current_max_msn) + ABUSE_MSN_FUTURE_BOUND {
            return StatusCode::BAD_REQUEST.into_response();
        }
        // §6.2.5.2: `_HLS_msn` alone waits for segment `msn` to CLOSE;
        // `_HLS_msn`+`_HLS_part` waits only for that part of the
        // (possibly still open) segment. These are genuinely different
        // conditions — treating a bare `_HLS_msn` as `_HLS_part=0` would
        // resolve as soon as the segment merely opens with one live part,
        // before it has an `#EXTINF`/URI at all.
        match q.hls_part {
            Some(part) => wait_for_progress(&store, msn, part).await,
            None => wait_for_closed_segment(&store, msn).await,
        }
    }
    let body = media_playlist_m3u8(&store, DEFAULT_TRACK_ID);
    ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response()
}

/// `GET /:file` — catch-all for the dynamic init/segment/part filenames
/// [`media_playlist_m3u8`] emits.
///
/// A single catch-all (rather than three routes with per-filename literals)
/// because axum 0.7's `matchit`-based router cannot mix multiple params with
/// literal text in one path segment (e.g. `seg-:track-:seq.m4s`) — only one
/// param per segment is supported, capturing the whole segment. `file` is
/// parsed here instead.
pub async fn dynamic_file(
    State(store): State<Arc<MediaStore>>,
    Path(file): Path<String>,
) -> Response {
    // A part request is the preload-hinted Partial Segment the client fetches
    // ahead of time (RFC 8216bis §6.2.2, §6.3.1). The origin promised it via
    // `#EXT-X-PRELOAD-HINT`, so when it isn't produced yet the request must be
    // *held* until the part becomes available — not answered with an immediate
    // 404 (which spams errors and defeats low latency, forcing the client back
    // to full-segment loads). See [`wait_for_part`].
    if let Some((seq, idx)) = parse_part(&file) {
        return match wait_for_part(&store, seq, idx).await {
            Some(bytes) => ([(header::CONTENT_TYPE, MP4_CONTENT_TYPE)], bytes).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    match resolve_file(&store, &file) {
        Some(bytes) => ([(header::CONTENT_TYPE, MP4_CONTENT_TYPE)], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Parse a `part-{track}-{seq}.{idx}.m4s` dynamic filename into `(seq, idx)`,
/// or `None` if it isn't a part filename (or its numeric fields don't parse).
/// `{track}` is validated but unused (see [`resolve_file`]).
fn parse_part(file: &str) -> Option<(u32, u32)> {
    let rest = file.strip_prefix("part-")?.strip_suffix(".m4s")?;
    let (track_seq, idx) = rest.rsplit_once('.')?;
    let (track, seq) = track_seq.split_once('-')?;
    track.parse::<u32>().ok()?;
    Some((seq.parse().ok()?, idx.parse().ok()?))
}

/// Parse a dynamic origin filename and fetch its bytes from `store`:
/// - `init-{track}.mp4` -> [`MediaStore::init_bytes`]
/// - `seg-{track}-{seq}.m4s` -> [`MediaStore::segment_bytes`]
///
/// Part filenames (`part-{track}-{seq}.{idx}.m4s`) are handled separately in
/// [`dynamic_file`] (they block until available — see [`parse_part`]), not
/// here. `{track}` is validated as a number but otherwise unused: `store`
/// holds a single track's data (see [`DEFAULT_TRACK_ID`]). Returns `None`
/// (-> 404) for any filename that doesn't match one of these shapes, or whose
/// numeric fields don't parse.
fn resolve_file(store: &MediaStore, file: &str) -> Option<Vec<u8>> {
    if let Some(rest) = file.strip_prefix("init-") {
        let track = rest.strip_suffix(".mp4")?;
        track.parse::<u32>().ok()?;
        return store.init_bytes();
    }
    if let Some(rest) = file.strip_prefix("seg-") {
        let rest = rest.strip_suffix(".m4s")?;
        let (track, seq) = rest.split_once('-')?;
        track.parse::<u32>().ok()?;
        let seq: u32 = seq.parse().ok()?;
        return store.segment_bytes(seq);
    }
    None
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
    /// in-progress segment 2 -- so `latest_progress()` is `(2, 2)`.
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
        // latest_progress() for the store is (2, 2): asking for msn=1 (an
        // earlier segment) is already satisfied and must not wait.
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

    // --- Playlist-rendering tests moved from `store.rs` (they exercise
    // `media_playlist_m3u8`, which now lives here) ---

    fn plain_seg(seq: u32, parts: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: parts,
        }
    }
    fn plain_part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    #[test]
    fn playlist_has_llhls_tags_and_parts() {
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(plain_part(1, 0));
        s.add_part(plain_part(1, 1));
        let m = media_playlist_m3u8(&s, 1);
        assert!(m.contains("#EXT-X-PART-INF"), "PART-INF present");
        assert!(
            m.contains("#EXT-X-SERVER-CONTROL"),
            "SERVER-CONTROL present"
        );
        assert!(m.contains("#EXT-X-PART"), "at least one PART");
        assert!(
            m.contains("part-1-1.0.m4s") || m.contains("part-1-1.1.m4s"),
            "part URI"
        );
    }

    #[test]
    fn open_segment_has_parts_but_no_extinf() {
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(plain_part(1, 0));
        s.add_part(plain_part(1, 1));
        let m = media_playlist_m3u8(&s, 1);
        // The in-progress segment's parts are advertised...
        assert!(m.contains("#EXT-X-PART"), "at least one PART line");
        assert!(m.contains("part-1-1.0.m4s"), "part 0 URI present");
        assert!(m.contains("part-1-1.1.m4s"), "part 1 URI present");
        // ...but RFC 8216bis §4.4.4.9: no premature #EXTINF/URI for the
        // not-yet-closed segment itself — "seg-1-1.m4s" must not appear
        // anywhere (it isn't fetchable; that segment hasn't been closed).
        assert!(
            !m.contains("seg-1-1.m4s"),
            "no full-segment URI for the open segment: {m}"
        );
        assert!(
            !m.contains("#EXTINF"),
            "no EXTINF for the open segment: {m}"
        );
    }

    #[test]
    fn final_part_fetchable_after_its_segment_closes() {
        // The segmenter emits a segment's final part and then closes the
        // segment in the same step. A preload-hint request for that final part
        // is typically in flight when the close happens, so it must remain
        // fetchable afterwards (from recent_parts) rather than 404 — the LL-HLS
        // preload-hint boundary bug.
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(plain_part(1, 0));
        s.add_part(plain_part(1, 1)); // .1 is this segment's final part
        s.add_segment(plain_seg(1, 2)); // close segment 1 (moves its parts to recent_parts)
        assert_eq!(
            s.part_bytes(1, 1),
            Some(vec![1; 4]),
            "final part of a just-closed segment must still be individually fetchable"
        );
        assert_eq!(s.part_bytes(1, 0), Some(vec![0; 4]), "earlier parts too");
        // A genuinely-nonexistent part of the closed segment is still absent.
        assert_eq!(s.part_bytes(1, 9), None);
        // Closing does not resurrect parts into the rendered open segment: the
        // playlist advertises the whole segment, not its parts.
        let m = media_playlist_m3u8(&s, 1);
        assert!(
            m.contains("seg-1-1.m4s"),
            "closed segment rendered whole: {m}"
        );
        assert!(
            !m.contains("part-1-1."),
            "closed parts not rendered as open: {m}"
        );
    }

    #[test]
    fn live_parts_capped_when_segment_never_closes() {
        // target_duration_secs=4.0, part_target_ms=500 -> cap =
        // ceil(4.0 / 0.5) + 4 margin = 12 (see
        // `crate::store::compute_max_live_parts`).
        let s = MediaStore::new(4.0, 500, 4);
        let cap = crate::store::compute_max_live_parts(4.0, 500);
        assert_eq!(cap, 12, "sanity-check the expected cap for these params");
        s.set_init(vec![0; 4]);

        // Push far more parts than the cap into a single never-closed
        // segment (no add_segment call) — RAM must stay bounded.
        for i in 0..(cap as u32 * 5) {
            s.add_part(plain_part(1, i));
        }
        assert_eq!(
            s.live_part_count(),
            cap,
            "live_parts must stay capped even though the segment never closed"
        );

        // The playlist must still render correctly from the capped parts:
        // only the most recent (highest-index) parts survive.
        let m = media_playlist_m3u8(&s, 1);
        assert!(m.contains("#EXT-X-PART"), "still has PART lines: {m}");
        let last_idx = cap as u32 * 5 - 1;
        assert!(
            m.contains(&format!("part-1-1.{last_idx}.m4s")),
            "most recent part must survive the cap: {m}"
        );
        let first_idx = cap as u32 * 5 - cap as u32;
        assert!(
            !m.contains(&format!("part-1-1.{}.m4s", first_idx - 1)),
            "an older part beyond the cap must have been dropped: {m}"
        );
    }

    // --- P2 LL-HLS spec-conformance fixes (audit-llhls #1/#2/#3/#4) ---

    #[test]
    fn target_duration_is_max_of_configured_and_actual_segment_duration() {
        // Configured target is 4.0s, but the segmenter cuts on the next
        // keyframe after the target so a real segment can run long (7.5s
        // here) — RFC 8216bis §4.4.3.1 (MUST) requires TARGETDURATION to be
        // >= every EXTINF, rounded. The old hardcoded
        // `ceil(target_duration_secs)` would render `4`, violating the MUST.
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        let mut long_seg = plain_seg(1, 2);
        long_seg.duration = 7.5;
        s.add_segment(long_seg);
        let m = media_playlist_m3u8(&s, 1);
        assert!(
            m.contains("#EXT-X-TARGETDURATION:8"),
            "TARGETDURATION must be round(7.5)=8, not the configured target (4): {m}"
        );
    }

    #[test]
    fn target_duration_falls_back_to_configured_when_segments_are_short() {
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_segment(plain_seg(1, 2)); // plain_seg's fixed duration is 4.0
        let m = media_playlist_m3u8(&s, 1);
        assert!(
            m.contains("#EXT-X-TARGETDURATION:4"),
            "unchanged behaviour when no segment exceeds the configured target: {m}"
        );
    }

    #[tokio::test]
    async fn media_playlist_msn_only_waits_for_closed_segment_not_just_open_parts() {
        // make_store()'s segment 2 is OPEN with 2 live parts
        // (latest_progress() == (2, 2)) but not yet CLOSED. RFC 8216bis
        // §6.2.5.2: a bare `_HLS_msn=2` (no `_HLS_part`) must wait for
        // segment 2 to actually close, not resolve merely because it has
        // live parts. Reverting to `q.hls_part.unwrap_or(0)` treats this
        // identically to `_HLS_part=0`, which IS satisfied by
        // `part_count(2) > 0` — it would return immediately and this test
        // would fail (elapsed far under the close delay).
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
        // latest_progress() for make_store() is (2, 2). A `_HLS_msn` 1000
        // ahead of the live edge is not a legitimate blocking-reload request
        // (RFC 8216bis §6.2.5.2 abuse prevention) — it must 400 immediately,
        // not consume the full BLOCKING_RELOAD_TIMEOUT before giving up.
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
        let store = make_store(); // latest_progress() == (2, 2)
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
