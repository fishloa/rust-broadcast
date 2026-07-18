#![cfg(feature = "testsupport")]
//! Deterministic end-to-end integration gate for the LL-HLS origin (#663):
//! `MockSource` -> [`run_pipeline`] -> [`MediaStore`] -> the axum
//! [`router`], driven with `tower::ServiceExt::oneshot` (no real TCP socket,
//! no timing-dependent assertions) so the whole demux-free pipeline-to-HTTP
//! path is exercised without flakiness.
//!
//! Test 1 proves the served bytes are actually valid fMP4/CMAF (via
//! `transmux::validate`), not just "some bytes came back". Test 2 proves the
//! blocking-reload path (RFC 8216bis §6.2.5.2) really wakes on new data,
//! rather than serving a stale playlist or hanging to its timeout.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::pipeline::{MockSource, run_pipeline};
use multimux::store::MediaStore;
use transmux::avc_config_from_sprop;
use transmux::ll_hls::PartInfo;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::validate::{Severity, validate_init_segment, validate_media_segment};

/// A real-ish sprop-parameter-sets pair (SPS+PPS) — same one used by
/// `multimux::pipeline`'s and `multimux::source::rtsp`'s own tests — decoded
/// into an `avcC` config so the init segment carries a genuine AVC
/// configuration record rather than a fabricated one.
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

/// 90 kHz video timescale (the CMAF movie timescale `run_pipeline` builds
/// with) — 1/30 s per access unit at 30 fps.
const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;

fn video_track_spec() -> TrackSpec {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        1,
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 0,
            height: 0,
        },
    )
}

/// Only `Severity::Error` issues fail the gate — warnings (e.g. a missing
/// `mvex`/`trex` on a segment that isn't the init segment) are informational.
fn errors_only(
    issues: &[transmux::validate::ConformanceIssue],
) -> Vec<&transmux::validate::ConformanceIssue> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect()
}

/// Extract the first `seg-{track}-{seq}.m4s` URI out of a rendered media
/// playlist body.
fn first_segment_uri(playlist: &str) -> Option<&str> {
    let start = playlist.find("seg-")?;
    let rest = &playlist[start..];
    let end = rest.find(".m4s")? + ".m4s".len();
    Some(&rest[..end])
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("well-formed GET request")
}

