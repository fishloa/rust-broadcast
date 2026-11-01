//! Defect 4 (rtmp): no awaited IO is unbounded; cancel-safety of `next_events`
//! (the reason `pending_write` existed). Paused virtual time over `duplex` pipes.
#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::time::Duration;

use rtmp_runtime::client::{ClientConfig, ClientSession};
use rtmp_runtime::io::{RtmpConnection, RtmpTimeouts};
use rtmp_runtime::server::ServerSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

fn conn(io: tokio::io::DuplexStream, t: RtmpTimeouts) -> RtmpConnection<tokio::io::DuplexStream> {
    RtmpConnection::from_stream(io, ServerSession::with_defaults(), t)
}

#[tokio::test(start_paused = true)]
async fn an_idle_peer_times_out_at_read_idle() {
    let (_client, server) = duplex(4096);
    let mut c = conn(
        server,
        RtmpTimeouts::default()
            .with_handshake(Duration::from_secs(300))
            .with_read_idle(Duration::from_secs(5)),
    );
    let t0 = tokio::time::Instant::now();
    let err = c.next_events().await.expect_err("must time out");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(5));
    assert!(
        c.next_events().await.unwrap().is_none(),
        "connection is closed after a timeout"
    );
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_never_completes_connect_times_out_at_handshake() {
    let (mut client, server) = duplex(4096);
    // Client sends C0+C1 and then goes quiet: handshake bytes arrive, `connect` never does.
    let mut cs = ClientSession::new(ClientConfig::default());
    client.write_all(&cs.start()).await.unwrap();
    let mut c = conn(
        server,
        RtmpTimeouts::default()
            .with_handshake(Duration::from_secs(3))
            .with_read_idle(Duration::from_secs(60)),
    );
    let t0 = tokio::time::Instant::now();
    let first = c.next_events().await.unwrap().expect("handshake batch");
    assert!(first.is_empty());
    let err = c.next_events().await.expect_err("handshake deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(3));
    drop(client);
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_times_out_the_write() {
    let (mut client, server) = duplex(2048); // reply (3073 B) cannot fit
    let mut cs = ClientSession::new(ClientConfig::default());
    client.write_all(&cs.start()).await.unwrap(); // 1537 B fits
    let mut c = conn(
        server,
        RtmpTimeouts::default().with_write(Duration::from_secs(4)),
    );
    let err = c.next_events().await.expect_err("write deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
}

/// The reason `pending_write` used to exist, now covered by `Framed`'s own buffer:
/// cancel `next_events` while its reply is only partly written, then call it again.
/// The peer must receive the reply exactly once, byte-identical to an uncancelled run.
#[tokio::test]
async fn a_cancelled_next_events_neither_loses_nor_duplicates_the_reply() {
    let c0c1 = ClientSession::new(ClientConfig::default()).start();
    let expected = {
        let mut s = ServerSession::with_defaults();
        s.handle_data(&c0c1).unwrap().0
    };
    assert_eq!(expected.len(), 3073);

    let (mut client, server) = duplex(2048);
    client.write_all(&c0c1).await.unwrap();
    let mut c = conn(server, RtmpTimeouts::default());

    // First call: decodes C0+C1, starts writing the 3073-byte reply into a 2048-byte pipe, blocks.
    // `biased` + an always-ready arm cancels the call right after its first suspension.
    tokio::select! {
        biased;
        r = c.next_events() => panic!("must not finish: {r:?}"),
        _ = std::future::ready(()) => {}
    }

    // Drain the client side concurrently while the second call finishes the flush.
    let reader = tokio::spawn(async move {
        let mut got = vec![0u8; 3073];
        client.read_exact(&mut got).await.unwrap();
        // The server side is dropped below, so EOF follows: anything else that arrives is a duplicate.
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        (got, rest)
    });
    let events = c
        .next_events()
        .await
        .unwrap()
        .expect("the events of the cancelled call are not lost");
    assert!(events.is_empty());
    drop(c); // closes the server side: the reader sees EOF right after the reply
    let (got, rest) = reader.await.unwrap();
    assert_eq!(got, expected, "reply must be byte-identical and complete");
    assert!(
        rest.is_empty(),
        "reply must not be duplicated: {} extra bytes",
        rest.len()
    );
}

/// `read_idle` bounds the whole wait, not each chunk: after the handshake reply, the peer drips the
/// 1536-byte C2 one byte every 2 s (every chunk yields an empty batch). The idle deadline must not
/// restart per chunk, so the connection times out at 5 s, long before the 300 s handshake bound.
#[tokio::test(start_paused = true)]
async fn a_peer_dripping_empty_chunks_is_bounded_by_read_idle_across_batches() {
    let c0c1 = ClientSession::new(ClientConfig::default()).start();
    let (reply, c2) = {
        let mut s = ServerSession::with_defaults();
        let reply = s.handle_data(&c0c1).unwrap().0;
        let mut cs = ClientSession::new(ClientConfig::default());
        let _ = cs.start();
        (reply.clone(), cs.handle_data(&reply).unwrap().0)
    };
    assert!(c2.len() >= 1536, "C2 is {} bytes", c2.len());
    let (mut client, server) = duplex(64 * 1024);
    client.write_all(&c0c1).await.unwrap();
    tokio::spawn(async move {
        for b in c2 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if client.write_all(&[b]).await.is_err() {
                return;
            }
        }
    });
    let mut c = conn(
        server,
        RtmpTimeouts::default()
            .with_handshake(Duration::from_secs(300))
            .with_read_idle(Duration::from_secs(5)),
    );
    let t0 = tokio::time::Instant::now();
    let mut calls = 0;
    let err = loop {
        calls += 1;
        match c.next_events().await {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("closed without a timeout"),
            Err(e) => break e,
        }
    };
    let _ = reply;
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert!(calls >= 2, "several empty batches were returned ({calls})");
    assert_eq!(t0.elapsed(), Duration::from_secs(5));
}
