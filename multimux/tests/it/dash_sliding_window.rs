//! DASH / LL-DASH MPDs stay valid while the live window slides (issue #1058,
//! audit r07-C9).
//!
//! Before the fix `SegmentTemplate@startNumber` followed the window's OLDEST
//! segment while `MPD@availabilityStartTime` stayed fixed. ISO/IEC 23009-1
//! §5.3.9.5.3 defines the live edge of a `$Number$` + `@duration` template from
//! exactly those two attributes (segment N is available at
//! `AST + (N - startNumber + 1) * duration`), so once the window slid a player
//! computed segment numbers that were never published and got 404s.
//!
//! Here real H.264 access units (the committed `h264-aac-40s.ts` fixture) are
//! cut by the real `LlHlsSegmenter` into far more segments than the window
//! holds. After each new segment the MPD is fetched through the real router,
//! the segment numbers a DASH client derives from it AT THAT MOMENT (live edge
//! down to the time-shift-buffer start) are requested from the origin, and
//! every one must be served; the MPD must also pass `media-doctor`'s DASH
//! validator and keep `startNumber`/`@duration` constant as it refreshes.

use std::collections::HashMap;
use std::sync::Arc;
use transmux::TrackId;

use crate::support::origin::{body_string, get};
use axum::http::StatusCode;
use broadcast_common::Unpackage;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::dash::DashOutput;
use multimux::output::ll_dash::LlDashOutput;
use multimux::route::RouteHandle;
use tower::ServiceExt;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{Mpd, TsDemux};

const TARGET_SECS: f64 = 2.0;
const PART_MS: u32 = 500;
/// Segments the route's rolling window holds.
const WINDOW: usize = 3;
/// How many segments to publish: well past several slides.
const PUBLISHED: u32 = 12;

/// What a DASH client reads from one MPD fetch.
struct Edge {
    start_number: u64,
    /// `@duration` in seconds.
    seg_secs: f64,
    /// `@timeShiftBufferDepth` in seconds.
    tsbd_secs: f64,
    media: String,
}

fn read_mpd(xml: &str) -> Edge {
    let mpd = Mpd::parse(xml).unwrap_or_else(|e| panic!("MPD parse: {e:?}\n{xml}"));
    let t = mpd.periods[0].adaptation_sets[0]
        .segment_template
        .clone()
        .or_else(|| {
            mpd.periods[0].adaptation_sets[0].representations[0]
                .segment_template
                .clone()
        })
        .expect("SegmentTemplate");
    Edge {
        start_number: t.start_number,
        seg_secs: t.duration.expect("@duration") as f64 / t.timescale as f64,
        tsbd_secs: mpd
            .time_shift_buffer_depth
            .expect("timeShiftBufferDepth")
            .as_secs_f64(),
        media: t.media.expect("media template"),
    }
}

/// The segment numbers a client derives at `elapsed` seconds after
/// `availabilityStartTime` (ISO/IEC 23009-1 §5.3.9.5.3): from the start of the
/// time-shift buffer up to the newest fully available segment.
fn derivable(e: &Edge, elapsed: f64) -> std::ops::RangeInclusive<u64> {
    let newest = e.start_number + (elapsed / e.seg_secs).floor() as u64 - 1;
    let oldest_rel = ((elapsed - e.tsbd_secs) / e.seg_secs).ceil().max(0.0) as u64;
    (e.start_number + oldest_rel)..=newest
}

