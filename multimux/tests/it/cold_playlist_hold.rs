//! A media-playlist request without `_HLS_msn`/`_HLS_part` made before the
//! playlist holds a closed segment is held until the first one closes (issue
//! #1754), then answered with at least one `EXTINF`. RFC 8216bis section
//! 6.2.5.2 bounds the hold: a Server that cannot provide the Playlist after
//! blocking more than three Target Durations SHOULD answer 503.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{StatusCode, header};
use tokio::time::Instant;
use tower::ServiceExt;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::origin::{AppState, router};
use multimux::output::llhls::LlHlsOutput;
use multimux::output::{Output, OutputKind};
use multimux::registry::SchemeRegistry;
use multimux::route::RouteHandle;
use multimux::serve_with_registry_on;
use test_fixtures::process::{output_bounded, tool_available};

use crate::support::listener::bind_tcp;
use crate::support::origin::{body_string, get};
use crate::support::segmenter_feed::{
    FRAME_DUR, PART_TARGET_MS, TARGET_DURATION_SECS, drain_ready, feed_open_segment,
    video_batches_n, video_track_spec,
};

const FRAMES_PER_SEGMENT: u32 = 30;
const COLD_FRAMES: u32 = 20;
const SEGMENT_CLOSING_FRAMES: u32 = FRAMES_PER_SEGMENT + 2;
const PUBLISH_DELAY: Duration = Duration::from_secs(1);
const MIN_BLOCKING_TIMEOUT: Duration = Duration::from_secs(5);

fn cold_route() -> (Arc<RouteHandle>, transmux::ll_hls::LlHlsSegmenter) {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let seg = feed_open_segment(
        &store,
        vec![video_track_spec()],
        video_batches_n(COLD_FRAMES),
    );
    (store, seg)
}

fn push_frames(
    store: &RouteHandle,
    seg: &mut transmux::ll_hls::LlHlsSegmenter,
    frames: std::ops::Range<u32>,
) {
    for i in frames {
        let ts = i64::from(i) * i64::from(FRAME_DUR);
        let sample = transmux::pipeline::Sample::new(
            vec![0xAA; 64],
            Some(ts),
            Some(ts),
            Some(FRAME_DUR),
            i % FRAMES_PER_SEGMENT == 0,
        );
        seg.push(transmux::TrackId::new(1), sample).expect("push");
        drain_ready(store, seg);
    }
}

fn app(store: &Arc<RouteHandle>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            Arc::clone(store),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    router(Arc::new(AppState::new(streams)))
}

#[tokio::test(start_paused = true)]
async fn cold_request_is_held_until_the_first_segment_closes() {
    let (store, mut seg) = cold_route();
    let app = app(&store);

    let started = Instant::now();
    let request = app.clone().oneshot(get("/cam/media.m3u8"));
    let publish = async {
        tokio::time::sleep(PUBLISH_DELAY).await;
        push_frames(&store, &mut seg, COLD_FRAMES..SEGMENT_CLOSING_FRAMES);
    };
    let (resp, ()) = tokio::join!(request, publish);
    let resp = resp.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        started.elapsed() >= PUBLISH_DELAY,
        "answered after {:?}, before the segment closed",
        started.elapsed()
    );
    let body = body_string(resp).await;
    assert!(body.contains("#EXTINF:"), "body: {body}");

    let again = Instant::now();
    let resp = app.clone().oneshot(get("/cam/media.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        again.elapsed(),
        Duration::ZERO,
        "a warm request is immediate"
    );
    assert!(body_string(resp).await.contains("#EXTINF:"));
}

