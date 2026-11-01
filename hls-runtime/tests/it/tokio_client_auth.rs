//! `TokioClient` attaches configured credentials only to the origin
//! (scheme + host + port) of the configured playlist URL. A playlist from
//! that origin naming a segment on a second host must not hand that host the
//! `Authorization` header.
//!
//! Two loopback axum servers (different ports, so different origins)
//! serve the committed `tests/fixtures/ts-hls/` fixture: the playlist and
//! `index0.ts` from the first, `index1.ts` from the second.
#![cfg(feature = "tokio")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use broadcast_auth::Credentials;
use hls_runtime::client::Output;
use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use tokio::net::TcpListener;

/// Every request a mock server received: its path, and whether it carried an
/// `Authorization` header.
type Log = Arc<Mutex<Vec<(String, bool)>>>;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ts-hls"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[derive(Clone)]
struct Site {
    routes: Arc<Vec<(String, u16, Vec<u8>)>>,
    log: Log,
}

async fn serve_one(State(s): State<Site>, req: Request) -> axum::response::Response {
    let path = req.uri().path().to_string();
    s.log.lock().unwrap().push((
        path.clone(),
        req.headers().contains_key(header::AUTHORIZATION),
    ));
    match s.routes.iter().find(|(p, _, _)| *p == path) {
        Some((_, status, body)) => {
            (StatusCode::from_u16(*status).unwrap(), body.clone()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Serves `routes` (path → (status, body)) forever, logging each request.
async fn serve(listener: TcpListener, routes: Vec<(String, u16, Vec<u8>)>, log: Log) {
    let app = Router::new().fallback(serve_one).with_state(Site {
        routes: Arc::new(routes),
        log,
    });
    axum::serve(listener, app).await.unwrap();
}

async fn run_with(auth: Credentials) -> (Vec<(String, bool)>, Vec<(String, bool)>) {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_port = origin.local_addr().unwrap().port();
    let other_port = other.local_addr().unwrap().port();

    let playlist = String::from_utf8(fixture("index.m3u8")).unwrap().replace(
        "\nindex1.ts",
        &format!("\nhttp://127.0.0.1:{other_port}/cdn/index1.ts"),
    );
    assert!(
        playlist.contains("/cdn/index1.ts"),
        "fixture rewrite failed"
    );

    let (origin_log, other_log) = (Log::default(), Log::default());
    tokio::spawn(serve(
        origin,
        vec![
            ("/live/index.m3u8".into(), 200, playlist.into_bytes()),
            ("/live/index0.ts".into(), 200, fixture("index0.ts")),
        ],
        origin_log.clone(),
    ));
    tokio::spawn(serve(
        other,
        vec![("/cdn/index1.ts".into(), 200, fixture("index1.ts"))],
        other_log.clone(),
    ));

    let config = TokioClientConfig {
        auth: Some(auth),
        ..TokioClientConfig::default()
    };
    let mut client = TokioClient::with_config(
        format!("http://127.0.0.1:{origin_port}/live/index.m3u8"),
        config,
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
    .expect("client did not reach EndOfStream");

    let origin_log = origin_log.lock().unwrap().clone();
    let other_log = other_log.lock().unwrap().clone();
    (origin_log, other_log)
}

async fn assert_credentials_stay_on_origin(auth: Credentials) {
    let (origin_log, other_log) = run_with(auth).await;
    assert!(
        origin_log.iter().any(|(p, _)| p == "/live/index0.ts"),
        "origin never saw its segment: {origin_log:?}"
    );
    assert!(
        origin_log.iter().all(|(_, auth)| *auth),
        "same-origin requests must carry the credentials: {origin_log:?}"
    );
    assert!(
        !other_log.is_empty(),
        "second host never saw its segment request"
    );
    assert!(
        other_log.iter().all(|(_, auth)| !*auth),
        "second host received the Authorization header: {other_log:?}"
    );
}

#[tokio::test]
async fn basic_credentials_are_not_sent_to_a_second_host() {
    assert_credentials_stay_on_origin(Credentials::Basic {
        username: "admin".into(),
        password: "origin-only".into(),
    })
    .await;
}

#[tokio::test]
async fn bearer_token_is_not_sent_to_a_second_host() {
    assert_credentials_stay_on_origin(Credentials::bearer("origin-only-token")).await;
}
