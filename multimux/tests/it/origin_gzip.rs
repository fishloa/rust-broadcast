//! Real-HTTP gzip compression of HLS playlists (#1424): only
//! `application/vnd.apple.mpegurl` (and `application/dash+xml`) responses get
//! `Content-Encoding: gzip`; segments/init/parts stay byte-exact.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use flate2::read::GzDecoder;
use tower::ServiceExt;

use media_plane::ProgramId;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::dash::DashOutput;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use transmux::TrackId;
use transmux::ll_hls::SegmentInfo;
use transmux::{CodecConfig, TrackSpec};

fn make_app() -> (axum::Router, Arc<RouteHandle>) {
    let store = Arc::new(RouteHandle::new(4.0, 500, 4));
    store.publish_new_program(ProgramId(0));
    store.set_init(ProgramId(0), vec![0xAA; 4]);
    store.set_track_specs(
        ProgramId(0),
        vec![TrackSpec::new(
            TrackId::new(1),
            90_000,
            CodecConfig::Vp8 {
                width: 640,
                height: 480,
            },
        )],
    );
    store
        .add_segment(
            ProgramId(0),
            SegmentInfo {
                bytes: vec![0x33; 16],
                duration: 4.0,
                segment_seq: 1,
                part_count: 1,
            },
        )
        .expect("add_segment");

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

async fn get_bytes(
    app: axum::Router,
    uri: &str,
    accept: Option<&str>,
    range: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().uri(uri);
    if let Some(a) = accept {
        builder = builder.header("accept-encoding", a);
    }
    if let Some(r) = range {
        builder = builder.header("range", r);
    }
    let req = builder.body(Body::empty()).expect("request");
    let resp = app.oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    (status, headers, bytes)
}

fn vary_contains(headers: &axum::http::HeaderMap, needle: &str) -> bool {
    headers.get_all(axum::http::header::VARY).iter().any(|v| {
        v.to_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    })
}

fn decompress_gzip(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).expect("gzip decode");
    out
}

#[tokio::test]
async fn playlist_gzip_roundtrip() {
    let (app, _store) = make_app();
    let uri = "/cam1/media.m3u8";

    let (status, id_headers, id_body) = get_bytes(app.clone(), uri, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(id_headers.get("content-encoding").is_none());

    let (status, gz_headers, gz_body) = get_bytes(app.clone(), uri, Some("gzip"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        gz_headers
            .get("content-encoding")
            .unwrap()
            .to_str()
            .unwrap(),
        "gzip"
    );
    assert!(vary_contains(&gz_headers, "Origin"));
    assert!(vary_contains(&gz_headers, "Accept-Encoding"));
    assert_eq!(decompress_gzip(&gz_body), id_body);
}

#[tokio::test]
async fn playlist_no_accept_encoding_identity() {
    let (app, _store) = make_app();
    let (status, headers, _body) = get_bytes(app, "/cam1/media.m3u8", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("content-encoding").is_none());
}

#[tokio::test]
async fn playlist_gzip_q0_identity() {
    let (app, _store) = make_app();
    let uri = "/cam1/media.m3u8";
    let (_s, _h, id_body) = get_bytes(app.clone(), uri, None, None).await;
    let (status, headers, body) = get_bytes(app, uri, Some("gzip;q=0"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("content-encoding").is_none());
    assert_eq!(body, id_body);
}

#[tokio::test]
async fn segment_with_gzip_accepted_no_encoding_and_range_exact() {
    let (app, _store) = make_app();
    let uri = "/cam1/seg-1-1.m4s";

    let (status, id_headers, id_body) = get_bytes(app.clone(), uri, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(id_headers.get("content-encoding").is_none());

    let (status, gz_headers, gz_body) = get_bytes(app.clone(), uri, Some("gzip"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(gz_headers.get("content-encoding").is_none());
    assert_eq!(gz_body, id_body);

    // Range request stays byte-exact and 206.
    let req = Request::builder()
        .uri(uri)
        .header("range", "bytes=0-3")
        .header("accept-encoding", "gzip")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert!(resp.headers().get("content-encoding").is_none());
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    assert_eq!(body, vec![0x33u8; 4]);
}

#[tokio::test]
async fn init_segment_unchanged_with_gzip() {
    let (app, _store) = make_app();
    let (_s, _h, playlist_body) = get_bytes(app.clone(), "/cam1/media.m3u8", None, None).await;
    let playlist = String::from_utf8(playlist_body).expect("playlist utf8");
    let start = playlist.find("init-").expect("init uri in playlist");
    let rest = &playlist[start..];
    let end = rest.find(".mp4").expect("mp4") + ".mp4".len();
    let init_uri = format!("/cam1/{}", &rest[..end]);

    let (status, id_headers, id_body) = get_bytes(app.clone(), &init_uri, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(id_headers.get("content-encoding").is_none());

    let (status, gz_headers, gz_body) = get_bytes(app, &init_uri, Some("gzip"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(gz_headers.get("content-encoding").is_none());
    assert_eq!(gz_body, id_body);
}

#[tokio::test]
async fn master_and_dash_playlist_gzip() {
    let (app, _store) = make_app();

    for uri in ["/cam1/master.m3u8", "/cam1/media.m3u8"] {
        let (status, headers, body) = get_bytes(app.clone(), uri, Some("gzip"), None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            headers.get("content-encoding").unwrap().to_str().unwrap(),
            "gzip",
            "{uri}"
        );
        // identity comparison not required; just ensure decompresses to non-empty
        assert!(!decompress_gzip(&body).is_empty(), "{uri}");
    }

    // DASH MPD
    let (status, headers, body) =
        get_bytes(app.clone(), "/cam1/manifest.mpd", Some("gzip"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-encoding").unwrap().to_str().unwrap(),
        "gzip"
    );
    assert!(!decompress_gzip(&body).is_empty());
}

#[tokio::test]
async fn unknown_encoding_identity() {
    let (app, _store) = make_app();
    let uri = "/cam1/media.m3u8";
    let (_s, _h, id_body) = get_bytes(app.clone(), uri, None, None).await;
    let (status, headers, body) = get_bytes(app, uri, Some("br"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("content-encoding").is_none());
    assert_eq!(body, id_body);
}