/// Run one output (`manifest_path`) through the sliding-window scenario.
async fn slide(output: Arc<dyn Output>, manifest_path: &str) {
    let media = TsDemux::new()
        .unpackage(&std::fs::read(test_fixtures::path("ts/h264-aac-40s.ts")).expect("fixture"))
        .expect("demux fixture");
    let mut track = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .expect("video track");
    // The single-rendition outputs name the track DEFAULT_TRACK_ID (1).
    track.spec.track_id = TrackId::new(1);
    let specs = vec![track.spec.clone()];

    let store = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, WINDOW));
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    let mut seg = LlHlsSegmenter::with_part_target(
        specs.clone(),
        transmux::VIDEO_CLOCK_RATE,
        TARGET_SECS,
        PART_MS,
    )
    .expect("segmenter");
    store.set_init(program, seg.init_segment().expect("init"));
    store.set_track_specs(program, specs);

    let mut streams = HashMap::new();
    streams.insert("cam".to_string(), (store.clone(), vec![output]));
    let app = router(Arc::new(AppState::new(streams)));

    let mut samples = track.samples.into_iter();
    let mut published = 0u32;
    let mut first: Option<Edge> = None;
    while published < PUBLISHED {
        let s = samples
            .next()
            .expect("fixture has enough video for the run");
        seg.push(TrackId::new(1), s).expect("push");
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add_segment");
            published += 1;

            // The MPD at this moment: exactly `published` segments exist and the
            // client's clock stands at their end, `published * D` after AST.
            let resp = app.clone().oneshot(get(manifest_path)).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let xml = body_string(resp).await;

            let mut report = media_doctor::Report::new();
            media_doctor::check_dash_mpd(&xml, &mut report);
            assert!(
                report.findings().is_empty(),
                "media-doctor DASH findings after {published} segments: {:?}\n{xml}",
                report.findings()
            );

            let edge = read_mpd(&xml);
            let base = first.get_or_insert_with(|| Edge {
                start_number: edge.start_number,
                seg_secs: edge.seg_secs,
                tsbd_secs: 0.0,
                media: edge.media.clone(),
            });
            assert_eq!(
                edge.start_number, base.start_number,
                "startNumber moved after {published} segments (window slid): {xml}"
            );
            assert_eq!(edge.seg_secs, base.seg_secs, "@duration changed: {xml}");

            let elapsed = f64::from(published) * edge.seg_secs;
            let range = derivable(&edge, elapsed);
            assert!(
                !range.is_empty(),
                "client derives no segments at {elapsed}s: {xml}"
            );
            for n in range.clone() {
                let uri = edge
                    .media
                    .replace("$RepresentationID$", "1")
                    .replace("$Number$", &n.to_string());
                let resp = app
                    .clone()
                    .oneshot(get(&format!("/cam/{uri}")))
                    .await
                    .unwrap();
                assert_eq!(
                    resp.status(),
                    StatusCode::OK,
                    "{uri} derived from the MPD after {published} segments \
                     (client range {range:?}) is not served:\n{xml}"
                );
            }
            // The newest derivable number is the newest published one.
            assert_eq!(
                *range.end(),
                edge.start_number + u64::from(published) - 1,
                "live edge disagrees with what was published"
            );
        }
    }
    assert!(
        PUBLISHED as usize > WINDOW * 2,
        "the scenario must slide the window several times"
    );
}

#[tokio::test]
async fn dash_mpd_stays_servable_as_the_window_slides() {
    slide(Arc::new(DashOutput), "/cam/manifest.mpd").await;
}

#[tokio::test]
async fn ll_dash_mpd_stays_servable_as_the_window_slides() {
    slide(Arc::new(LlDashOutput), "/cam/manifest-ll.mpd").await;
}

fn fixture_video_track() -> transmux::Track {
    let media = TsDemux::new()
        .unpackage(&std::fs::read(test_fixtures::path("ts/h264-aac-40s.ts")).expect("fixture"))
        .expect("demux fixture");
    let mut track = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .expect("video track");
    track.spec.track_id = TrackId::new(1);
    track
}

