//! Deterministic end-to-end integration gate for the LL-HLS origin (#663): a
//! real [`LlHlsSegmenter`] feeding [`RouteHandle`] directly -> the axum
//! [`router`], driven with `tower::ServiceExt::oneshot` (no real TCP socket,
//! no timing-dependent assertions) so the whole segmenter-to-HTTP path is
//! exercised without flakiness.
//!
//! Test 1 proves the served bytes are actually valid fMP4/CMAF (via
//! `transmux::validate`), not just "some bytes came back". Test 2 proves the
//! blocking-reload path (RFC 8216bis §6.2.5.2) really wakes on new data,
//! rather than serving a stale playlist or hanging to its timeout.
//!
//! Issue #805 task 5 deleted `crate::pipeline::run_pipeline`/`MockSource`
//! (the old `SourceConnector`-fed path's test scaffolding); test 1 below
//! drives the same real `LlHlsSegmenter` that module used to wrap, directly —
//! mirroring `tests/lldash_dashjs.rs`'s own `run_live_producer` — since
//! `RouteHandle`'s owned `Trunk` is a test-only placeholder now (see
//! `RouteHandle`'s own doc), and this test exists precisely to drive it
//! directly and publish it.

use crate::support::origin::{body_bytes, body_string, get};
use crate::support::segmenter_feed::{
    FRAME_DUR, PART_TARGET_MS, TARGET_DURATION_SECS, feed_via_segmenter, video_track_spec,
};
use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;
use transmux::ll_hls::PartInfo;
use transmux::pipeline::Sample;
use transmux::validate::{Severity, validate_init_segment, validate_media_segment};

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

#[tokio::test]
async fn end_to_end_pipeline_serves_valid_llhls() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    // 120 frames @ 30 fps = 4 s of video, with sync samples every 30 frames
    // (1 s, == the 1.0 s target duration) so at least 3 segments close via
    // the normal boundary path (not just the EOS flush tail), each carrying
    // several 500 ms parts (15 frames/part at 30 fps/90kHz).
    let mut batches = Vec::new();
    for i in 0..120u32 {
        let is_sync = i % 30 == 0;
        let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 64];
        let sample = Sample::new(
            data,
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }

    // `feed_via_segmenter` publishes `SPTS_PROGRAM_ID` itself before writing
    // (issue #805 task 6) — no separate publish call needed here.
    feed_via_segmenter(&store, specs, batches);

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
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

/// Bites the blocking-reload wakeup path (`output::hls`'s internal `wait_for_progress`):
/// without the `watch`-based wakeup + re-render, a `_HLS_msn`/`_HLS_part`
/// request for a not-yet-available part would either serve the stale
/// playlist (missing the new part) or hang to `BLOCKING_RELOAD_TIMEOUT`
/// (5 s) rather than resolving as soon as the part lands.
#[tokio::test]
async fn blocking_reload_resolves_when_part_arrives() {
    let store = Arc::new(RouteHandle::new(4.0, 500, 8));
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    store.set_init(program, vec![0xAA; 8]);
    store.add_part(program, part(1, 0));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
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

    store.add_part(program, part(1, 1));

    // NOT a pure hang guard (issue #807): this must stay meaningfully below
    // the 5s internal `BLOCKING_RELOAD_TIMEOUT` fallback it distinguishes
    // from, or a broken (never-wakes) watch would still pass by falling
    // through to that same fallback within a too-generous window. Bound the
    // wait in *real* time (no `tokio::time::pause()`): a working watch wakeup
    // resolves within milliseconds of `add_part`, whereas a broken wakeup
    // only resolves at the 5s fallback. 500 ms comfortably separates the two
    // (10x below the 5s cap) without being flaky on a loaded CI box.
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
    // (see `multimux::output::hls::media_playlist_m3u8`). Only a genuine PART line
    // proves the new part was actually rendered.
    // (The playlist's part names carry the origin's instance token:
    // `part-1-{instance}-1.1.m4s`.)
    assert!(
        playlist.lines().any(|l| l
            .strip_prefix("#EXT-X-PART:DURATION=0.5,URI=\"part-1-")
            .is_some_and(|rest| rest.ends_with("-1.1.m4s\""))),
        "resolved playlist must include the real #EXT-X-PART line for the \
         newly-arrived part (not just the preload-hint URI): {playlist}"
    );
}
