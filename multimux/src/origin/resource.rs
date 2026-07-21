//! The shared origin-level resource route: `init-*.mp4` / `seg-*.m4s` /
//! `part-*.m4s` byte serving, mounted **once per stream** by
//! [`crate::origin::router`] rather than per-`Output` — LL-HLS and DASH are
//! both fMP4/CMAF and reference the exact same
//! [`crate::store::MediaStore`]-produced bytes, so serving them per-output
//! would duplicate the route (and previously caused an axum panic: two
//! `Output`s both mounting a `/:file` catch-all under the same `/{stream}`
//! nest — issue #663 P4's "multi-output nest collision" fix).
//!
//! Each [`crate::output::Output`] contributes only its manifest route(s)
//! (`master.m3u8`/`media.m3u8` for LL-HLS, `manifest.mpd` for DASH); this
//! module is the one thing every output shares.
//!
//! # Chunked-transfer whole-segment serving (issue #721)
//!
//! [`crate::output::ll_dash`]'s true low-latency DASH design addresses whole
//! segments (`seg-{track}-{seq}.m4s`, the same filenames
//! [`crate::output::dash`]'s regular MPD uses) but needs a segment's bytes to
//! start flowing *before* it closes. [`dynamic_file`] implements this: a
//! `seg-*.m4s` request that doesn't (yet) resolve to a closed segment falls
//! through to [`stream_in_progress_segment`], which re-fetches that
//! segment's `part-{track}-{seq}.{idx}.m4s` entries in order — the exact
//! bytes [`ResourceOutcome`]'s existing blocking-wait machinery already
//! produces for LL-HLS's own preload-hint requests — and streams them as one
//! HTTP chunked-transfer-encoded response body, ending once a part index
//! resolves [`ResourceOutcome::NotFound`] (which only happens once the
//! segment has actually closed without that part, i.e. exactly the segment's
//! end). A genuinely future segment (nothing produced yet) blocks the same
//! bounded [`BLOCKING_RELOAD_TIMEOUT`] on its first part before giving up
//! (404), mirroring the plain closed-segment/part lookups below. LL-HLS
//! itself never triggers this path: its playlist never advertises an
//! in-progress segment's whole-segment URI (RFC 8216bis §4.4.4.9), so a
//! well-behaved client never requests one.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::stream;
use ll_hls_runtime::server::ResourceOutcome;

use crate::store::MediaStore;

/// Upper bound on how long a blocking dynamic-file request (a preload-hinted
/// part not yet produced) waits before falling back to `404`. Mirrors
/// `output::llhls`'s own playlist-blocking timeout (RFC 8216bis §6.2.5.2
/// requires the origin to eventually respond either way) — kept as a
/// separate constant here (rather than shared with the playlist one) since
/// the two waits are conceptually independent, even though they currently
/// have the same value.
pub(crate) const BLOCKING_RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) const MP4_CONTENT_TYPE: &str = "video/mp4";

/// Abuse bound for [`stream_in_progress_segment`]'s whole-segment number,
/// mirroring `ll_hls_runtime::server::engine`'s own `ABUSE_MSN_FUTURE_BOUND`
/// (RFC 8216bis §6.2.5.2's abuse-prevention SHOULD, applied here to the
/// DASH-facing whole-segment lookup): a legitimate LL-DASH client only ever
/// requests the segment right after the one it already has, so a segment
/// number more than a few ahead of the current live edge is either a broken
/// client or abuse — reject it immediately (404) rather than tying up a
/// blocking-wait task and a connection slot for the full
/// [`BLOCKING_RELOAD_TIMEOUT`].
const SEGMENT_ABUSE_FUTURE_BOUND: u32 = 4;

/// RAII guard bumping/dropping [`crate::prometheus::ACTIVE_BLOCKING_REQUESTS`]
/// for the lifetime of a blocking wait ([`resource_blocking`], and
/// `output::llhls`'s own playlist-blocking wait) — incremented on
/// construction, decremented on drop, so the gauge stays accurate even if the
/// awaited future is itself dropped (e.g. the client disconnects mid-wait),
/// not just on a normal return.
pub(crate) struct BlockingRequestGuard;