#[tokio::test]
async fn end_to_end_pipeline_serves_valid_llhls() {
    let store = Arc::new(MediaStore::new(1.0, 500, 8));
    let specs = vec![video_track_spec()];

    // 120 frames @ 30 fps = 4 s of video, with sync samples every 30 frames
    // (1 s, == the 1.0 s target duration) so at least 3 segments close via
    // the normal boundary path (not just the EOS flush tail), each carrying
    // several 500 ms parts (15 frames/part at 30 fps/90kHz).
    let mut batches = Vec::new();
    for i in 0..120u32 {
        let is_sync = i % 30 == 0;
        let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 64];
        let sample = Sample::new(data, FRAME_DUR, is_sync, 0);
        batches.push(vec![(1u32, sample)]);
    }

    let source = MockSource::new(specs, batches);
    run_pipeline(store.clone(), 1.0, 500, source, "cam")
        .await
        .expect("pipeline runs to completion");

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // 1. Media playlist: LL-HLS tags present.
    let resp = app
        .clone()
        .oneshot(get("/cam/media.m3u8"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(playlist.contains("#EXT-X-PART"), "playlist: {playlist}");
    assert!(playlist.contains("#EXT-X-PART-INF"), "playlist: {playlist}");
    assert!(
        playlist.contains("#EXT-X-SERVER-CONTROL"),
        "playlist: {playlist}"
    );

    // 2. Init segment: 200 + structurally conformant fMP4 init segment.
    let resp = app
        .clone()
        .oneshot(get("/cam/init-1.mp4"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK);
    let init_bytes = body_bytes(resp).await;
    assert!(!init_bytes.is_empty(), "init segment body non-empty");
    let init_issues = validate_init_segment(&init_bytes);
    let init_errors = errors_only(&init_issues);
    assert!(
        init_errors.is_empty(),
        "init segment must have no conformance errors, got: {init_errors:?}"
    );

    // 3. A real media segment URI from the playlist: 200 + conformant media
    // segment.
    let seg_uri = first_segment_uri(&playlist)
        .unwrap_or_else(|| panic!("no seg-*.m4s URI in playlist: {playlist}"));
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{seg_uri}")))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK, "segment URI: {seg_uri}");
    let seg_bytes = body_bytes(resp).await;
    assert!(!seg_bytes.is_empty(), "segment body non-empty");
    let seg_issues = validate_media_segment(&seg_bytes);
    let seg_errors = errors_only(&seg_issues);
    assert!(
        seg_errors.is_empty(),
        "media segment must have no conformance errors, got: {seg_errors:?}"
    );

    // 4. Unknown stream -> 404.
    let resp = app
        .oneshot(get("/ghost/media.m3u8"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

fn part(seq: u32, idx: u32) -> PartInfo {
    PartInfo {
        bytes: vec![0x10 + idx as u8; 4],
        duration: 0.5,
        independent: idx == 0,
        segment_seq: seq,
        part_index: idx,
    }
}

/// Bites the blocking-reload wakeup path (`output::llhls`'s internal `wait_for_progress`):
/// without the `watch`-based wakeup + re-render, a `_HLS_msn`/`_HLS_part`
/// request for a not-yet-available part would either serve the stale
/// playlist (missing the new part) or hang to `BLOCKING_RELOAD_TIMEOUT`
/// (5 s) rather than resolving as soon as the part lands.
#[tokio::test]
async fn blocking_reload_resolves_when_part_arrives() {
    let store = Arc::new(MediaStore::new(4.0, 500, 8));
    store.set_init(vec![0xAA; 8]);
    store.add_part(part(1, 0));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // latest_progress() is currently (1, 1): in-progress segment 1 has one
    // part (index 0). Asking for msn=1/part=1 is NOT yet satisfied
    // (part_count(1) > part(1) is false), so this request must block.
    let app_for_task = app.clone();
    let handle = tokio::spawn(async move {
        app_for_task
            .oneshot(get("/cam/media.m3u8?_HLS_msn=1&_HLS_part=1"))
            .await
            .expect("router call")
    });

    // Let the spawned task actually reach the `watch` await point before we
    // publish the part it's waiting for; the correctness of the assertion
    // below rests on the spawn+add_part ordering and the handler's own
    // `changed().await`, not on this yield being "long enough".
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }

    store.add_part(part(1, 1));

    // Bound the wait in *real* time (no `tokio::time::pause()`): a working
    // watch wakeup resolves within milliseconds of `add_part`, whereas a
    // broken wakeup only resolves at the handler's internal 5 s
    // `BLOCKING_RELOAD_TIMEOUT` fallback. 500 ms comfortably separates the
    // two without being flaky on a loaded CI box.
    let resp = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
        .await
        .expect("blocking reload must resolve promptly on the watch wakeup, not the 5s timeout fallback")
        .expect("blocking request task did not panic");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "blocking reload must resolve (not timeout/404) once the awaited part lands"
    );
    let playlist = body_string(resp).await;
    // Assert the *real* `#EXT-X-PART` line for the arrived part, not merely
    // the bare URI — that URI also appears in the always-emitted
    // `#EXT-X-PRELOAD-HINT:TYPE=PART,URI="part-1-1.1.m4s"` line for the
    // next-expected part, which is present even before `add_part(1, 1)` runs
    // (see `multimux::output::llhls::media_playlist_m3u8`). Only a genuine PART line
    // proves the new part was actually rendered.
    let real_part_line = "#EXT-X-PART:DURATION=0.5,URI=\"part-1-1.1.m4s\"";
    assert!(
        playlist.contains(real_part_line),
        "resolved playlist must include the real #EXT-X-PART line for the \
         newly-arrived part (not just the preload-hint URI): {playlist}"
    );
}
