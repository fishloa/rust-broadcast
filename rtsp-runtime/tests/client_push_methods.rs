//! `AsyncRtspClient::announce`/`record`/`send_interleaved` (W2b-1 Task 6) — the
//! RTSP push (ANNOUNCE -> SETUP -> RECORD with interleaved delivery) needs them.
#![cfg(feature = "tokio")]

use std::time::Duration;

use rtsp_runtime::client::ClientEvent;
use rtsp_runtime::server::ServerEvent;
use rtsp_runtime::{
    AsyncRtspClient, AsyncRtspServer, ClientSession, RtspTimeouts, StatusCode, Transport,
    TransportSpec,
};
use tokio::io::duplex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const URI: &str = "rtsp://127.0.0.1/live";

fn tcp_interleaved() -> Transport {
    Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))
}

const SDP: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n";

#[tokio::test]
async fn announce_then_record_round_trips_over_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        // ANNOUNCE, SETUP, RECORD = 3 requests.
        let mut methods = Vec::new();
        for _ in 0..3 {
            let events = srv.next_request().await.unwrap().expect("request");
            for e in events {
                if let ServerEvent::RequestAccepted { method, .. } = e {
                    methods.push(method);
                }
            }
        }
        methods
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    let ev = client.announce(URI, SDP).await.unwrap();
    assert!(
        matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok),
        "{ev:?}"
    );
    let ev = client.setup(URI, &tcp_interleaved()).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { .. }), "{ev:?}");
    let ev = client.record(URI).await.unwrap();
    assert!(
        matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok),
        "{ev:?}"
    );
    // After RECORD the client session must be in the Recording state.
    assert_eq!(
        client.state(),
        rtsp_runtime::SessionState::Recording,
        "a pusher that has RECORDed must be in the Recording state"
    );

    let methods = server.await.unwrap();
    assert_eq!(methods.len(), 3, "server saw {methods:?}");
}

#[tokio::test]
async fn a_client_interleaved_send_arrives_as_a_frame_at_the_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        loop {
            let events = srv.next_request().await.unwrap()?;
            for e in events {
                if let ServerEvent::MediaData { channel, data } = e {
                    return Some((channel, data));
                }
            }
        }
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    client.send_interleaved(0, b"media-payload").await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server must receive the interleaved frame")
        .unwrap()
        .expect("a MediaData event");
    assert_eq!(got.0, 0, "channel");
    assert_eq!(got.1, b"media-payload".to_vec());
}

#[tokio::test(start_paused = true)]
async fn a_stalled_interleaved_send_times_out_at_the_write_bound() {
    // An 8-byte duplex pipe whose peer is kept alive but never drained: the
    // write blocks, and the `write` bound must fail it (paused virtual time).
    let (client_io, _server_io) = duplex(8);
    let timeouts = RtspTimeouts::default().with_write(Duration::from_secs(1));
    let mut client =
        AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), timeouts);
    // Wrapped in an outer virtual-time bound so a MISSING write bound fails
    // with a clear message instead of hanging the suite.
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        client.send_interleaved(0, &[0u8; 64]),
    )
    .await
    .expect("the stalled send must fail at the write bound, not hang")
    .expect_err("the stalled send must fail at the write bound");
    assert!(
        matches!(err, rtsp_runtime::Error::Timeout { what: "write" }),
        "got {err:?}"
    );
}

/// The ANNOUNCE the pusher sends must carry the SDP body and
/// `Content-Type: application/sdp` on the wire (the round-trip test above only
/// counted methods). Captured by a raw listener, since `AsyncRtspServer` does
/// not surface the request body.
#[tokio::test]
async fn announce_carries_the_sdp_body_and_content_type_to_the_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Read the ANNOUNCE request (head + Content-Length body), then ack it.
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let head_end = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            assert!(n > 0, "peer closed before a full request");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let clen = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        while buf.len() < head_end + clen {
            let n = sock.read(&mut tmp).await.unwrap();
            assert!(n > 0, "peer closed before the body");
            buf.extend_from_slice(&tmp[..n]);
        }
        sock.write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n")
            .await
            .unwrap();
        buf
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    let ev = client.announce(URI, SDP).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { .. }), "{ev:?}");

    let text = String::from_utf8_lossy(&server.await.unwrap()).to_string();
    assert!(
        text.starts_with("ANNOUNCE "),
        "first request must be ANNOUNCE: {text}"
    );
    assert!(
        text.to_ascii_lowercase()
            .contains("content-type: application/sdp"),
        "ANNOUNCE must declare application/sdp: {text}"
    );
    assert!(
        text.contains(SDP.trim_end_matches("\r\n")),
        "ANNOUNCE body must carry the SDP: {text}"
    );
}

