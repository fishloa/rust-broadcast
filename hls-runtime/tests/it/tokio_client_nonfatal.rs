//! Pre-release audit #1485 (items 2, 4, 14, 16, tokio adapter): a response the
//! sans-IO core rejects is a per-request failure that must not end the
//! session, and a response body is bounded by `max_body_bytes`.
#![cfg(feature = "tokio")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use hls_runtime::client::Output;
use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig, TokioError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use transmux::sample_aes::aes128_encrypt_segment;

const FIXTURE: &[u8] = include_bytes!("../fixtures/ts-hls/index0.ts");
const KEY: [u8; 16] = [0x61; 16];
const IV: [u8; 16] = [0x17; 16];
const PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x17171717171717171717171717171717\n\
#EXTINF:2.0,\nindex0.ts\n#EXT-X-ENDLIST\n";
const SESSION_DEADLINE: Duration = Duration::from_secs(30);
const FAST_RETRY: Duration = Duration::from_millis(5);

#[derive(Clone)]
struct Origin {
    key_hits: Arc<AtomicUsize>,
    segment_hits: Arc<AtomicUsize>,
    bad_key_first: bool,
    truncated_segment_first: bool,
}

async fn handle(State(o): State<Origin>, req: Request) -> axum::response::Response {
    match req.uri().path() {
        "/live/index.m3u8" => (StatusCode::OK, PLAYLIST).into_response(),
        "/live/key.bin" => {
            let n = o.key_hits.fetch_add(1, Ordering::SeqCst);
            if o.bad_key_first && n == 0 {
                (StatusCode::OK, "<html>sign in</html>").into_response()
            } else {
                (StatusCode::OK, KEY.to_vec()).into_response()
            }
        }
        "/live/index0.ts" => {
            let n = o.segment_hits.fetch_add(1, Ordering::SeqCst);
            let cipher = aes128_encrypt_segment(&KEY, &IV, FIXTURE);
            let body = if o.truncated_segment_first && n == 0 {
                cipher[..cipher.len() - 1].to_vec()
            } else {
                cipher
            };
            (StatusCode::OK, body).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn run_origin(origin: Origin) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new().fallback(handle).with_state(origin);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}/live/index.m3u8")
}

fn fast_config() -> TokioClientConfig {
    TokioClientConfig {
        retry_backoff: FAST_RETRY,
        max_retry_backoff: FAST_RETRY,
        jitter: false,
        ..TokioClientConfig::default()
    }
}

async fn drive_to_end(client: &mut TokioClient) -> Vec<Output> {
    tokio::time::timeout(SESSION_DEADLINE, async {
        let mut outputs = Vec::new();
        while let Some(out) = client.next_output().await.unwrap() {
            let end = matches!(out, Output::EndOfStream);
            outputs.push(out);
            if end {
                break;
            }
        }
        outputs
    })
    .await
    .expect("the session never finished")
}

fn has_samples(outputs: &[Output]) -> bool {
    outputs.iter().any(|o| matches!(o, Output::Samples { .. }))
}

#[tokio::test]
async fn an_html_page_served_as_the_key_does_not_kill_the_session() {
    let key_hits = Arc::new(AtomicUsize::new(0));
    let url = run_origin(Origin {
        key_hits: key_hits.clone(),
        segment_hits: Arc::default(),
        bad_key_first: true,
        truncated_segment_first: false,
    })
    .await;
    let mut client = TokioClient::with_config(url, fast_config()).unwrap();
    let outputs = drive_to_end(&mut client).await;
    assert!(has_samples(&outputs), "{outputs:?}");
    assert_eq!(key_hits.load(Ordering::SeqCst), 2);
    assert!(
        client.stats().rejected_responses >= 1,
        "{:?}",
        client.stats()
    );
}

#[tokio::test]
async fn a_corrupt_segment_is_fetched_again_instead_of_ending_the_session() {
    let segment_hits = Arc::new(AtomicUsize::new(0));
    let url = run_origin(Origin {
        key_hits: Arc::default(),
        segment_hits: segment_hits.clone(),
        bad_key_first: false,
        truncated_segment_first: true,
    })
    .await;
    let mut client = TokioClient::with_config(url, fast_config()).unwrap();
    let outputs = drive_to_end(&mut client).await;
    assert!(has_samples(&outputs), "{outputs:?}");
    assert_eq!(segment_hits.load(Ordering::SeqCst), 2);
}

const HUGE_BODY_BYTES: usize = 128 * 1024 * 1024;
const CHUNK_BYTES: usize = 1024 * 1024;
const BODY_CAP_BYTES: usize = 1024 * 1024;
const SENT_CEILING_BYTES: usize = 32 * 1024 * 1024;

async fn serve_one(mut sock: tokio::net::TcpStream, sent: Arc<Mutex<Vec<usize>>>) {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match sock.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let line = String::from_utf8_lossy(&request);
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    if path.ends_with("index.m3u8") {
        let body = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXTINF:2.0,\nindex0.ts\n#EXT-X-ENDLIST\n";
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(head.as_bytes()).await;
        return;
    }
    let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    if sock.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let chunk = vec![0u8; CHUNK_BYTES];
    let mut total = 0usize;
    while total < HUGE_BODY_BYTES {
        let frame_head = format!("{:x}\r\n", chunk.len());
        if sock.write_all(frame_head.as_bytes()).await.is_err()
            || sock.write_all(&chunk).await.is_err()
            || sock.write_all(b"\r\n").await.is_err()
        {
            break;
        }
        total += chunk.len();
    }
    let _ = sock.write_all(b"0\r\n\r\n").await;
    sent.lock().unwrap().push(total);
}

#[tokio::test]
async fn a_response_body_is_bounded_by_max_body_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let sent: Arc<Mutex<Vec<usize>>> = Arc::default();
    let sent_server = sent.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(serve_one(sock, sent_server.clone()));
        }
    });
    let config = TokioClientConfig {
        max_body_bytes: BODY_CAP_BYTES,
        request_timeout: Duration::from_secs(20),
        ..fast_config()
    };
    let mut client =
        TokioClient::with_config(format!("http://127.0.0.1:{port}/live/index.m3u8"), config)
            .unwrap();
    let result = tokio::time::timeout(SESSION_DEADLINE, async {
        loop {
            match client.next_output().await {
                Ok(Some(Output::EndOfStream)) | Ok(None) => return Ok(()),
                Ok(Some(_)) => {}
                Err(e) => return Err(e),
            }
        }
    })
    .await
    .expect("the session hung");
    assert!(
        matches!(result, Err(TokioError::Stalled)),
        "a body over the cap must never be delivered: {result:?}"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let sent = sent.lock().unwrap().clone();
    assert!(!sent.is_empty(), "the segment was never requested");
    assert!(
        sent.iter().all(|&n| n <= SENT_CEILING_BYTES),
        "the client kept reading past the cap: {sent:?}"
    );
}