impl BlockingRequestGuard {
    pub(crate) fn new() -> Self {
        metrics::gauge!(crate::prometheus::ACTIVE_BLOCKING_REQUESTS).increment(1.0);
        BlockingRequestGuard
    }
}

impl Drop for BlockingRequestGuard {
    fn drop(&mut self) {
        metrics::gauge!(crate::prometheus::ACTIVE_BLOCKING_REQUESTS).decrement(1.0);
    }
}

/// Build the shared resource router for one stream: `GET /:file`, serving
/// `init-{track}.mp4` / `seg-{track}-{seq}.m4s` / `part-{track}-{seq}.{idx}.m4s`
/// from `store`. Mounted once per stream by [`crate::origin::router`],
/// merged alongside every configured `Output`'s manifest routes before the
/// whole per-stream router is `.nest`ed — see this module's docs.
///
/// `Cache-Control`/CORS headers are applied by the origin's shared
/// `add_response_headers` middleware (wrapping the *merged* per-stream
/// router, not this one alone), so every output's responses get the same
/// policy uniformly.
pub(crate) fn router(store: Arc<MediaStore>) -> Router {
    Router::new()
        .route("/:file", get(dynamic_file).options(cors_preflight))
        .with_state(store)
}

/// `OPTIONS` preflight handler shared by every route this origin serves
/// (manifest and resource alike): browsers (hls.js/dash.js) send a CORS
/// preflight before the real `GET` for cross-origin requests with custom
/// headers. Returns `204 No Content` with no body; the origin's
/// `add_response_headers` middleware adds the actual
/// `Access-Control-Allow-*` headers to this response the same as every other
/// response.
pub(crate) async fn cors_preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Resolve `store`'s dynamic resource `name`, waiting (bounded by
/// [`BLOCKING_RELOAD_TIMEOUT`]) on a [`ResourceOutcome::WouldBlock`] (a
/// preload-hinted part not yet produced) rather than 404ing immediately.
/// Same caller-driven wait-loop shape as `output::llhls`'s playlist-blocking
/// wait. On timeout, falls back to [`ResourceOutcome::NotFound`] (a `404`) —
/// there is no "current" resource to serve instead.
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

