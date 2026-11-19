//! Audit 22: the async client must refuse a failed PLAY, ignore a stray `CSeq`
//! reply, keep its pending media queue bounded, and report the state and
//! timeouts it really holds.
#![cfg(feature = "tokio")]

use std::time::Duration;

use broadcast_common::timeouts::Timeouts;
use rtsp_runtime::client::ClientEvent;
use rtsp_runtime::server::ServerEvent;
use rtsp_runtime::{
    AsyncRtspClient, AsyncRtspServer, Error, Method, RtspClientIoConfig, RtspServerConfig,
    SessionId, SessionIdSource, SessionState, StatusCode, Transport, TransportSpec,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use url::Url;

const URI: &str = "rtsp://127.0.0.1/stream";

const SDP: &str = "v=0\r\n\
                   o=- 0 0 IN IP4 127.0.0.1\r\n\
                   s=cam\r\n\
                   t=0 0\r\n\
                   m=video 0 RTP/AVP 96\r\n\
                   a=control:streamid=0\r\n\
                   a=rtpmap:96 H264/90000\r\n";

const MAX_PENDING_MEDIA_FRAMES: usize = 1024;

async fn read_request(peer: &mut DuplexStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = peer.read(&mut chunk).await.expect("read a request");
        assert_ne!(n, 0, "the peer closed before a request head");
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return String::from_utf8(buf).expect("a UTF-8 request");
        }
    }
}

fn client(io: DuplexStream) -> AsyncRtspClient<DuplexStream> {
    AsyncRtspClient::from_stream(
        io,
        RtspClientIoConfig::default()
            .with_timeouts(Timeouts::default().with_read_idle(Duration::from_secs(1))),
    )
}

fn tcp_interleaved() -> Transport {
    Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))
}

#[tokio::test]
async fn a_play_answered_with_an_error_fails_start_play() {
    for (code, status) in [
        (
            "455 Method Not Valid in This State",
            StatusCode::MethodNotValidInThisState,
        ),
        ("454 Session Not Found", StatusCode::SessionNotFound),
    ] {
        let (client_io, mut server_io) = duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let describe = read_request(&mut server_io).await;
            assert!(describe.starts_with("DESCRIBE "), "{describe}");
            let reply = format!(
                "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{SDP}",
                SDP.len()
            );
            server_io
                .write_all(reply.as_bytes())
                .await
                .expect("the DESCRIBE reply");
            let setup = read_request(&mut server_io).await;
            assert!(setup.starts_with("SETUP "), "{setup}");
            server_io
                .write_all(
                    b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nSession: 0123456789abcdef\r\n\
                      Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
                )
                .await
                .expect("the SETUP reply");
            let play = read_request(&mut server_io).await;
            assert!(play.starts_with("PLAY "), "{play}");
            let reply = format!("RTSP/1.0 {code}\r\nCSeq: 3\r\n\r\n");
            server_io
                .write_all(reply.as_bytes())
                .await
                .expect("the PLAY reply");
        });

        let mut c = client(client_io);
        let url = Url::parse(URI).expect("a URL");
        let outcome = tokio::time::timeout(Duration::from_secs(5), c.start_play(&url, 0))
            .await
            .expect("start_play hung");
        match outcome {
            Err(Error::UnexpectedResponse {
                method,
                status: got,
            }) => {
                assert_eq!(method, Method::Play);
                assert_eq!(got, status);
            }
            other => panic!("a {code} PLAY must fail start_play: {other:?}"),
        }
        server.await.expect("the server task");
    }
}

#[tokio::test]
async fn a_stale_response_is_dropped_and_the_current_cseq_answered() {
    let (client_io, mut server_io) = duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let first = read_request(&mut server_io).await;
        assert!(first.starts_with("OPTIONS "), "{first}");
        let second = read_request(&mut server_io).await;
        assert!(second.starts_with("DESCRIBE "), "{second}");
        server_io
            .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\nRTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n")
            .await
            .expect("the stale reply then the real one");
    });

    let mut c = client(client_io);
    let abandoned = tokio::time::timeout(Duration::from_millis(50), c.options(URI)).await;
    assert!(
        abandoned.is_err(),
        "the first exchange is abandoned before its answer: {abandoned:?}"
    );
    let event = tokio::time::timeout(Duration::from_secs(5), c.describe(URI))
        .await
        .expect("describe hung")
        .expect("the DESCRIBE response");
    match event {
        ClientEvent::Response {
            cseq,
            method,
            status,
            ..
        } => {
            assert_eq!(cseq, 2, "the stale CSeq 1 reply must not answer DESCRIBE");
            assert_eq!(method, Method::Describe);
            assert_eq!(status, StatusCode::Ok);
        }
        other => panic!("expected the DESCRIBE response, got {other:?}"),
    }
    server.await.expect("the server task");
}

