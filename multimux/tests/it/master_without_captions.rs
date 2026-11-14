//! A route whose video carries no captions serves the same master playlist
//! it did before caption detection existed (issue #1428).

use multimux::route::RouteConfig;
use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_common::mux::Unpackage;
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use tower::ServiceExt;
use transmux::ts_demux::TsDemux;

use crate::support::shared_it::nz;
use media_plane::segmenter::ProgramSegmenter;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 200;
const WINDOW_SEGMENTS: usize = 64;
const RING: usize = 1 << 16;
const SIDE_RINGS: usize = 64;
const PUMP_EVERY: usize = 64;
const DECLARED_BPS: u64 = 16_000_000;
const STREAM: &str = "cam";

/// Captured from origin/main before caption detection landed.
const EXPECTED_MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=16000000,AVERAGE-BANDWIDTH=164357,CODECS=\"avc1.4D400D,mp4a.40.2\",RESOLUTION=320x240,FRAME-RATE=25.000\nmedia.m3u8\n#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=39288,AVERAGE-BANDWIDTH=37933,CODECS=\"avc1.4D400D\",RESOLUTION=320x240,URI=\"iframe.m3u8\"\n";

#[tokio::test]
async fn master_without_captions_is_unchanged() {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let media = TsDemux::new().unpackage(&bytes).expect("demux fixture");
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(RING))
            .with_sparse_capacity(nz(SIDE_RINGS))
            .with_segment_capacity(nz(WINDOW_SEGMENTS))
            .with_event_capacity(nz(SIDE_RINGS))
            .with_part_capacity(nz(RING)),
    );
    let writer = trunk.writer().expect("trunk writer");
    let specs: Vec<_> = media
        .tracks
        .iter()
        .filter(|t| t.spec.config.is_muxable_in_bmff())
        .map(|t| t.spec.clone())
        .collect();
    writer.set_tracks(specs.clone());
    let route = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target(std::time::Duration::from_millis(u64::from(PART_TARGET_MS)))
                .with_window_segments(WINDOW_SEGMENTS),
        )
        .with_declared_bandwidth_bps(DECLARED_BPS),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    route.set_track_specs(ProgramId(0), trunk.tracks().to_vec());
    let mut segmenter =
        ProgramSegmenter::try_new_with_siblings(&trunk, route.segmenter_config(), route.as_ref())
            .0
            .expect("segmenter");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }
    let mut order: Vec<(f64, usize, usize)> = Vec::new();
    for (ti, track) in media.tracks.iter().enumerate() {
        if !track.spec.config.is_muxable_in_bmff() {
            continue;
        }
        for (si, s) in track.samples.iter().enumerate() {
            let dts = s
                .timing
                .and_then(|t| t.dts())
                .or(s.timing.and_then(|t| t.pts()))
                .unwrap_or(0);
            order.push((dts as f64 / f64::from(track.spec.timescale.get()), ti, si));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (n, (_, ti, si)) in order.into_iter().enumerate() {
        let track = &media.tracks[ti];
        let mut sample = track.samples[si].clone();
        if let Some(t) = sample.timing.as_mut()
            && t.dts() == Some(0)
        {
            t.shift(t.pts().unwrap_or(0));
        }
        writer.publish(track.spec.track_id, RetentionClass::Timed, sample);
        if n % PUMP_EVERY == 0 {
            multimux::source::segment::pump_segmenter(&mut segmenter, ProgramId(0), &trunk, &route);
        }
    }
    multimux::source::segment::pump_segmenter(&mut segmenter, ProgramId(0), &trunk, &route);
    segmenter.flush();

    let mut streams = HashMap::new();
    streams.insert(
        STREAM.to_string(),
        (
            route,
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/{STREAM}/master.m3u8"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let master = String::from_utf8(body.to_vec()).expect("utf8");
    assert_eq!(master, EXPECTED_MASTER);
}
