//! `TokioClient` attaches configured credentials only to the origin
//! (scheme + host + port) of the configured playlist URL. A playlist from
//! that origin naming a segment on a second host must not hand that host the
//! `Authorization` header.
//!
//! Two loopback HTTP/1.1 servers (different ports, so different origins)
//! serve the committed `tests/fixtures/ts-hls/` fixture: the playlist and
//! `index0.ts` from the first, `index1.ts` from the second.
#![cfg(feature = "tokio")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use broadcast_auth::Credentials;
use hls_runtime::client::Output;
use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Every request head a mock server received.
type Log = Arc<Mutex<Vec<String>>>;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ts-hls"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Serves `routes` (path → (status, body)) forever, one request per
/// connection, logging each request head.
async fn serve(listener: TcpListener, routes: Vec<(String, u16, Vec<u8>)>, log: Log) {
    let routes = Arc::new(routes);
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let (routes, log) = (routes.clone(), log.clone());
        tokio::spawn(async move {
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            log.lock().unwrap().push(head);
            let (status, body) = routes
                .iter()
                .find(|(p, _, _)| *p == path)
                .map(|(_, s, b)| (*s, b.clone()))
                .unwrap_or((404, Vec::new()));
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.shutdown().await;
        });
    }
}

fn has_authorization(head: &str) -> bool {
    head.lines()
        .any(|l| l.to_ascii_lowercase().starts_with("authorization:"))
}

async fn run_with(auth: Credentials) -> (Vec<String>, Vec<String>) {
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
        origin_log.iter().any(|h| h.contains("/live/index0.ts")),
        "origin never saw its segment: {origin_log:?}"
    );
    assert!(
        origin_log.iter().all(|h| has_authorization(h)),
        "same-origin requests must carry the credentials: {origin_log:?}"
    );
    assert!(
        !other_log.is_empty(),
        "second host never saw its segment request"
    );
    assert!(
        other_log.iter().all(|h| !has_authorization(h)),
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
