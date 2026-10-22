//! End-to-end content-type gate for the shared origin resource route
//! (Apple HLS authoring spec 10.4): init segments are `video/mp4`,
//! fragmented-MP4 segments/parts are `video/iso.segment`, MPEG-2 TS
//! segments are `video/mp2t`. Range (206) responses carry the same type.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use transmux::ll_hls::PartInfo;

fn fmp4_route() -> Arc<RouteHandle> {
    let route = Arc::new(RouteHandle::new(4.0, 500, 4));
    route.publish_new_program(media_plane::ProgramId(0));
    route.set_init(media_plane::ProgramId(0), vec![0xAA; 8]);
    // closed segment 1
    route
        .add_segment(
            media_plane::ProgramId(0),
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x21; 8],
                duration: 4.0,
                segment_seq: 1,
                part_count: 2,
            },
        )
        .expect("add_segment");
    route.add_part(
        media_plane::ProgramId(0),
        PartInfo {
            bytes: vec![0x10; 4],
            duration: 0.5,
            independent: true,
            segment_seq: 2,
            part_index: 0,
        },
    );
    route
}

fn ts_route() -> Arc<RouteHandle> {
    use hls_runtime::server::Container;
    let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs));
    route.publish_new_program(media_plane::ProgramId(0));
    route
        .add_segment(
            media_plane::ProgramId(0),
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x47; 188],
                duration: 4.0,
                segment_seq: 1,
                part_count: 0,
            },
        )
        .expect("add_segment");
    route
}

fn app_for(route: Arc<RouteHandle>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            route,
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    router(Arc::new(AppState::new(streams)))
}

fn ct(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn init_is_video_mp4() {
    let app = app_for(fmp4_route());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/init-1.mp4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ct(&resp), "video/mp4");
}

#[tokio::test]
async fn segment_is_video_iso_segment() {
    let app = app_for(fmp4_route());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/seg-1-1.m4s")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ct(&resp), "video/iso.segment");
}

#[tokio::test]
async fn part_is_video_iso_segment() {
    let app = app_for(fmp4_route());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/part-1-2.0.m4s")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ct(&resp), "video/iso.segment");
}

#[tokio::test]
async fn ts_segment_is_video_mp2t() {
    let app = app_for(ts_route());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/seg-1-1.ts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(ct(&resp), "video/mp2t");
}

#[tokio::test]
async fn range_206_carries_same_type() {
    for (uri, expected) in [
        ("/cam/seg-1-1.m4s", "video/iso.segment"),
        ("/cam/init-1.mp4", "video/mp4"),
        ("/cam/part-1-2.0.m4s", "video/iso.segment"),
    ] {
        let app = app_for(fmp4_route());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("range", "bytes=0-3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT, "{uri}");
        assert_eq!(ct(&resp), expected, "{uri}");
    }
    // TS
    let app = app_for(ts_route());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/seg-1-1.ts")
                .header("range", "bytes=0-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(ct(&resp), "video/mp2t");
}
