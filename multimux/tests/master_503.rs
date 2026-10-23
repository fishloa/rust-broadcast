//! Master playlist 503/Retry-After gate (issue #1426 part A).

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::output::ts_hls::TsHlsOutput;
use multimux::route::RouteHandle;

fn llhls_route_declared(declared: u64) -> Arc<RouteHandle> {
    let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_declared_bandwidth_bps(declared));
    route.publish_new_program(media_plane::ProgramId(0));
    route
}

fn llhls_route_undeclared_unpublished() -> Arc<RouteHandle> {
    Arc::new(RouteHandle::new(4.0, 500, 4))
}

fn llhls_route_with_segment() -> Arc<RouteHandle> {
    let route = Arc::new(RouteHandle::new(4.0, 500, 4));
    route.publish_new_program(media_plane::ProgramId(0));
    route
        .add_segment(
            media_plane::ProgramId(0),
            transmux::ll_hls::SegmentInfo {
                bytes: vec![0x21; 8],
                duration: 4.0,
                segment_seq: 1,
                part_count: 0,
            },
        )
        .expect("add_segment");
    route
}

fn ts_route_undeclared_unpublished() -> Arc<RouteHandle> {
    use hls_runtime::server::Container;
    Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs))
}

fn app_for(route: Arc<RouteHandle>, output: Arc<dyn Output>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert("cam".to_string(), (route, vec![output]));
    router(Arc::new(AppState::new(streams)))
}

#[tokio::test]
async fn master_503_with_retry_after_before_first_segment() {
    for output in [
        Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
        Arc::new(TsHlsOutput::new("media.m3u8")) as Arc<dyn Output>,
    ] {
        let route = if output.kind() == multimux::output::OutputKind::TsHls {
            ts_route_undeclared_unpublished()
        } else {
            llhls_route_undeclared_unpublished()
        };
        let app = app_for(route, output);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/cam/master.m3u8")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers()
                .get(header::RETRY_AFTER)
                .unwrap()
                .to_str()
                .unwrap(),
            "1"
        );
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains("BANDWIDTH"));
    }
}

#[tokio::test]
async fn master_200_with_declared_bandwidth_before_first_segment() {
    let route = llhls_route_declared(9876);
    let app = app_for(route, Arc::new(LlHlsOutput::default()));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/master.m3u8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("BANDWIDTH=9876"));
}

#[tokio::test]
async fn master_200_after_first_segment_uses_measured() {
    let route = llhls_route_with_segment();
    let app = app_for(route, Arc::new(LlHlsOutput::default()));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/cam/master.m3u8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("BANDWIDTH="));
    // 8 bytes over 4 s = 16 b/s.
    assert!(text.contains("BANDWIDTH=16"));
}
