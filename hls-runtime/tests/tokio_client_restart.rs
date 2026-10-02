//! Audit r09-C3 / issue #1031: a blocking playlist reload that the origin
//! rejects with `400` (it restarted below the requested Media Sequence
//! Number) must be retried once as a plain GET, not repeated forever.
//!
//! One loopback HTTP/1.1 origin on an OS-assigned port: the plain playlist is
//! live with `CAN-BLOCK-RELOAD=YES` the first time and `ENDLIST` the second;
//! any request carrying `_HLS_msn` is answered `400`.
#![cfg(feature = "tokio")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hls_runtime::client::Output;
use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LIVE: &str = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nindex0.ts\n";
const ENDED: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nindex0.ts\n#EXT-X-ENDLIST\n";

#[tokio::test]
async fn a_rejected_blocking_reload_is_retried_once_as_a_plain_get() {
    let segment = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ts-hls/index0.ts"
    ))
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let plain_playlists = Arc::new(AtomicUsize::new(0));
    {
        let (log, plain_playlists) = (log.clone(), plain_playlists.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (log, plain_playlists, segment) =
                    (log.clone(), plain_playlists.clone(), segment.clone());
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
                    log.lock().unwrap().push(path.clone());
                    let (status, body): (u16, Vec<u8>) = if path.contains("_HLS_msn") {
                        (400, Vec::new())
                    } else if path == "/live/index.m3u8" {
                        let n = plain_playlists.fetch_add(1, Ordering::SeqCst);
                        (200, if n == 0 { LIVE } else { ENDED }.as_bytes().to_vec())
                    } else if path == "/live/index0.ts" {
                        (200, segment)
                    } else {
                        (404, Vec::new())
                    };
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
    }

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