#[tokio::test]
async fn pending_media_is_capped_by_dropping_the_oldest() {
    let (client_io, mut server_io) = duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let request = read_request(&mut server_io).await;
        assert!(request.starts_with("OPTIONS "), "{request}");
        let mut wire = Vec::new();
        for index in 0..MAX_PENDING_MEDIA_FRAMES + 5 {
            let payload = u16::try_from(index).expect("index fits").to_be_bytes();
            wire.push(rtsp_runtime::interleaved::MAGIC);
            wire.push(0);
            wire.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_be_bytes());
            wire.extend_from_slice(&payload);
        }
        wire.extend_from_slice(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n");
        server_io
            .write_all(&wire)
            .await
            .expect("the frames then the response");
    });

    let mut c = client(client_io);
    tokio::time::timeout(Duration::from_secs(5), c.options(URI))
        .await
        .expect("options hung")
        .expect("the OPTIONS response");

    let mut seen = Vec::new();
    for _ in 0..MAX_PENDING_MEDIA_FRAMES + 5 {
        match tokio::time::timeout(Duration::from_secs(2), c.recv_interleaved()).await {
            Ok(Ok(Some(ClientEvent::MediaData { channel, data }))) => {
                assert_eq!(channel, 0);
                seen.extend_from_slice(&data);
            }
            Ok(Ok(None)) => break,
            Ok(other) => panic!("unexpected recv outcome: {other:?}"),
            Err(_) => break,
        }
    }
    server.await.expect("the server task");

    assert_eq!(seen.len(), MAX_PENDING_MEDIA_FRAMES * 2);
    for (index, pair) in seen.chunks_exact(2).enumerate() {
        assert_eq!(
            u16::from_be_bytes([pair[0], pair[1]]),
            u16::try_from(index + 5).unwrap(),
            "the oldest frames are dropped, the newest kept, in order"
        );
    }
}

#[tokio::test]
async fn the_server_accessors_report_the_configured_and_allocated_values() {
    let (mut peer, server_io) = duplex(64 * 1024);
    let timeouts = Timeouts::default()
        .with_read_idle(Duration::from_secs(7))
        .with_handshake(Duration::from_secs(8));
    let mut server = AsyncRtspServer::from_stream(
        server_io,
        RtspServerConfig::default()
            .with_session_ids(SessionIdSource::counter(0x1234))
            .with_timeouts(timeouts),
    )
    .expect("a server");
    peer.write_all(
        b"SETUP rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP/TCP;interleaved=0-1\r\n\r\n",
    )
    .await
    .expect("the SETUP request");
    let events = tokio::time::timeout(Duration::from_secs(5), server.next_request())
        .await
        .expect("next_request hung")
        .expect("a SETUP")
        .expect("events");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ServerEvent::SessionSetup { .. }))
    );

    let id = server.session_id().expect("SETUP allocated an id");
    assert_eq!(id, SessionId::new(0x1234));
    assert_eq!(server.timeouts(), timeouts);
    assert_eq!(server.timeouts().read_idle, Duration::from_secs(7));

    let mut response = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(5), peer.read(&mut response))
        .await
        .expect("reading the response hung")
        .expect("the SETUP response");
    let text = String::from_utf8_lossy(&response[..n]).to_string();
    assert!(text.contains(&format!("Session: {id}")), "{text}");
}

#[tokio::test]
async fn the_client_session_accessor_reflects_the_setup_answer() {
    let (client_io, mut server_io) = duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let request = read_request(&mut server_io).await;
        assert!(request.starts_with("SETUP "), "{request}");
        server_io
            .write_all(
                b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nSession: 0123456789abcdef\r\n\
                  Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
            )
            .await
            .expect("the SETUP reply");
    });

    let mut c = client(client_io);
    tokio::time::timeout(Duration::from_secs(5), c.setup(URI, &tcp_interleaved()))
        .await
        .expect("setup hung")
        .expect("the SETUP response");

    let id = SessionId::new(0x0123_4567_89ab_cdef);
    assert_eq!(c.session().state(), SessionState::Ready);
    assert_eq!(c.session().session_id(), Some(id));
    assert_eq!(c.session_id(), Some(id));
    server.await.expect("the server task");
}

#[tokio::test]
async fn the_push_drain_applies_the_same_media_bound() {
    let (client_io, mut server_io) = duplex(256 * 1024);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut wire = Vec::new();
        for index in 0..MAX_PENDING_MEDIA_FRAMES + 5 {
            let payload = u16::try_from(index).expect("index fits").to_be_bytes();
            wire.push(rtsp_runtime::interleaved::MAGIC);
            wire.push(0);
            wire.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_be_bytes());
            wire.extend_from_slice(&payload);
        }
        server_io.write_all(&wire).await.expect("the frames");
        let _ = ready_tx.send(());
        let _ = done_rx.await;
    });

    ready_rx.await.expect("the server wrote its frames");
    let mut c = client(client_io);
    for _ in 0..32 {
        c.send_interleaved(1, b"r").await.expect("the push write");
    }
    let _ = done_tx.send(());
    server.await.expect("the server task");

    let mut seen = Vec::new();
    for _ in 0..MAX_PENDING_MEDIA_FRAMES + 5 {
        match tokio::time::timeout(Duration::from_secs(2), c.recv_interleaved()).await {
            Ok(Ok(Some(ClientEvent::MediaData { channel, data }))) => {
                assert_eq!(channel, 0);
                seen.extend_from_slice(&data);
            }
            Ok(Ok(None)) => break,
            Ok(other) => panic!("unexpected recv outcome: {other:?}"),
            Err(_) => break,
        }
    }
    assert_eq!(seen.len(), MAX_PENDING_MEDIA_FRAMES * 2);
    for (index, pair) in seen.chunks_exact(2).enumerate() {
        assert_eq!(
            u16::from_be_bytes([pair[0], pair[1]]),
            u16::try_from(index + 5).unwrap(),
            "the push drain drops the oldest and keeps the newest, in order"
        );
    }
}
