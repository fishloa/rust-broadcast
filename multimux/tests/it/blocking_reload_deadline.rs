//! A Playlist blocking reload waits three Target Durations before it answers
//! 503 (RFC 8216bis section 6.2.5.2, issue #1651), not a fixed 5 s.

use multimux::route::RouteConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

use crate::support::origin::get;
use axum::http::{StatusCode, header};
use tower::ServiceExt;
use transmux::TrackId;
use transmux::avc_config_from_sprop;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const FRAME_DUR: u32 = transmux::VIDEO_CLOCK_RATE / 30;
const TARGET_DURATION_SECS: f64 = 3.0;
const PART_TARGET_MS: u32 = 500;
const FRAMES_PER_SEGMENT: u32 = 120;
const FIXED_TIMEOUT: Duration = Duration::from_secs(5);
const THREE_TARGET_DURATIONS: Duration = Duration::from_secs(12);

struct Feeder {
    store: Arc<RouteHandle>,
    seg: LlHlsSegmenter,
}

impl Feeder {
    fn new() -> Self {
        let store = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration_secs(TARGET_DURATION_SECS)
                    .with_part_target(std::time::Duration::from_millis(u64::from(PART_TARGET_MS)))
                    .with_window_segments(8),
            )
            .with_declared_bandwidth_bps(5_000_000),
        );
        let config = avc_config_from_sprop(SPROP).expect("valid sprop");
        let spec = TrackSpec::new(
            TrackId::new(1),
            transmux::VIDEO_CLOCK_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        );
        let seg = LlHlsSegmenter::with_part_target(
            vec![spec],
            transmux::VIDEO_CLOCK_TIMESCALE,
            TARGET_DURATION_SECS,
            PART_TARGET_MS,
        )
        .expect("segmenter builds");
        let program = media_plane::ProgramId(0);
        store.publish_new_program(program);
        store.set_init(program, seg.init_segment().expect("init segment builds"));
        Self { store, seg }
    }

    fn frames(&mut self, range: std::ops::Range<u32>) {
        let program = media_plane::ProgramId(0);
        for i in range {
            let pts = i64::from(i) * i64::from(FRAME_DUR);
            let sample = Sample::new(
                vec![0xAA; 64],
                Some(pts),
                Some(pts),
                Some(FRAME_DUR),
                i % FRAMES_PER_SEGMENT == 0,
            );
            self.seg.push(TrackId::new(1), sample).expect("push");
            for part in self.seg.take_ready_parts() {
                self.store.add_part(program, part);
            }
            for segment in self.seg.take_ready_segments() {
                self.store.add_segment(program, segment).expect("segment");
            }
        }
    }
}

fn app(store: &Arc<RouteHandle>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    router(Arc::new(AppState::new(streams).with_limits(
        multimux::origin::HttpLimits {
            request_timeout: Duration::from_secs(30),
            ..multimux::origin::HttpLimits::default()
        },
    )))
}

async fn open_segment_msn(app: &axum::Router) -> u64 {
    let resp = app.clone().oneshot(get("/cam/media.m3u8")).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("#EXT-X-TARGETDURATION:4"), "body: {body}");
    let first: u64 = body
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:"))
        .unwrap()
        .parse()
        .unwrap();
    first + body.matches("#EXTINF").count() as u64
}

#[tokio::test(start_paused = true)]
async fn blocking_reload_waits_beyond_five_seconds_when_target_is_longer() {
    let mut feeder = Feeder::new();
    feeder.frames(0..FRAMES_PER_SEGMENT * 2 + FRAMES_PER_SEGMENT / 2);
    let app = app(&feeder.store);
    let msn = open_segment_msn(&app).await;
    let late = FIXED_TIMEOUT + Duration::from_millis(500);
    assert!(late < THREE_TARGET_DURATIONS);

    let request = app
        .clone()
        .oneshot(get(&format!("/cam/media.m3u8?_HLS_msn={msn}")));
    let publish = async {
        tokio::time::sleep(late).await;
        feeder.frames(FRAMES_PER_SEGMENT * 2 + FRAMES_PER_SEGMENT / 2..FRAMES_PER_SEGMENT * 3 + 2);
    };
    let (resp, ()) = tokio::join!(request, publish);
    assert_eq!(resp.unwrap().status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn blocking_reload_past_three_target_durations_is_503() {
    let mut feeder = Feeder::new();
    feeder.frames(0..FRAMES_PER_SEGMENT * 2 + FRAMES_PER_SEGMENT / 2);
    let app = app(&feeder.store);
    let msn = open_segment_msn(&app).await;

    let started = Instant::now();
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/media.m3u8?_HLS_msn={msn}")))
        .await
        .unwrap();
    let waited = started.elapsed();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(resp.headers().contains_key(header::RETRY_AFTER));
    assert!(
        waited >= THREE_TARGET_DURATIONS - Duration::from_millis(100),
        "gave up after {waited:?}, before three Target Durations"
    );
}