/// Publish `segments` closed segments of the fixture video as one source run
/// on `program`'s current `Trunk`.
fn publish_run(
    route: &RouteHandle,
    program: media_plane::ProgramId,
    track: &transmux::Track,
    segments: u32,
) {
    let specs = vec![track.spec.clone()];
    let mut seg = LlHlsSegmenter::with_part_target(
        specs.clone(),
        transmux::VIDEO_CLOCK_RATE,
        TARGET_SECS,
        PART_MS,
    )
    .expect("segmenter");
    route.set_init(program, seg.init_segment().expect("init"));
    route.set_track_specs(program, specs);
    let mut published = 0;
    for s in track.samples.iter().cloned() {
        seg.push(TrackId::new(1), s).expect("push");
        for part in seg.take_ready_parts() {
            route.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            route.add_segment(program, segment).expect("add_segment");
            published += 1;
        }
        if published >= segments {
            return;
        }
    }
    panic!("fixture too short for {segments} segments");
}

fn attribute(xml: &str, key: &str) -> String {
    let at = xml.find(key).unwrap_or_else(|| panic!("{key} in {xml}")) + key.len();
    xml[at..]
        .split('"')
        .next()
        .expect("closing quote")
        .to_string()
}

fn period_open_tag(xml: &str) -> String {
    let at = xml.find("<Period").expect("Period");
    xml[at..].split('>').next().expect("tag end").to_string()
}

async fn fetch(app: &axum::Router, path: &str) -> (StatusCode, String) {
    let resp = app.clone().oneshot(get(path)).await.unwrap();
    let status = resp.status();
    (status, body_string(resp).await)
}

/// A reconnect leaves `availabilityStartTime` and the Period untouched
/// (`private/specs-verified/dash/etsi-ts-103-285/ch04-dash-constraints-http-sync.md`
/// 4.8.3 items 2 and 5), keeps the numbers counting, and advertises only
/// numbers that are served, with `publishTime` moving forward.
async fn run_then_reconnect_then_run(output: Arc<dyn Output>, manifest_path: &str) {
    let track = fixture_video_track();
    let route = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, 8));
    let program = media_plane::ProgramId(0);
    let mut streams = HashMap::new();
    streams.insert("cam".to_string(), (route.clone(), vec![output]));
    let app = router(Arc::new(AppState::new(streams)));

    let first = route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    let (status, before) = fetch(&app, manifest_path).await;
    assert_eq!(status, StatusCode::OK);

    route.release_program(program, &first);
    route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    let (status, after) = fetch(&app, manifest_path).await;
    assert_eq!(status, StatusCode::OK);

    for key in ["availabilityStartTime=\"", "startNumber=\""] {
        assert_eq!(
            attribute(&before, key),
            attribute(&after, key),
            "{key} changed"
        );
    }
    assert_eq!(period_open_tag(&before), period_open_tag(&after));
    assert!(
        attribute(&before, "publishTime=\"") < attribute(&after, "publishTime=\""),
        "publishTime must move forward across the reconnect"
    );

    let edge = read_mpd(&after);
    // The replaced program's open segment keeps its number (RFC 8216bis
    // §6.2.2 shared offset), so the new run's segments are 5..=7.
    let range = derivable(&edge, 7.0 * edge.seg_secs);
    assert_eq!(range, 5..=7);
    for n in range {
        let uri = edge
            .media
            .replace("$RepresentationID$", "1")
            .replace("$Number$", &n.to_string());
        let status = app
            .clone()
            .oneshot(get(&format!("/cam/{uri}")))
            .await
            .unwrap()
            .status();
        assert_eq!(
            status,
            StatusCode::OK,
            "{uri} advertised after the reconnect"
        );
    }
}

#[tokio::test]
async fn dash_segments_derived_from_the_mpd_are_served_after_a_source_reconnect() {
    run_then_reconnect_then_run(Arc::new(DashOutput), "/cam/manifest.mpd").await;
}

#[tokio::test]
async fn ll_dash_segments_derived_from_the_mpd_are_served_after_a_source_reconnect() {
    run_then_reconnect_then_run(Arc::new(LlDashOutput), "/cam/manifest-ll.mpd").await;
}