/// A SEND-ONLY pusher that only calls `send_interleaved` must still refresh its
/// session: the keepalive fires (deterministically, under a paused clock) at
/// the session's half-timeout.
///
/// PRE-FIX: `send_interleaved` never drove the keepalive, so a pusher that only
/// sent interleaved media emitted no `GET_PARAMETER` and its session expired.
#[tokio::test]
async fn a_send_only_pusher_still_emits_a_get_parameter_keepalive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut requests: Vec<String> = Vec::new();
        let mut saw_frame = false;
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 4096];
        'outer: for _ in 0..64 {
            let n = match sock.read(&mut tmp).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            buf.extend_from_slice(&tmp[..n]);
            loop {
                if buf.is_empty() {
                    continue 'outer;
                }
                if buf[0] == b'$' {
                    if buf.len() < 4 {
                        continue 'outer;
                    }
                    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                    if buf.len() < 4 + len {
                        continue 'outer;
                    }
                    saw_frame = true;
                    buf.drain(..4 + len);
                    continue;
                }
                let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
                else {
                    continue 'outer;
                };
                let text = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let clen = text
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if buf.len() < head_end + clen {
                    continue 'outer;
                }
                let cseq = text
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("cseq").then(|| v.trim().to_string())
                    })
                    .unwrap_or_default();
                let method = text.split(' ').next().unwrap_or("").to_string();
                // A SETUP response allocates a Session id with a short declared
                // timeout so the keepalive arms and fires quickly.
                let resp = if method == "SETUP" {
                    format!(
                        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 12345678;timeout=2\r\n\r\n"
                    )
                } else {
                    format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 12345678\r\n\r\n")
                };
                sock.write_all(resp.as_bytes()).await.unwrap();
                buf.drain(..head_end + clen);
                let done = saw_frame && method == "GET_PARAMETER";
                requests.push(method);
                if done {
                    break 'outer;
                }
            }
        }
        requests
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    client.announce(URI, SDP).await.unwrap();
    client.setup(URI, &tcp_interleaved()).await.unwrap();
    client.record(URI).await.unwrap();

    // Pause virtual time only now (real time for connect/handshake), so the
    // keepalive half-timeout can be advanced deterministically, not slept out.
    tokio::time::pause();
    for _ in 0..5 {
        client
            .send_interleaved(0, b"frame")
            .await
            .expect("interleaved send");
        // Advance past the keepalive half-timeout (timeout=2 -> 1 s).
        tokio::time::advance(Duration::from_secs(5)).await;
    }

    // Resume real time so the final bounded wait cannot be short-circuited.
    tokio::time::resume();
    let seen = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the server task must finish")
        .unwrap();
    assert!(
        seen.iter().any(|m| m == "GET_PARAMETER"),
        "a send-only pusher must emit a GET_PARAMETER keepalive; saw {seen:?}"
    );
}

/// A `454 Session Not Found` (RFC 2326 §11.3.2) that the server sends in
/// answer to our `GET_PARAMETER` keepalive must be SURFACED to the pusher, not
/// silently discarded by the send-only drain: `send_interleaved` must fail with
/// [`rtsp_runtime::Error::SessionNotFound`] so the pusher learns the session is
/// gone on its next call (instead of only via a later write error).
///
/// PRE-FIX: `drain_inbound` matched every control response into `_ => {}`, so
/// the 454 was swallowed and the pusher kept sending into a dead session.
#[tokio::test]
async fn a_454_keepalive_response_is_surfaced_not_discarded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 4096];
        'outer: for _ in 0..64 {
            let n = match sock.read(&mut tmp).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            buf.extend_from_slice(&tmp[..n]);
            loop {
                if buf.is_empty() {
                    continue 'outer;
                }
                if buf[0] == b'$' {
                    if buf.len() < 4 {
                        continue 'outer;
                    }
                    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                    if buf.len() < 4 + len {
                        continue 'outer;
                    }
                    buf.drain(..4 + len);
                    continue;
                }
                let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
                else {
                    continue 'outer;
                };
                let text = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let clen = text
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if buf.len() < head_end + clen {
                    continue 'outer;
                }
                let cseq = text
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("cseq").then(|| v.trim().to_string())
                    })
                    .unwrap_or_default();
                let method = text.split(' ').next().unwrap_or("").to_string();
                // SETUP arms a short session timeout so the keepalive fires; the
                // answer to the keepalive's GET_PARAMETER is a 454 — the session
                // the server "forgot".
                let resp = match method.as_str() {
                    "SETUP" => format!(
                        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 12345678;timeout=2\r\n\r\n"
                    ),
                    "GET_PARAMETER" => {
                        format!(
                            "RTSP/1.0 454 Session Not Found\r\nCSeq: {cseq}\r\n\
                             Session: 12345678\r\n\r\n"
                        )
                    }
                    _ => format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 12345678\r\n\r\n"),
                };
                sock.write_all(resp.as_bytes()).await.unwrap();
                buf.drain(..head_end + clen);
                if method == "GET_PARAMETER" {
                    break 'outer;
                }
            }
        }
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    client.announce(URI, SDP).await.unwrap();
    client.setup(URI, &tcp_interleaved()).await.unwrap();
    client.record(URI).await.unwrap();

    // Pause virtual time only now (real time for connect/handshake), so the
    // keepalive half-timeout fires deterministically.
    tokio::time::pause();
    let mut errored = None;
    for _ in 0..10 {
        match client.send_interleaved(0, b"frame").await {
            Ok(()) => {
                // Advance past the keepalive half-timeout to trigger the next
                // GET_PARAMETER, and let the server's 454 land.
                tokio::time::advance(Duration::from_secs(5)).await;
                tokio::task::yield_now().await;
            }
            Err(e) => {
                errored = Some(e);
                break;
            }
        }
    }
    tokio::time::resume();

    let err = errored.expect("the 454 must surface to the pusher, not be discarded");
    assert!(
        matches!(
            err,
            rtsp_runtime::Error::SessionNotFound {
                method: rtsp_runtime::Method::GetParameter
            }
        ),
        "a 454 to the keepalive must surface as SessionNotFound(GetParameter), got {err:?}"
    );
    let _ = server.await;
}
