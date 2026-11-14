//! Load soak (#2515 item 6): one ingest feeding a route, `VIEWERS` LL-HLS
//! blocking-reload viewers and `CONSUMERS` sample consumers (the WHEP-style
//! cursor readers), all on one multi-thread runtime for `SOAK_SECONDS`.
//!
//! Gated by `RUST_BROADCAST_SOAK=1` (the nightly soak env); without it the
//! test prints why it skipped and passes.
//!
//! Asserts zero failed viewer requests, a viewer throughput floor, a p99
//! response-after-publish ceiling for the viewers, a delivery floor for the
//! consumers and a p99 publish-to-receive ceiling for them.
//!
//! Measured (24-core Apple-silicon macOS laptop, debug test profile, 20 s at
//! 1000 viewers and 200 consumers): 40 responses per viewer (a part every
//! 500 ms), 0 failures, viewer p50 9 ms / p99 21 ms after the publish;
//! 400000 of 400000 sample deliveries, consumer p50 0 ms / p99 7 ms. The
//! floors and ceilings below are 2x to 50x off those, for a shared CI runner.

use multimux::route::RouteConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::support::origin::get;
use axum::http::StatusCode;
use media_plane::{RetentionClass, SampleCursorItem};
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;
use tower::ServiceExt;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{TrackId, avc_config_from_sprop};

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const FRAME_DUR: u32 = transmux::VIDEO_CLOCK_RATE / 30;
const TARGET_DURATION_SECS: f64 = 2.0;
const PART_TARGET_MS: u32 = 500;
const FRAMES_PER_SEGMENT: u32 = 60;
const TICK: Duration = Duration::from_millis(10);
const TICKS_PER_FRAME: u64 = 3;
const SOAK_SECONDS: u64 = 20;
const VIEWERS: usize = 1000;
const CONSUMERS: usize = 200;
const SAMPLE_BYTES: usize = 1316;
const CONSUMER_POLL_LIMIT: Duration = Duration::from_millis(100);
const NANOS_PER_MILLI: u64 = 1_000_000;

const MIN_VIEWER_RESPONSES_PER_VIEWER: u64 = 20;
const VIEWER_P99_CEILING: Duration = Duration::from_millis(1000);
const MIN_CONSUMER_DELIVERY_PERCENT: u64 = 95;
const CONSUMER_P99_CEILING: Duration = Duration::from_millis(250);

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn next_reload(playlist: &str) -> (u64, usize) {
    let first: u64 = playlist
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut closed = 0u64;
    let mut parts_after_last_segment = 0usize;
    for line in playlist.lines() {
        if line.starts_with("#EXTINF") {
            closed += 1;
            parts_after_last_segment = 0;
        } else if line.starts_with("#EXT-X-PART:") {
            parts_after_last_segment += 1;
        }
    }
    (first + closed, parts_after_last_segment)
}

