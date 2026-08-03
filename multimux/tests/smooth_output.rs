//! Integration tests for the Smooth Streaming output (issue #742).
//!
//! Test 1: The manifest is served and is well-formed — parses as
//!   `SmoothManifest`, has quality levels and track entries, durations are
//!   non-zero.
//! Test 2: Advertised == servable — parse the manifest, extract fragment
//!   URLs, request them, assert real bytes come back.
//! Test 3: Auth is enforced — unauthenticated requests to manifest and
//!   fragments are rejected on a route with output auth.
//! Test 4: A route with `["llhls","smooth"]` serves both from one ingest,
//!   proving shared Trunk.
//! Test 5: Config round-trip — `"smooth"` deserialises and validates.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_auth::Credentials;
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::output::smooth::SmoothOutput;
use multimux::route::RouteHandle;
use transmux::avc_config_from_sprop;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::smooth_parse::SmoothManifest;

/// A real-ish sprop-parameter-sets pair (SPS+PPS) — same one used by
/// `multimux::source::rtsp`'s own tests.
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
/// 90 kHz video timescale.
const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 500;

fn video_track_spec() -> TrackSpec {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        1,
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 1280,
            height: 720,
        },
    )
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

/// Feed video frames through a real `LlHlsSegmenter` into `store`,
/// publishing the SPTS program.
fn feed_via_segmenter(
    store: &RouteHandle,
    specs: Vec<TrackSpec>,
    batches: Vec<Vec<(u32, Sample)>>,
) {
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);

    let mut seg = LlHlsSegmenter::with_part_target(
        specs.clone(),
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init segment builds"));
    store.set_track_specs(program, specs);

    for batch in batches {
        for (track_id, sample) in batch {
            seg.push(track_id, sample).expect("push succeeds");
        }
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment);
        }
    }

    seg.flush().expect("flush succeeds");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment);
    }
}

/// Build an axum app from a route handle with Smooth output.
fn build_app(store: Arc<RouteHandle>, output: Arc<dyn Output>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert("cam".to_string(), (store.clone(), vec![output]));
    router(Arc::new(AppState::new(streams)))
}

/// Test 1: The manifest is served and is well-formed.
#[tokio::test]
async fn manifest_is_served_and_well_formed() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    // 60 frames = ~2s of video, enough for 2 segments.
    let mut batches = Vec::new();
    for i in 0..60u32 {
        let is_sync = i % 30 == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    let app = build_app(store.clone(), Arc::new(SmoothOutput));

    // Request the manifest
    let resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK);

    let manifest_xml = body_string(resp).await;
    let manifest = SmoothManifest::parse(&manifest_xml)
        .unwrap_or_else(|e| panic!("manifest parse failed: {e:?}\nmanifest:\n{manifest_xml}"));

    assert_eq!(manifest.major_version, 2);
    assert!(
        !manifest.streams.is_empty(),
        "must have at least one StreamIndex"
    );
    assert!(
        manifest.duration.unwrap_or(0) > 0,
        "duration must be non-zero"
    );

    let stream = &manifest.streams[0];
    assert_eq!(
        stream.stream_type,
        transmux::smooth_parse::StreamType::Video
    );
    assert!(
        !stream.qualities.is_empty(),
        "must have at least one QualityLevel"
    );
    assert!(
        stream.chunks.unwrap_or(0) > 0,
        "must have at least one chunk"
    );

    println!(
        "manifest parse OK: {} streams, chunks: {:?}",
        manifest.streams.len(),
        manifest.streams[0].chunks
    );
}

/// Test 2: Advertised == servable — parse the manifest, extract the fragment
/// URLs it actually advertises, request each one, assert real bytes come back.
#[tokio::test]
async fn advertised_equals_servable() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    let mut batches = Vec::new();
    for i in 0..60u32 {
        let is_sync = i % 30 == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    let app = build_app(store.clone(), Arc::new(SmoothOutput));

    // Get manifest
    let resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK);
    let manifest_xml = body_string(resp).await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("manifest must parse");

    // Derive fragment URLs from the manifest
    for stream in &manifest.streams {
        let chunks = stream
            .enumerate_chunks()
            .unwrap_or_else(|e| panic!("enumerate_chunks failed: {e:?}"));
        assert!(
            !chunks.is_empty(),
            "manifest must advertise at least one chunk"
        );

        // Use the first quality level's bitrate
        let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);

        for (start_time, _duration) in &chunks {
            let fragment_url = stream.resolve_fragment_url(bitrate, *start_time);
            let uri = format!("/cam/{fragment_url}");

            let frag_resp = app
                .clone()
                .oneshot(get(&uri))
                .await
                .unwrap_or_else(|e| panic!("fragment request failed for {uri}: {e}"));

            assert_eq!(
                frag_resp.status(),
                StatusCode::OK,
                "fragment {uri} must return 200 OK"
            );

            let fragment_bytes = body_bytes(frag_resp).await;
            assert!(
                !fragment_bytes.is_empty(),
                "fragment {uri} must return non-empty bytes"
            );
        }
    }
}

