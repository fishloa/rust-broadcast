//! Bug audit F: DASH `$Number$` addressing across a source reconnect.
//!
//! A reconnect binds a fresh `Trunk` (segments numbered from 1 again). The HLS
//! origin continues its Media Sequence Number through an offset, and the shared
//! resource route names segments by that offset number, while the DASH MPD
//! keeps `startNumber=1`. A DASH client therefore derives numbers the origin
//! does not serve.

#[allow(dead_code)]
#[path = "support/origin.rs"]
mod origin_support;

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::dash::DashOutput;
use multimux::route::RouteHandle;
use origin_support::{body_string, get};
use tower::ServiceExt;
use transmux::TrackId;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{Mpd, TsDemux};

use broadcast_common::Unpackage;

const TARGET_SECS: f64 = 2.0;
const PART_MS: u32 = 500;

fn video_track() -> transmux::Track {
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

#[tokio::test]
async fn control_without_a_reconnect_serves_every_derived_number() {
    let track = video_track();
    let route = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, 8));
    let program = media_plane::ProgramId(0);
    route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    assert_derived_numbers_served(route).await;
}

#[tokio::test]
async fn dash_numbers_derived_from_the_mpd_are_served_after_a_reconnect() {
    let track = video_track();
    let route = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, 8));
    let program = media_plane::ProgramId(0);
    let first = route.publish_new_program(program);
    publish_run(&route, program, &track, 3);

    route.release_program(program, &first);
    route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    assert_derived_numbers_served(route).await;
}

async fn assert_derived_numbers_served(route: Arc<RouteHandle>) {
    derived_numbers_served(route, Arc::new(DashOutput), "/cam/manifest.mpd").await;
}

async fn derived_numbers_served(route: Arc<RouteHandle>, output: Arc<dyn Output>, path: &str) {
    let app = app_with(route, output);

    let resp = app.clone().oneshot(get(path)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = body_string(resp).await;
    let mpd = Mpd::parse(&xml).expect("MPD parse");
    let t = mpd.periods[0].adaptation_sets[0]
        .segment_template
        .clone()
        .or_else(|| {
            mpd.periods[0].adaptation_sets[0].representations[0]
                .segment_template
                .clone()
        })
        .expect("SegmentTemplate");
    let media = t.media.expect("media template");

    // The new run published three segments; a client three segments after
    // availabilityStartTime derives startNumber..startNumber+2.
    let mut failures = Vec::new();
    for n in t.start_number..t.start_number + 3 {
        let uri = media
            .replace("$RepresentationID$", "1")
            .replace("$Number$", &n.to_string());
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{uri}")))
            .await
            .unwrap();
        if resp.status() != StatusCode::OK {
            failures.push(format!("{uri} -> {}", resp.status()));
        }
    }
    assert!(
        failures.is_empty(),
        "DASH segments derived from the post-reconnect MPD are not served: {failures:?}\n{xml}"
    );
}

fn app_with(route: Arc<RouteHandle>, output: Arc<dyn Output>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert("cam".to_string(), (route, vec![output]));
    router(Arc::new(AppState::new(streams)))
}

async fn manifest_status(
    reconnect: bool,
    output: Arc<dyn Output>,
    path: &str,
) -> (StatusCode, String) {
    let track = video_track();
    let route = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, 8));
    let program = media_plane::ProgramId(0);
    let first = route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    if reconnect {
        route.release_program(program, &first);
        route.publish_new_program(program);
        publish_run(&route, program, &track, 3);
    }
    let app = app_with(route, output);
    let resp = app.oneshot(get(path)).await.unwrap();
    let status = resp.status();
    (status, body_string(resp).await)
}

#[tokio::test]
async fn control_smooth_manifest_is_served_without_a_reconnect() {
    let (status, body) = manifest_status(
        false,
        Arc::new(multimux::output::smooth::SmoothOutput),
        "/cam/Manifest",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn smooth_manifest_is_served_after_a_reconnect() {
    let (status, body) = manifest_status(
        true,
        Arc::new(multimux::output::smooth::SmoothOutput),
        "/cam/Manifest",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Smooth manifest after a reconnect: {body}"
    );
}

#[tokio::test]
async fn ll_dash_numbers_derived_from_the_mpd_are_served_after_a_reconnect() {
    let track = video_track();
    let route = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, 8));
    let program = media_plane::ProgramId(0);
    let first = route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    route.release_program(program, &first);
    route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    derived_numbers_served(
        route,
        Arc::new(multimux::output::ll_dash::LlDashOutput),
        "/cam/manifest-ll.mpd",
    )
    .await;
}

fn video_chunks(xml: &str) -> Vec<(u64, u64)> {
    let manifest = transmux::SmoothManifest::parse(xml).expect("Smooth manifest parse");
    manifest.streams[0].enumerate_chunks().expect("chunks")
}

#[tokio::test]
async fn smooth_timeline_does_not_go_backwards_across_a_reconnect() {
    let track = video_track();
    let route = Arc::new(RouteHandle::new(TARGET_SECS, PART_MS, 8));
    let program = media_plane::ProgramId(0);
    let first = route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    let app = app_with(
        route.clone(),
        Arc::new(multimux::output::smooth::SmoothOutput),
    );
    let before = body_string(app.clone().oneshot(get("/cam/Manifest")).await.unwrap()).await;
    let (last_t, last_d) = *video_chunks(&before).last().expect("chunks before");

    route.release_program(program, &first);
    route.publish_new_program(program);
    publish_run(&route, program, &track, 3);
    let after = body_string(app.oneshot(get("/cam/Manifest")).await.unwrap()).await;
    let (first_t, _) = video_chunks(&after)[0];
    assert!(
        first_t >= last_t + last_d,
        "first chunk after the reconnect starts at {first_t}, before the previous run ended at {}",
        last_t + last_d
    );
}