/// `GET /:file` — catch-all for the dynamic init/segment/part filenames
/// `ll_hls_runtime::server::media_playlist_m3u8` emits (and the same
/// filenames a DASH `SegmentTemplate` references — see
/// `crate::output::dash`).
///
/// A single catch-all (rather than three routes with per-filename literals)
/// because axum 0.7's `matchit`-based router cannot mix multiple params with
/// literal text in one path segment (e.g. `seg-:track-:seq.m4s`) — only one
/// param per segment is supported, capturing the whole segment. Parsing
/// `file` into a segment/part/init lookup — including the "block until a
/// preload-hinted part is produced" behaviour (RFC 8216bis §6.2.2, §6.3.1) —
/// is [`ll_hls_runtime::server::MediaStore::resolve_resource`]'s job; this
/// handler only drives the wait ([`resource_blocking`]) and maps the outcome
/// to an HTTP response.
async fn dynamic_file(State(store): State<Arc<MediaStore>>, Path(file): Path<String>) -> Response {
    match resource_blocking(&store, &file).await {
        ResourceOutcome::Ready { bytes, .. } => {
            ([(header::CONTENT_TYPE, MP4_CONTENT_TYPE)], bytes).into_response()
        }
        ResourceOutcome::NotFound => {
            // Not a closed segment (yet) -- if this is a whole-segment
            // filename, try the chunked-transfer in-progress/future-segment
            // path (issue #721) before giving up. Every other filename shape
            // (init/part) has nothing more to try.
            if let Some((track, seq)) = parse_segment_filename(&file) {
                if let Some(resp) = stream_in_progress_segment(store, track, seq).await {
                    return resp;
                }
            }
            StatusCode::NOT_FOUND.into_response()
        }
        ResourceOutcome::WouldBlock => StatusCode::NOT_FOUND.into_response(),
        // `ResourceOutcome` is `#[non_exhaustive]` — treat any future
        // variant this handler doesn't yet know how to serve as a 404
        // rather than panicking or fabricating a body.
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Parse a whole-segment dynamic filename (`seg-{track}-{seq}.m4s`) into
/// `(track, seq)`. Mirrors `ll_hls_runtime::server`'s own (private)
/// `seg-`/`part-` filename parsing, but keeps `track` as a borrowed `&str`
/// (rather than discarding it once validated) so [`stream_in_progress_segment`]
/// can reuse it verbatim to build this segment's `part-{track}-{seq}.{idx}.m4s`
/// filenames -- `{track}` is otherwise unused (the store holds a single
/// track's data regardless of the number a client's `$RepresentationID$`
/// substitution produces, exactly like `resolve_resource` itself).
fn parse_segment_filename(file: &str) -> Option<(&str, u32)> {
    let rest = file.strip_prefix("seg-")?.strip_suffix(".m4s")?;
    let (track, seq) = rest.split_once('-')?;
    track.parse::<u32>().ok()?;
    Some((track, seq.parse().ok()?))
}

/// Serve a not-yet-closed whole-segment filename (`seg-{track}-{seq}.m4s`)
/// over **HTTP chunked transfer-encoding**, streaming `seq`'s
/// `part-{track}-{seq}.{idx}.m4s` bytes in order as they are produced
/// (issue #721 -- see this module's docs and `crate::output::ll_dash`).
///
/// `None` (caller 404s) if the segment's very first part never arrives
/// within [`BLOCKING_RELOAD_TIMEOUT`] — a genuinely future segment whose
/// ingest hasn't reached it yet (or a stalled/dead route), mirroring the
/// plain closed-segment lookup's own bound. Once the first part is ready,
/// `Some` commits to a `200 OK` streamed response that keeps pulling
/// subsequent parts (each wait bounded the same way) until a part index
/// resolves [`ResourceOutcome::NotFound`] — which only happens once the
/// segment has actually closed (or been evicted) without that part, i.e.
/// exactly the segment's end — at which point the stream ends normally
/// (the response completes; axum/hyper terminate the chunked-transfer
/// encoding on drop).
async fn stream_in_progress_segment(
    store: Arc<MediaStore>,
    track: &str,
    seq: u32,
) -> Option<Response> {
    // Abuse/malformed-request bound (see `SEGMENT_ABUSE_FUTURE_BOUND`) --
    // checked before ever registering a blocking wait.
    let (in_progress_seg_seq, _) = store.latest_progress();
    if seq > in_progress_seg_seq.saturating_add(SEGMENT_ABUSE_FUTURE_BOUND) {
        return None;
    }

    let track = track.to_string();
    let first = resource_blocking(&store, &format!("part-{track}-{seq}.0.m4s")).await;
    let first_bytes = match first {
        ResourceOutcome::Ready { bytes, .. } => bytes,
        _ => return None,
    };

    let cursor = PartCursor {
        store,
        track,
        seq,
        next_index: 1,
        pending_first: Some(first_bytes),
    };
    let body_stream = stream::unfold(cursor, |mut cursor| async move {
        if let Some(bytes) = cursor.pending_first.take() {
            return Some((Ok::<_, std::io::Error>(bytes), cursor));
        }
        let name = format!(
            "part-{}-{}.{}.m4s",
            cursor.track, cursor.seq, cursor.next_index
        );
        match resource_blocking(&cursor.store, &name).await {
            ResourceOutcome::Ready { bytes, .. } => {
                cursor.next_index += 1;
                Some((Ok(bytes), cursor))
            }
            // WouldBlock cannot escape `resource_blocking` (it only returns
            // a terminal outcome), and any other/future variant has no
            // bytes to add -- end the stream rather than loop or panic.
            _ => None,
        }
    });

    let mut response = Response::new(Body::from_stream(body_stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(MP4_CONTENT_TYPE),
    );
    Some(response)
}

/// Streaming state for [`stream_in_progress_segment`]'s `futures_util::stream::unfold`.
struct PartCursor {
    store: Arc<MediaStore>,
    track: String,
    seq: u32,
    /// The 0-based index of the next part to fetch once `pending_first` is
    /// drained.
    next_index: u32,
    /// Part 0's bytes, already fetched by the caller to decide whether to
    /// commit to a streamed response at all -- yielded first so it isn't
    /// fetched twice.
    pending_first: Option<Vec<u8>>,
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
    /// in-progress segment 2 -- so `latest_progress()` treats the store as
    /// `(2, 2)`.
    fn make_store() -> Arc<MediaStore> {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 8]);
        store.add_segment(seg(1));
        store.add_part(part(2, 0));
        store.add_part(part(2, 1));
        store
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
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

    // --- issue #721: chunked-transfer whole-segment serving ---

    #[tokio::test]
    async fn dynamic_file_in_progress_segment_streams_concatenated_parts_and_completes_on_close() {
        // Only part 0 exists when the request is made -- part 1 doesn't
        // land, and the segment doesn't close, until *after* the handler
        // must already have committed to a streamed response (it can only
        // ever see part 0 at call time). This proves genuine incremental
        // streaming, not "wait for everything, then answer once": if the
        // handler eagerly required the whole segment up front, this request
        // would have nothing to serve yet and would 404/block differently.
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 8]);
        store.add_part(part(2, 0));

        let store_for_task = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_task.add_part(part(2, 1));
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_task.add_segment(seg(2));
        });

        let resp = dynamic_file(State(store), Path("seg-1-2.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "an in-progress whole-segment request must stream, not 404"
        );
        assert_eq!(
            body_bytes(resp).await,
            [vec![0x10; 4], vec![0x11; 4]].concat(),
            "streamed body must be part 0 + part 1 concatenated in order, \
             including the part that only landed after the response started"
        );
    }

    #[tokio::test]
    async fn dynamic_file_future_segment_within_bound_blocks_then_streams_once_started() {
        // Segment 3 hasn't started at all (latest_progress() == (2, 2), so 3
        // is the very next segment -- within SEGMENT_ABUSE_FUTURE_BOUND).
        // The request must block (not immediately 404) until the segment's
        // first part lands, then stream from it.
        let store = make_store();
        let store_for_start = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            store_for_start.add_part(PartInfo {
                bytes: vec![0x77; 4],
                duration: 0.5,
                independent: true,
                segment_seq: 3,
                part_index: 0,
            });
        });

        let started = std::time::Instant::now();
        let resp = dynamic_file(State(store.clone()), Path("seg-1-3.m4s".to_string())).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a near-future segment must be waited for, not rejected"
        );
        assert!(
            started.elapsed() < BLOCKING_RELOAD_TIMEOUT,
            "must resolve once the part lands, not idle out the full timeout"
        );
        // Only one part exists so far; the response completes once segment 3
        // eventually closes. Close it now so the body finishes.
        store.add_segment(seg(3));
        assert_eq!(body_bytes(resp).await, vec![0x77; 4]);
    }

    #[tokio::test]
    async fn dynamic_file_far_future_segment_beyond_abuse_bound_404_promptly() {
        // latest_progress() == (2, 2); segment 99 is far beyond
        // SEGMENT_ABUSE_FUTURE_BOUND ahead of the live edge -- must reject
        // immediately (no blocking wait at all), unlike a legitimate
        // near-future segment.
        let store = make_store();
        let started = std::time::Instant::now();
        let resp = dynamic_file(State(store), Path("seg-1-99.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "an abusive far-future segment number must 404 promptly, not block: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn dynamic_file_closed_segment_still_served_whole_not_streamed() {
        // Regression: a segment that is ALREADY closed must still take the
        // plain, non-streaming fast path (`resolve_resource`'s whole bytes,
        // never falling through to `stream_in_progress_segment`) -- proven
        // by the exact byte match ([0x21; 8] is `seg`'s literal whole-segment
        // fixture bytes, not a concatenation of any parts).
        let store = make_store();
        let resp = dynamic_file(State(store), Path("seg-1-1.m4s".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, vec![0x21; 8]);
    }
}
