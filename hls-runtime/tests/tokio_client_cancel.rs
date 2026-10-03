#![cfg(feature = "tokio")]

use std::time::Duration;

use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// A token cancelled BEFORE the first call ends the stream at once: no request is made.
#[tokio::test]
async fn a_pre_cancelled_client_returns_end_of_stream_without_any_io() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut c = TokioClient::with_config(
        "http://127.0.0.1:1/never.m3u8",
        TokioClientConfig::default().with_cancel(cancel),
    )
    .unwrap();
    let out = tokio::time::timeout(Duration::from_secs(5), c.next_output())
        .await
        .expect("must not block")
        .unwrap();
    assert!(out.is_none());
}

/// A request that would block for ever (the origin accepts and says nothing) must be abandoned
/// as soon as the token is cancelled, not after `request_timeout`. The token is cancelled only
/// AFTER the origin has accepted the connection, i.e. while the request is genuinely in flight.
#[tokio::test]
async fn cancelling_aborts_an_in_flight_request() {
    let hole = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = hole.local_addr().unwrap().port();
    let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::unbounded_channel();
    let _keep = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = hole.accept().await {
            held.push(s); // accept, never answer
            let _ = accepted_tx.send(());
        }
    });
    let cancel = CancellationToken::new();
    let cfg = TokioClientConfig {
        request_timeout: Duration::from_secs(300),
        blocking_timeout: Duration::from_secs(300),
        ..TokioClientConfig::default().with_cancel(cancel.clone())
    };
    let mut c = TokioClient::with_config(format!("http://127.0.0.1:{port}/p.m3u8"), cfg).unwrap();
    let task = tokio::spawn(async move { c.next_output().await });
    tokio::time::timeout(Duration::from_secs(10), accepted_rx.recv())
        .await
        .expect("the client must connect")
        .expect("accept notification");
    cancel.cancel();
    let out = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("cancel must abort the request")
        .unwrap()
        .unwrap();
    assert!(out.is_none());
}

/// Cancelling while the client sleeps between playlist reloads (a backoff after a
/// failing origin) ends the sleep at once.
#[tokio::test]
async fn cancelling_aborts_a_retry_backoff_sleep() {
    // Nothing listens on this port: connect fails fast, so the client is in its backoff.
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let cancel = CancellationToken::new();
    let cfg = TokioClientConfig {
        retry_backoff: Duration::from_secs(300),
        max_retry_backoff: Duration::from_secs(300),
        ..TokioClientConfig::default()
            .with_jitter(false)
            .with_cancel(cancel.clone())
    };
    let mut c = TokioClient::with_config(format!("http://127.0.0.1:{port}/p.m3u8"), cfg).unwrap();
    let task = tokio::spawn(async move { c.next_output().await });
    // Let the first attempt fail and the backoff start: yield until the task
    // has had its chance to run, then cancel (bounded by the final timeout).
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    let out = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("cancel must end the backoff sleep")
        .unwrap()
        .unwrap();
    assert!(out.is_none());
}