#[tokio::test(start_paused = true)]
async fn cold_hold_that_outlasts_the_blocking_timeout_is_503() {
    let (store, _seg) = cold_route();
    let app = app(&store);

    let started = Instant::now();
    let resp = app.clone().oneshot(get("/cam/media.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(resp.headers().contains_key(header::RETRY_AFTER));
    assert!(
        started.elapsed() >= MIN_BLOCKING_TIMEOUT - Duration::from_millis(100),
        "gave up after {:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn msn_requests_on_a_cold_stream_keep_their_semantics() {
    let (store, mut seg) = cold_route();
    let app = app(&store);

    let started = Instant::now();
    let resp = app
        .clone()
        .oneshot(get("/cam/media.m3u8?_HLS_msn=1&_HLS_part=0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        started.elapsed(),
        Duration::ZERO,
        "an existing part is served at once"
    );
    assert!(!body_string(resp).await.contains("#EXTINF:"));

    let resp = app
        .clone()
        .oneshot(get("/cam/media.m3u8?_HLS_part=0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let resp = app
        .clone()
        .oneshot(get("/cam/media.m3u8?_HLS_msn=99"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let request = app.clone().oneshot(get("/cam/media.m3u8?_HLS_msn=1"));
    let publish = async {
        tokio::time::sleep(PUBLISH_DELAY).await;
        push_frames(&store, &mut seg, COLD_FRAMES..SEGMENT_CLOSING_FRAMES);
    };
    let (resp, ()) = tokio::join!(request, publish);
    assert_eq!(resp.unwrap().status(), StatusCode::OK);
}

const FIXTURE_TARGET_SECS: f64 = 2.0;
const FIXTURE_PART_TARGET_MS: u32 = 200;
const FIXTURE_WINDOW_SEGMENTS: usize = 8;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const HANG_GUARD: Duration = Duration::from_secs(30);
const WARM_REQUEST_BOUND: Duration = Duration::from_millis(500);
const FFMPEG_DEADLINE: Duration = Duration::from_secs(60);
const FFMPEG_READ_SECS: &str = "3";

fn fixture_config(bind: std::net::SocketAddr, loop_file: bool) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: FIXTURE_TARGET_SECS,
        part_target_ms: FIXTURE_PART_TARGET_MS,
        window_segments: FIXTURE_WINDOW_SEGMENTS,
        routes: vec![Route {
            name: "cam".to_string(),
            input: InputSpec::File {
                path: test_fixtures::path("ts/h264-aac.ts")
                    .to_string_lossy()
                    .into_owned(),
                loop_file,
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }],
        ..Config::default()
    }
}

async fn first_answered_playlist(client: &reqwest::Client, url: &str) -> String {
    let deadline = Instant::now() + HANG_GUARD;
    loop {
        let resp = client.get(url).send().await.expect("GET media playlist");
        if resp.status() != reqwest::StatusCode::SERVICE_UNAVAILABLE {
            assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET {url}");
            return resp.text().await.expect("playlist body");
        }
        assert!(Instant::now() < deadline, "route never left 503: {url}");
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_for_init(client: &reqwest::Client, url: &str) {
    let deadline = Instant::now() + HANG_GUARD;
    loop {
        let resp = client.get(url).send().await.expect("GET init segment");
        if resp.status() == reqwest::StatusCode::OK {
            return;
        }
        assert!(Instant::now() < deadline, "route never served {url}");
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[tokio::test]
async fn cold_file_route_first_answer_carries_a_segment() {
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        fixture_config(bind, false),
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let url = format!("http://{bind}/cam/media.m3u8");

    let first = first_answered_playlist(&client, &url).await;
    assert!(first.contains("#EXTINF:"), "first answer: {first}");

    let started = std::time::Instant::now();
    let second = client.get(&url).send().await.expect("second GET");
    assert_eq!(second.status(), reqwest::StatusCode::OK);
    assert!(second.text().await.expect("body").contains("#EXTINF:"));
    assert!(
        started.elapsed() < WARM_REQUEST_BOUND,
        "a request after the first segment took {:?}",
        started.elapsed()
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn ffmpeg_opens_a_cold_stream() {
    if !tool_available("ffmpeg", "-version") {
        eprintln!("SKIP ffmpeg_opens_a_cold_stream: ffmpeg not on PATH");
        return;
    }
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        fixture_config(bind, true),
        SchemeRegistry::new(),
    ));
    let url = format!("http://{bind}/cam/media.m3u8");
    wait_for_init(
        &reqwest::Client::new(),
        &format!("http://{bind}/cam/init-1.mp4"),
    )
    .await;

    let probe = url.clone();
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-nostdin", "-v", "error", "-i", &probe]).args([
            "-t",
            FFMPEG_READ_SECS,
            "-f",
            "null",
            "-",
        ]);
        output_bounded(&mut cmd, FFMPEG_DEADLINE)
    })
    .await
    .expect("join")
    .expect("run ffmpeg");
    server.abort();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ffmpeg failed: {stderr}");
    assert!(!stderr.contains("Empty segment"), "ffmpeg stderr: {stderr}");
}
