//! Defect 4: no awaited IO in the RTSP adapters is unbounded. All time is paused
//! virtual time over in-memory `duplex` pipes (never real sockets + paused time).
#![cfg(feature = "tokio")]

use std::time::Duration;

use rtsp_runtime::{AsyncRtspClient, ClientSession, Error, RtspTimeouts};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

const URI: &str = "rtsp://h/s";

fn short() -> RtspTimeouts {
    RtspTimeouts::default()
        .with_read_idle(Duration::from_secs(5))
        .with_write(Duration::from_secs(5))
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_times_out_the_request() {
    let (client_io, _server_io) = duplex(4096); // peer kept alive, never replies
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c.options(URI).await.expect_err("must time out");
    assert!(
        matches!(err, Error::Timeout { what: "read" }),
        "got {err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_times_out_the_write() {
    let (client_io, _server_io) = duplex(8); // 8-byte pipe, never drained
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c.options(URI).await.expect_err("must time out");
    assert!(
        matches!(err, Error::Timeout { what: "write" }),
        "got {err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_response_dripped_one_byte_at_a_time_still_times_out_as_a_whole() {
    let (client_io, mut server_io) = duplex(4096);
    // Server: swallow the request, then send a response byte every 2 s: never finishes in 5 s.
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        let _ = server_io.read(&mut buf).await;
        for b in b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n" {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if server_io.write_all(&[*b]).await.is_err() {
                break;
            }
        }
    });
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c
        .options(URI)
        .await
        .expect_err("frame deadline, not per-byte idle");
    assert!(
        matches!(err, Error::Timeout { what: "read" }),
        "got {err:?}"
    );
}

use rtsp_runtime::{AsyncRtspServer, ServerSession};

fn server_over(
    io: tokio::io::DuplexStream,
    t: RtspTimeouts,
) -> AsyncRtspServer<tokio::io::DuplexStream> {
    AsyncRtspServer::with_stream_timeouts(io, ServerSession::new(|| 7).with_session_seed(1), t)
}

