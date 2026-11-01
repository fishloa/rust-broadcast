//! New client adapter (SP6.1) against `AsyncRtmpServer` over real loopback sockets,
//! and the bounded failure modes over `duplex` + paused time.
#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::time::Duration;

use rtmp_runtime::io::{AsyncRtmpClient, AsyncRtmpServer, RtmpTimeouts};
use rtmp_runtime::server::{ServerConfig, ServerEvent};
use rtmp_runtime::target::RtmpTarget;

async fn publish_roundtrip(bind: &str, url_host: &str) {
    let server = match AsyncRtmpServer::bind(bind, ServerConfig::default()).await {
        Ok(s) => s,
        // Only a host genuinely without IPv6 loopback may skip; anything else is a failure.
        Err(e) if e.kind() == ErrorKind::AddrNotAvailable || e.raw_os_error() == Some(97) => {
            eprintln!("skipping {bind}: {e}");
            return;
        }
        Err(e) => panic!("bind {bind}: {e}"),
    };
    let port = server.local_addr().unwrap().port();
    let accepted = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        let mut seen = Vec::new();
        while let Some(batch) = conn.next_events().await.unwrap() {
            seen.extend(batch);
            if seen.iter().any(|e| matches!(e, ServerEvent::Media { .. })) {
                break;
            }
        }
        seen
    });
    let target = RtmpTarget::parse(&format!("rtmp://{url_host}:{port}/live/testkey")).unwrap();
    let mut c = AsyncRtmpClient::connect(&target, RtmpTimeouts::default())
        .await
        .unwrap();
    c.publish().await.unwrap();
    c.send_video(0, &[0x17, 0x01, 0, 0, 0, 0xDE, 0xAD])
        .await
        .unwrap();
    let seen = accepted.await.unwrap();
    assert!(
        seen.iter()
            .any(|e| matches!(e, ServerEvent::Connected { app } if app == "live"))
    );
    assert!(
        seen.iter().any(
            |e| matches!(e, ServerEvent::Publish { stream_key, .. } if stream_key == "testkey")
        )
    );
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::Media { .. })));
}

#[tokio::test]
async fn publishes_over_ipv4_loopback() {
    publish_roundtrip("127.0.0.1:0", "127.0.0.1").await;
}

/// Defect 8 end to end: the bracketed IPv6 URL both resolves and connects.
#[tokio::test]
async fn publishes_over_ipv6_loopback() {
    publish_roundtrip("[::1]:0", "[::1]").await;
}

#[tokio::test(start_paused = true)]
async fn publish_to_a_peer_that_never_answers_times_out_at_handshake() {
    let (client_io, _server_io) = tokio::io::duplex(8192);
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let mut c = AsyncRtmpClient::from_stream(
        client_io,
        &target,
        RtmpTimeouts::default().with_handshake(Duration::from_secs(4)),
    );
    let t0 = tokio::time::Instant::now();
    let err = c.publish().await.expect_err("handshake deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(4));
}

#[tokio::test]
async fn a_server_that_rejects_publish_surfaces_connection_refused() {
    let server = AsyncRtmpServer::bind(
        "127.0.0.1:0",
        ServerConfig::default().with_expected_stream_key(Some("right".into())),
    )
    .await
    .unwrap();
    let port = server.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        while let Ok(Some(_)) = conn.next_events().await {}
    });
    let target = RtmpTarget::parse(&format!("rtmp://127.0.0.1:{port}/live/wrong")).unwrap();
    let mut c = AsyncRtmpClient::connect(&target, RtmpTimeouts::default())
        .await
        .unwrap();
    let err = c.publish().await.expect_err("BadName");
    assert_eq!(err.kind(), ErrorKind::ConnectionRefused, "{err}");
}

/// A client already publishing against a server that stops reading: `send_video` must hit the
/// write bound (review: the `queue` flush was unbounded and untested). Virtual-time hang guard so a
/// missing bound fails instead of hanging.
#[tokio::test(start_paused = true)]
async fn send_video_to_a_peer_that_stops_reading_times_out_the_write() {
    use rtmp_runtime::io::RtmpConnection;
    let (client_io, server_io) = tokio::io::duplex(4096);
    tokio::spawn(async move {
        let mut conn = RtmpConnection::from_stream(
            server_io,
            rtmp_runtime::server::ServerSession::with_defaults(),
            RtmpTimeouts::default(),
        );
        while let Ok(Some(batch)) = conn.next_events().await {
            if batch
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. }))
            {
                std::future::pending::<()>().await; // stop reading, keep the pipe open
            }
        }
    });
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let mut c = AsyncRtmpClient::from_stream(
        client_io,
        &target,
        RtmpTimeouts::default().with_write(Duration::from_secs(4)),
    );
    c.publish().await.unwrap();
    let t0 = tokio::time::Instant::now();
    let big = vec![0x42u8; 256 * 1024];
    let err = tokio::time::timeout(Duration::from_secs(60), c.send_video(0, &big))
        .await
        .expect("send_video must be bounded by `write`, not hang")
        .expect_err("peer is not reading");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(4));
}

/// The client's `next_events` honours `read_idle` once publishing (a silent server).
#[tokio::test(start_paused = true)]
async fn next_events_on_a_silent_server_times_out_at_read_idle() {
    use rtmp_runtime::io::RtmpConnection;
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let mut conn = RtmpConnection::from_stream(
            server_io,
            rtmp_runtime::server::ServerSession::with_defaults(),
            RtmpTimeouts::default(),
        );
        while let Ok(Some(batch)) = conn.next_events().await {
            if batch
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. }))
            {
                std::future::pending::<()>().await;
            }
        }
    });
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let mut c = AsyncRtmpClient::from_stream(
        client_io,
        &target,
        RtmpTimeouts::default().with_read_idle(Duration::from_secs(5)),
    );
    c.publish().await.unwrap();
    let t0 = tokio::time::Instant::now();
    let err = loop {
        match tokio::time::timeout(Duration::from_secs(60), c.next_events())
            .await
            .expect("bounded by read_idle")
        {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("closed"),
            Err(e) => break e,
        }
    };
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert!(t0.elapsed() <= Duration::from_secs(5));
}
