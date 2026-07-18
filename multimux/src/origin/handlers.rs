//! LL-HLS request handlers: master/media playlists and the init/segment/part
//! byte ranges the media playlist references.
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`, rendered by
//! [`crate::store::StreamStore::media_playlist_m3u8`]); the blocking reload
//! query parameters (`_HLS_msn`/`_HLS_part`) are the Blocking Playlist Reload
//! mechanism of RFC 8216bis §6.2.5.2 — the client asks the origin to hold the
//! response open until the requested Media Sequence Number/part is available,
//! bounded so the origin never hangs indefinitely.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::origin::AppState;
use crate::store::StreamStore;

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

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";
const MP4_CONTENT_TYPE: &str = "video/mp4";

/// `GET /:stream/master.m3u8` — a minimal single-variant master playlist
/// pointing at `media.m3u8`.
pub async fn master_playlist(
    State(state): State<Arc<AppState>>,
    Path(stream): Path<String>,
) -> Response {
    if !state.streams.contains_key(&stream) {
        return StatusCode::NOT_FOUND.into_response();
    }
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

/// Block until `store`'s in-progress segment/part reaches at least
/// `(msn, part)`, or [`BLOCKING_RELOAD_TIMEOUT`] elapses. Never hangs
/// indefinitely and never errors — on timeout (or a closed watch channel) it
/// simply returns, and the caller renders the playlist as it currently is.
async fn wait_for_progress(store: &StreamStore, msn: u64, part: u32) {
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
async fn wait_for_part(store: &StreamStore, seq: u32, idx: u32) -> Option<Vec<u8>> {
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

/// `GET /:stream/media.m3u8` — the LL-HLS media playlist for
/// [`DEFAULT_TRACK_ID`], blocking on `_HLS_msn`/`_HLS_part` when present.
pub async fn media_playlist(
    State(state): State<Arc<AppState>>,
    Path(stream): Path<String>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    let Some(store) = state.streams.get(&stream) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(msn) = q.hls_msn {
        let part = q.hls_part.unwrap_or(0);
        wait_for_progress(store, msn, part).await;
    }
    let body = store.media_playlist_m3u8(DEFAULT_TRACK_ID);
    ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response()
}

/// `GET /:stream/:file` — catch-all for the dynamic init/segment/part
/// filenames [`crate::store::StreamStore::media_playlist_m3u8`] emits.
///
/// A single catch-all (rather than three routes with per-filename literals)
/// because axum 0.7's `matchit`-based router cannot mix multiple params with
/// literal text in one path segment (e.g. `seg-:track-:seq.m4s`) — only one
/// param per segment is supported, capturing the whole segment. `file` is
/// parsed here instead.
pub async fn dynamic_file(
    State(state): State<Arc<AppState>>,
    Path((stream, file)): Path<(String, String)>,
) -> Response {
    let Some(store) = state.streams.get(&stream) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // A part request is the preload-hinted Partial Segment the client fetches
    // ahead of time (RFC 8216bis §6.2.2, §6.3.1). The origin promised it via
    // `#EXT-X-PRELOAD-HINT`, so when it isn't produced yet the request must be
    // *held* until the part becomes available — not answered with an immediate
    // 404 (which spams errors and defeats low latency, forcing the client back
    // to full-segment loads). See [`wait_for_part`].
    if let Some((seq, idx)) = parse_part(&file) {
        return match wait_for_part(store, seq, idx).await {
            Some(bytes) => ([(header::CONTENT_TYPE, MP4_CONTENT_TYPE)], bytes).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    match resolve_file(store, &file) {
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
/// - `init-{track}.mp4` -> [`StreamStore::init_bytes`]
/// - `seg-{track}-{seq}.m4s` -> [`StreamStore::segment_bytes`]
///
/// Part filenames (`part-{track}-{seq}.{idx}.m4s`) are handled separately in
/// [`dynamic_file`] (they block until available — see [`parse_part`]), not
/// here. `{track}` is validated as a number but otherwise unused: `store`
/// holds a single track's data (see [`DEFAULT_TRACK_ID`]). Returns `None`
/// (-> 404) for any filename that doesn't match one of these shapes, or whose
/// numeric fields don't parse.
fn resolve_file(store: &StreamStore, file: &str) -> Option<Vec<u8>> {
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
    use crate::origin::AppState;
    use std::collections::HashMap;
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

    /// One populated stream ("cam1"): a closed segment 1, plus two live parts
    /// of in-progress segment 2 -- so `latest_progress()` is `(2, 2)`.
    fn make_state() -> Arc<AppState> {
        let store = Arc::new(StreamStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 8]);
        store.add_segment(seg(1));
        store.add_part(part(2, 0));
        store.add_part(part(2, 1));
        let mut streams = HashMap::new();
        streams.insert("cam1".to_string(), store);
        Arc::new(AppState { streams })
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
    async fn master_playlist_known_stream_ok() {
        let state = make_state();
        let resp = master_playlist(State(state), Path("cam1".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXTM3U"));
        assert!(body.contains("#EXT-X-STREAM-INF"));
        assert!(body.contains("media.m3u8"));
    }

    #[tokio::test]
    async fn master_playlist_unknown_stream_404() {
        let state = make_state();
        let resp = master_playlist(State(state), Path("nope".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_playlist_no_query_renders_now() {
        let state = make_state();
        let resp = media_playlist(
            State(state),
            Path("cam1".to_string()),
            Query(BlockingReloadQuery::default()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXT-X-PART"), "body: {body}");
    }

    #[tokio::test]
    async fn media_playlist_unknown_stream_404() {
        let state = make_state();
        let resp = media_playlist(
            State(state),
            Path("nope".to_string()),
            Query(BlockingReloadQuery::default()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_playlist_already_satisfied_blocking_request_resolves_immediately() {
        // latest_progress() for "cam1" is (2, 2): asking for msn=1 (an
        // earlier segment) is already satisfied and must not wait.
        let state = make_state();
        let resp = media_playlist(
            State(state),
            Path("cam1".to_string()),
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
        let state = make_state();
        let resp = media_playlist(
            State(state),
            Path("cam1".to_string()),
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
        let state = make_state();
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "init-1.mp4".to_string())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0xAA; 8]);
    }

    #[tokio::test]
    async fn dynamic_file_segment_present_and_absent() {
        let state = make_state();
        let ok = dynamic_file(
            State(state.clone()),
            Path(("cam1".to_string(), "seg-1-1.m4s".to_string())),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(body_bytes(ok).await, vec![0x21; 8]);

        let missing = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "seg-1-99.m4s".to_string())),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_part_present() {
        let state = make_state();
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "part-1-2.0.m4s".to_string())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0x10; 4]);
    }

    #[tokio::test]
    async fn dynamic_file_part_blocks_until_available_then_serves() {
        // part-1-2.2 is the preload-hinted next part of in-progress segment 2
        // (which currently has parts .0 and .1). The request must BLOCK until
        // the part is produced, not 404 immediately. Produce it after a short
        // delay from another task, then assert the handler returned its bytes.
        let state = make_state();
        let store = state.streams.get("cam1").unwrap().clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store.add_part(part(2, 2));
        });
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "part-1-2.2.m4s".to_string())),
        )
        .await;
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
        let state = make_state();
        let store = state.streams.get("cam1").unwrap().clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store.add_segment(seg(2)); // closes segment 2
        });
        let started = std::time::Instant::now();
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "part-1-2.9.m4s".to_string())),
        )
        .await;
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
        let state = make_state();
        let store = state.streams.get("cam1").unwrap().clone();
        store.add_segment(seg(2)); // close segment 2, moving its parts to recent_parts
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "part-1-2.1.m4s".to_string())),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a just-closed segment's part must still be served, not 404"
        );
        assert_eq!(body_bytes(resp).await, vec![0x11; 4]); // part(2,1): 0x10 + idx(1)
    }

    #[tokio::test]
    async fn dynamic_file_part_of_old_segment_404() {
        // Segment 1 closed in make_state() with no parts recorded and is old
        // enough to be past the recent-parts retention window, so a request for
        // one of its parts 404s without blocking (it will never be produced and
        // isn't individually addressable anymore).
        let state = make_state();
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "part-1-1.0.m4s".to_string())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_unknown_stream_404() {
        let state = make_state();
        let resp = dynamic_file(
            State(state),
            Path(("nope".to_string(), "init-1.mp4".to_string())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_unmatched_filename_404() {
        let state = make_state();
        let resp = dynamic_file(
            State(state),
            Path(("cam1".to_string(), "not-a-thing.txt".to_string())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