#[tokio::test(start_paused = true)]
async fn an_idle_connection_that_never_sends_a_request_times_out_at_handshake() {
    let (_client_io, server_io) = duplex(4096);
    let mut s = server_over(
        server_io,
        RtspTimeouts::default().with_handshake(Duration::from_secs(4)),
    );
    let err = s.next_request().await.expect_err("handshake deadline");
    assert!(
        matches!(err, Error::Timeout { what: "read" }),
        "got {err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn slow_loris_request_dripped_a_byte_every_few_seconds_times_out() {
    let (mut client_io, server_io) = duplex(4096);
    tokio::spawn(async move {
        for b in b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\n" {
            tokio::time::sleep(Duration::from_secs(3)).await; // never idle for 10 s...
            if client_io.write_all(&[*b]).await.is_err() {
                return;
            }
        }
    });
    let mut s = server_over(
        server_io,
        RtspTimeouts::default().with_handshake(Duration::from_secs(10)),
    );
    let err = s
        .next_request()
        .await
        .expect_err("...but the whole request must finish in 10 s");
    assert!(
        matches!(err, Error::Timeout { what: "read" }),
        "got {err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_header_block_that_never_terminates_is_rejected_at_the_head_cap() {
    let (mut client_io, server_io) = duplex(256 * 1024);
    let mut s = server_over(server_io, RtspTimeouts::default());
    tokio::spawn(async move {
        let _ = client_io
            .write_all(b"OPTIONS rtsp://h/s RTSP/1.0\r\nX-Pad: ")
            .await;
        let junk = vec![b'a'; 80 * 1024];
        let _ = client_io.write_all(&junk).await;
        std::future::pending::<()>().await; // keep the pipe open: the cap, not EOF, must trip
    });
    let err = s.next_request().await.expect_err("64 KiB header cap");
    assert!(matches!(err, Error::MessageParse(_)), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_content_length_larger_than_the_message_cap_is_rejected_up_front() {
    let (mut client_io, server_io) = duplex(4096);
    let mut s = server_over(server_io, RtspTimeouts::default());
    tokio::spawn(async move {
        let _ = client_io
            .write_all(
                b"ANNOUNCE rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 9999999\r\n\r\n",
            )
            .await;
        std::future::pending::<()>().await;
    });
    let err = s
        .next_request()
        .await
        .expect_err("body larger than the 2 MiB cap");
    assert!(matches!(err, Error::MessageParse(_)), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn recv_interleaved_sends_a_keepalive_before_the_session_expires() {
    use rtsp_runtime::{ClientEvent, Transport, TransportSpec};
    let (client_io, server_io) = duplex(16 * 1024);
    let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel::<tokio::time::Instant>();
    let server = tokio::spawn(async move {
        let mut s = AsyncRtspServer::with_stream_timeouts(
            server_io,
            ServerSession::new(|| 9)
                .with_session_seed(9)
                .with_session_timeout(20),
            RtspTimeouts::default().with_read_idle(Duration::from_secs(120)),
        );
        let mut n = 0;
        while let Ok(Some(events)) = s.next_request().await {
            n += 1;
            if n == 3 {
                // SETUP, PLAY, then the keepalive GET_PARAMETER arrived.
                seen_tx.send(tokio::time::Instant::now()).unwrap();
                s.send_interleaved(0, b"frame").await.unwrap();
            }
            let _ = events;
        }
    });
    let t0 = tokio::time::Instant::now();
    let mut c = AsyncRtspClient::with_stream_timeouts(
        client_io,
        ClientSession::new(),
        RtspTimeouts::default().with_read_idle(Duration::from_secs(120)),
    );
    c.setup(
        URI,
        &Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1)),
    )
    .await
    .unwrap();
    c.play(URI).await.unwrap();
    let ev = c
        .recv_interleaved()
        .await
        .unwrap()
        .expect("media after keepalive");
    assert!(matches!(ev, ClientEvent::MediaData { channel: 0, .. }));
    let seen = seen_rx.recv().await.unwrap();
    let after = seen - t0;
    assert!(
        after >= Duration::from_secs(10) && after < Duration::from_secs(11),
        "keepalive at {after:?}"
    );
    drop(c);
    let _ = server.await;
}

/// A live interleaved stream must not postpone the keepalive: the server sends a frame every
/// second, yet a `GET_PARAMETER` still arrives at half the 20 s session timeout.
#[tokio::test(start_paused = true)]
async fn keepalive_is_sent_while_interleaved_media_flows() {
    use rtsp_runtime::{ClientEvent, Transport, TransportSpec};
    let (client_io, server_io) = duplex(64 * 1024);
    let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel::<tokio::time::Instant>();
    let server = tokio::spawn(async move {
        let mut s = AsyncRtspServer::with_stream_timeouts(
            server_io,
            ServerSession::new(|| 9)
                .with_session_seed(9)
                .with_session_timeout(20),
            RtspTimeouts::default().with_read_idle(Duration::from_secs(120)),
        );
        let (mut n, mut tick) = (0, tokio::time::interval(Duration::from_secs(1)));
        loop {
            tokio::select! {
                r = s.next_request() => {
                    if !matches!(r, Ok(Some(_))) { return; }
                    n += 1;
                    if n == 3 {
                        seen_tx.send(tokio::time::Instant::now()).unwrap();
                        return; // SETUP, PLAY, then the keepalive arrived
                    }
                }
                _ = tick.tick(), if n >= 2 => {
                    s.send_interleaved(0, b"frame").await.unwrap();
                }
            }
        }
    });
    let t0 = tokio::time::Instant::now();
    let mut c = AsyncRtspClient::with_stream_timeouts(
        client_io,
        ClientSession::new(),
        RtspTimeouts::default().with_read_idle(Duration::from_secs(120)),
    );
    c.setup(
        URI,
        &Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1)),
    )
    .await
    .unwrap();
    c.play(URI).await.unwrap();
    let mut frames = 0;
    // Virtual-time hang guard: without a keepalive the server never ends the stream.
    tokio::time::timeout(Duration::from_secs(40), async {
        while let Some(ev) = c.recv_interleaved().await.unwrap() {
            assert!(matches!(ev, ClientEvent::MediaData { channel: 0, .. }));
            frames += 1;
        }
    })
    .await
    .expect("no keepalive within 40 s of continuous media");
    let after = seen_rx.recv().await.unwrap() - t0;
    assert!(frames >= 8, "media flowed continuously ({frames} frames)");
    assert!(
        after >= Duration::from_secs(10) && after < Duration::from_secs(12),
        "keepalive at {after:?} despite continuous media"
    );
    let _ = server.await;
}

/// Cancel safety (review item 7): drop `next_request` while its response write is blocked; the
/// response must reach the peer exactly once and the events must not be lost.
#[tokio::test]
async fn a_cancelled_server_next_request_neither_loses_events_nor_duplicates_the_response() {
    use rtsp_runtime::ServerEvent;
    let req: &[u8] = b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\n";
    let expected = ServerSession::new(|| 7)
        .with_session_seed(1)
        .handle_request(req)
        .unwrap()
        .0;
    assert!(expected.len() > 48, "response must not fit the pipe");
    let (mut client, server_io) = duplex(48);
    client.write_all(req).await.unwrap();
    let mut s = server_over(server_io, RtspTimeouts::default());
    tokio::select! {
        biased;
        r = s.next_request() => panic!("must block on the write: {r:?}"),
        _ = std::future::ready(()) => {}
    }
    let reader = tokio::spawn(async move {
        let mut got = vec![0u8; expected.len()];
        client.read_exact(&mut got).await.unwrap();
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        (got, rest, expected)
    });
    let events = s
        .next_request()
        .await
        .unwrap()
        .expect("events of the cancelled call");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ServerEvent::RequestAccepted { .. })),
        "{events:?}"
    );
    drop(s);
    let (got, rest, expected) = reader.await.unwrap();
    assert_eq!(got, expected);
    assert!(
        rest.is_empty(),
        "response duplicated: {} extra bytes",
        rest.len()
    );
}

/// Review item 14: EOF in the middle of an interleaved frame is an error, not a clean end.
#[tokio::test(start_paused = true)]
async fn eof_mid_frame_is_an_error_not_a_clean_end() {
    let (client_io, mut server_io) = duplex(4096);
    server_io.write_all(&[0x24, 0, 0, 10, 1, 2]).await.unwrap(); // frame cut short
    drop(server_io);
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c.recv_interleaved().await.expect_err("truncated frame");
    assert!(matches!(err, Error::Io(_)), "{err:?}");
}
