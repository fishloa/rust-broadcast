//! Audit r09-C3 / issue #1031: a blocking playlist reload that the origin
//! rejects with `400` (it restarted below the requested Media Sequence
//! Number) must be retried once as a plain GET, not repeated forever.
//!
//! One loopback axum origin on an OS-assigned port: the plain playlist is
//! live with `CAN-BLOCK-RELOAD=YES` the first time and `ENDLIST` the second;
//! any request carrying `_HLS_msn` is answered `400`.
#![cfg(feature = "tokio")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use hls_runtime::client::Output;
use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use tokio::net::TcpListener;

const LIVE: &str = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nindex0.ts\n";
const ENDED: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nindex0.ts\n#EXT-X-ENDLIST\n";

#[derive(Clone)]
struct Origin {
    log: Arc<Mutex<Vec<String>>>,
    plain_playlists: Arc<AtomicUsize>,
    segment: Arc<Vec<u8>>,
}

async fn handle(State(o): State<Origin>, req: Request) -> axum::response::Response {
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    o.log.lock().unwrap().push(path.clone());
    if path.contains("_HLS_msn") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match path.as_str() {
        "/live/index.m3u8" => {
            let n = o.plain_playlists.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, if n == 0 { LIVE } else { ENDED }).into_response()
        }
        "/live/index0.ts" => (StatusCode::OK, o.segment.to_vec()).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::test]
async fn a_rejected_blocking_reload_is_retried_once_as_a_plain_get() {
    let segment = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ts-hls/index0.ts"
    ))
    .unwrap();
    // Bound to port 0 first and handed to axum: no reserve-then-rebind.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let plain_playlists = Arc::new(AtomicUsize::new(0));
    let origin = Origin {
        log: log.clone(),
        plain_playlists: plain_playlists.clone(),
        segment: Arc::new(segment),
    };
    let app = Router::new().fallback(handle).with_state(origin);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut client = TokioClient::with_config(
        format!("http://127.0.0.1:{port}/live/index.m3u8"),
        TokioClientConfig::default(),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(out) = client.next_output().await.unwrap() {
            if matches!(out, Output::EndOfStream) {
                break;
            }
        }
    })
    .await
    .expect("client never recovered from the rejected blocking reload");

    let log = log.lock().unwrap().clone();
    let blocking = log.iter().filter(|p| p.contains("_HLS_msn")).count();
    assert_eq!(blocking, 1, "the rejected reload was repeated: {log:?}");
    assert_eq!(plain_playlists.load(Ordering::SeqCst), 2, "{log:?}");
}