/// Test 3: Auth is enforced — unauthenticated requests to manifest and
/// fragments are rejected.
#[tokio::test]
async fn auth_is_enforced() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    let mut batches = Vec::new();
    for i in 0..30u32 {
        let sample = Sample::new(
            vec![0xAA; 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            i % 30 == 0,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    // Build an app with output auth (Basic, user:pass)
    let verifier = Arc::new(broadcast_auth::Verifier::new(
        Credentials::Basic {
            username: "viewer".into(),
            password: "secret".into(),
        },
        "smooth-test",
    ));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(SmoothOutput) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams).with_output_auth(verifier)));

    // Manifest: no auth -> 401
    let resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("router call");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "manifest without auth must return 401"
    );

    // Fragment: no auth -> 401 (need a valid fragment URL first from an authed manifest)
    // First, get a manifest with auth to find a fragment URL
    let auth_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/cam/Manifest")
                .header("Authorization", "Basic dmlld2VyOnNlY3JldA==") // viewer:secret
                .body(Body::empty())
                .expect("auth request"),
        )
        .await
        .expect("authed manifest call");
    assert_eq!(auth_resp.status(), StatusCode::OK);
    let manifest_xml = body_string(auth_resp).await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("parse manifest");

    let stream = &manifest.streams[0];
    let chunks = stream.enumerate_chunks().expect("enumerate_chunks");
    let (start_time, _) = chunks[0];
    let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);
    let fragment_url = stream.resolve_fragment_url(bitrate, start_time);
    let fragment_uri = format!("/cam/{fragment_url}");

    // Fragment without auth -> 401
    let frag_resp = app
        .clone()
        .oneshot(get(&fragment_uri))
        .await
        .expect("fragment call");
    assert_eq!(
        frag_resp.status(),
        StatusCode::UNAUTHORIZED,
        "fragment without auth must return 401"
    );
}

/// Test 4: A route with `["llhls","smooth"]` serves both from one ingest,
/// proving they share the Trunk.
#[tokio::test]
async fn llhls_and_smooth_serve_from_same_trunk() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    let mut batches = Vec::new();
    for i in 0..60u32 {
        let is_sync = i % 30 == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![
                Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
                Arc::new(SmoothOutput) as Arc<dyn Output>,
            ],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // LL-HLS media playlist works
    let hls_resp = app
        .clone()
        .oneshot(get("/cam/media.m3u8"))
        .await
        .expect("hls router call");
    assert_eq!(hls_resp.status(), StatusCode::OK);
    let playlist = body_string(hls_resp).await;
    assert!(playlist.contains("#EXTM3U"), "must be valid HLS playlist");

    // Smooth manifest works
    let smooth_resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("smooth router call");
    assert_eq!(smooth_resp.status(), StatusCode::OK);
    let manifest_xml = body_string(smooth_resp).await;
    assert!(
        manifest_xml.contains("<SmoothStreamingMedia"),
        "must be valid Smooth manifest"
    );

    // Smooth fragment works
    let manifest = SmoothManifest::parse(&manifest_xml).expect("parse manifest");
    let stream = &manifest.streams[0];
    let chunks = stream.enumerate_chunks().expect("enumerate_chunks");
    let (start_time, _) = chunks[0];
    let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);
    let fragment_url = stream.resolve_fragment_url(bitrate, start_time);
    let fragment_uri = format!("/cam/{fragment_url}");

    let frag_resp = app
        .clone()
        .oneshot(get(&fragment_uri))
        .await
        .expect("fragment call");
    assert_eq!(frag_resp.status(), StatusCode::OK);
    let frag_bytes = body_bytes(frag_resp).await;
    assert!(!frag_bytes.is_empty(), "fragment bytes non-empty");
}

/// Test 5: Config round-trip — `"smooth"` deserialises and validates.
#[test]
fn config_smooth_deserializes_and_validates() {
    let cfg: multimux::config::Config = serde_json::from_str(
        r#"{
            "bind": "0.0.0.0:8080",
            "routes": [
                {
                    "name": "test",
                    "input": {"type": "rtsp", "url": "rtsp://example.com/stream"},
                    "outputs": ["smooth"]
                }
            ]
        }"#,
    )
    .expect("deserialize smooth config");
    assert_eq!(cfg.routes.len(), 1);
    assert_eq!(cfg.routes[0].outputs.len(), 1, "one output configured");
    assert_eq!(
        cfg.routes[0].outputs[0].name(),
        "smooth",
        "output kind must be smooth"
    );
    cfg.validate().expect("smooth config must validate");
}