#[tokio::test(flavor = "multi_thread")]
async fn soak_blocking_reload_viewers_and_sample_consumers() {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none() {
        eprintln!("SKIPPED soak_load: set RUST_BROADCAST_SOAK=1 to run the load soak");
        return;
    }

    let route = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target(std::time::Duration::from_millis(u64::from(PART_TARGET_MS)))
                .with_window_segments(8),
        )
        .with_declared_bandwidth_bps(5_000_000),
    );
    let spec = TrackSpec::new(
        TrackId::new(1),
        transmux::VIDEO_CLOCK_RATE,
        CodecConfig::Avc {
            config: avc_config_from_sprop(SPROP).expect("valid sprop"),
            width: 0,
            height: 0,
        },
    );
    let mut segmenter = LlHlsSegmenter::with_part_target(
        vec![spec],
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    let program = media_plane::ProgramId(0);
    let trunk = route.publish_new_program(program);
    route.set_init(program, segmenter.init_segment().expect("init segment"));
    let sample_writer = trunk.writer().expect("sample writer");

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            route.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams).with_limits(
        multimux::origin::HttpLimits {
            request_timeout: Duration::from_secs(30),
            max_concurrent_requests: VIEWERS * 2,
            max_blocking_reloads_per_route: VIEWERS * 2,
            ..multimux::origin::HttpLimits::default()
        },
    )));

    let base = Instant::now();
    let last_event = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let stamp = |base: Instant| base.elapsed().as_nanos() as u64;

    let mut consumers = tokio::task::JoinSet::new();
    for _ in 0..CONSUMERS {
        let mut cursor = trunk.subscribe();
        let stop = Arc::clone(&stop);
        consumers.spawn(async move {
            let mut latencies = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let Ok(item) = tokio::time::timeout(CONSUMER_POLL_LIMIT, cursor.next()).await
                else {
                    continue;
                };
                if let SampleCursorItem::Timed { sample, .. } = item
                    && let Some(at) = sample.pts
                {
                    latencies.push(stamp(base).saturating_sub(at as u64));
                }
            }
            latencies
        });
    }

    let mut viewers = tokio::task::JoinSet::new();
    for _ in 0..VIEWERS {
        let (app, last_event, stop) = (app.clone(), Arc::clone(&last_event), Arc::clone(&stop));
        viewers.spawn(async move {
            let mut latencies = Vec::new();
            let mut failures = 0u64;
            let mut uri = "/cam/media.m3u8".to_string();
            while !stop.load(Ordering::Relaxed) {
                let resp = app.clone().oneshot(get(&uri)).await.expect("response");
                let received = stamp(base);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if resp.status() != StatusCode::OK {
                    failures += 1;
                    break;
                }
                let body = crate::support::origin::body_string(resp).await;
                let (msn, part) = next_reload(&body);
                if uri.contains("_HLS_msn") {
                    latencies.push(received.saturating_sub(last_event.load(Ordering::Relaxed)));
                }
                uri = format!("/cam/media.m3u8?_HLS_msn={msn}&_HLS_part={part}");
            }
            (latencies, failures)
        });
    }

    let mut ticker = tokio::time::interval(TICK);
    let mut frame = 0u32;
    let mut published = 0u64;
    let total_ticks = SOAK_SECONDS * 1000 / TICK.as_millis() as u64;
    for tick in 0..total_ticks {
        ticker.tick().await;
        let at = stamp(base);
        let pts = at as i64;
        sample_writer.publish(
            TrackId::new(1),
            RetentionClass::Timed,
            Sample::new(vec![0xAB; SAMPLE_BYTES], Some(pts), Some(pts), None, true),
        );
        published += 1;
        if tick % TICKS_PER_FRAME == 0 {
            let pts = i64::from(frame) * i64::from(FRAME_DUR);
            last_event.store(stamp(base), Ordering::Relaxed);
            segmenter
                .push(
                    TrackId::new(1),
                    Sample::new(
                        vec![0xAA; 64],
                        Some(pts),
                        Some(pts),
                        Some(FRAME_DUR),
                        frame.is_multiple_of(FRAMES_PER_SEGMENT),
                    ),
                )
                .expect("push");
            for part in segmenter.take_ready_parts() {
                route.add_part(program, part);
            }
            for segment in segmenter.take_ready_segments() {
                route.add_segment(program, segment).expect("segment");
            }
            frame += 1;
        }
    }
    stop.store(true, Ordering::Relaxed);
    drop(sample_writer);

    let mut viewer_latencies = Vec::new();
    let mut failures = 0u64;
    while let Some(done) = viewers.join_next().await {
        let (latencies, failed) = done.expect("viewer task");
        viewer_latencies.extend(latencies);
        failures += failed;
    }
    let mut consumer_latencies = Vec::new();
    while let Some(done) = consumers.join_next().await {
        consumer_latencies.extend(done.expect("consumer task"));
    }
    viewer_latencies.sort_unstable();
    consumer_latencies.sort_unstable();
    let viewer_p99 = Duration::from_nanos(percentile(&viewer_latencies, 0.99));
    let consumer_p99 = Duration::from_nanos(percentile(&consumer_latencies, 0.99));
    eprintln!(
        "soak_load: viewer responses {} (per viewer {}), failures {failures}, viewer p50 {} ms p99 {} ms; \
         consumer deliveries {} of {}, consumer p50 {} ms p99 {} ms",
        viewer_latencies.len(),
        viewer_latencies.len() / VIEWERS,
        percentile(&viewer_latencies, 0.5) / NANOS_PER_MILLI,
        viewer_p99.as_millis(),
        consumer_latencies.len(),
        published * CONSUMERS as u64,
        percentile(&consumer_latencies, 0.5) / NANOS_PER_MILLI,
        consumer_p99.as_millis(),
    );

    assert_eq!(failures, 0, "every viewer request must be answered 200");
    assert!(
        viewer_latencies.len() as u64 >= MIN_VIEWER_RESPONSES_PER_VIEWER * VIEWERS as u64,
        "viewer throughput below the floor: {} responses",
        viewer_latencies.len()
    );
    assert!(
        viewer_p99 <= VIEWER_P99_CEILING,
        "viewer p99 {viewer_p99:?}"
    );
    assert!(
        consumer_latencies.len() as u64 * 100
            >= published * CONSUMERS as u64 * MIN_CONSUMER_DELIVERY_PERCENT,
        "consumers received {} of {}",
        consumer_latencies.len(),
        published * CONSUMERS as u64
    );
    assert!(
        consumer_p99 <= CONSUMER_P99_CEILING,
        "consumer p99 {consumer_p99:?}"
    );
}
