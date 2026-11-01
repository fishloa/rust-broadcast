//! Router-level coverage for `_HLS_skip` delivery directives (#blskip).

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use media_plane::ProgramId;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::dash::DashOutput;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use transmux::ll_hls::SegmentInfo;
use transmux::{CodecConfig, TrackSpec};

fn make_app() -> (axum::Router, Arc<RouteHandle>) {
    let store = Arc::new(RouteHandle::new(4.0, 500, 16));
    store.publish_new_program(ProgramId(0));
    store.set_init(ProgramId(0), vec![0xAA; 4]);
    store.set_track_specs(
        ProgramId(0),
        vec![TrackSpec::new(
            transmux::TrackId::new(1),
            90_000,
            CodecConfig::Vp8 {
                width: 640,
                height: 480,
            },
        )],
    );
    for seq in 1..=8u32 {
        store
            .add_segment(
                ProgramId(0),
                SegmentInfo {
                    bytes: vec![0x33; 16],
                    duration: 4.0,
                    segment_seq: seq,
                    part_count: 1,
                },
            )
            .expect("add_segment");
    }

    let mut streams = HashMap::new();
    streams.insert(
        "cam1".to_string(),
        (
            store.clone(),
            vec![
                Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
                Arc::new(DashOutput) as Arc<dyn Output>,
            ],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    (app, store)
}

async fn get_body(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn hls_skip_yes_returns_ext_x_skip() {
    let (app, _) = make_app();
    let (status, body) = get_body(app, "/cam1/media.m3u8?_HLS_skip=YES").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("#EXT-X-SKIP:"), "body: {body}");
}

#[tokio::test]
async fn hls_skip_bogus_returns_full_playlist() {
    let (app, _) = make_app();
    let (status, body) = get_body(app, "/cam1/media.m3u8?_HLS_skip=bogus").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("#EXT-X-SKIP:"), "body: {body}");
}

#[tokio::test]
async fn priority_header_playlist_and_segment() {
    let (app, _) = make_app();
    // playlist
    let req = Request::builder()
        .uri("/cam1/media.m3u8")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.headers().get("priority").and_then(|v| v.to_str().ok()),
        Some("u=1")
    );
    // segment
    let req = Request::builder()
        .uri("/cam1/seg-1-1.m4s")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.headers().get("priority").and_then(|v| v.to_str().ok()),
        Some("u=2")
    );
}
