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
/// session: the keepalive fires at the session's half-timeout.
///
/// PRE-FIX: `send_interleaved` never drove the keepalive, so a pusher that only
/// sent interleaved media emitted no `GET_PARAMETER` and its session expired.
///
/// Driven over an in-memory duplex with the test acting as the server in
/// lockstep, so the keepalive assertion does not depend on a real IO driver
/// being polled between a clock advance and a read.
#[tokio::test(start_paused = true)]
async fn a_send_only_pusher_still_emits_a_get_parameter_keepalive() {
    let (client_io, mut peer) = duplex(64 * 1024);
    let mut client = AsyncRtspClient::with_stream_timeouts(
        client_io,
        ClientSession::new(),
        RtspTimeouts::default(),
    );

    let resp = tokio::join!(client.announce(URI, SDP), answer_next(&mut peer, None));
    assert!(matches!(resp.0.unwrap(), ClientEvent::Response { .. }));
    let transport = tcp_interleaved();
    let resp = tokio::join!(
        client.setup(URI, &transport),
        answer_next(&mut peer, Some("Session: 12345678;timeout=2"))
    );
    assert!(matches!(resp.0.unwrap(), ClientEvent::Response { .. }));
    let resp = tokio::join!(client.record(URI), answer_next(&mut peer, None));
    assert!(matches!(resp.0.unwrap(), ClientEvent::Response { .. }));

    // The first send writes an interleaved frame (and no keepalive yet); read
    // and discard it.
    client.send_interleaved(0, b"frame").await.unwrap();
    let frame = read_one_interleaved(&mut peer).await;
    assert_eq!(frame, b"frame", "the pusher's frame must reach the peer");

    // Advance past the keepalive half-timeout (timeout=2 -> 1 s) so the next
    // send must emit a GET_PARAMETER as well.
    tokio::time::advance(Duration::from_secs(5)).await;

    // `send_interleaved` must emit the keepalive BEFORE the frame: read the
    // GET_PARAMETER, answer it 200, then read the frame it wrote after it.
    client.send_interleaved(0, b"frame").await.unwrap();
    // Bounded so a MISSING keepalive fails cleanly instead of hanging the run.
    let (head, _) = tokio::time::timeout(Duration::from_secs(60), read_one_request(&mut peer))
        .await
        .expect("the pusher must emit its keepalive instead of blocking the peer");
    assert!(
        head.starts_with("GET_PARAMETER"),
        "a send-only pusher must emit a GET_PARAMETER keepalive; got: {head}"
    );
    let cseq = header_of(&head, "cseq");
    peer.write_all(
        format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 12345678\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let frame = read_one_interleaved(&mut peer).await;
    assert_eq!(frame, b"frame", "the post-keepalive frame must reach the peer");
}

/// Read one interleaved frame (`$` + channel + big-endian length + payload) off
/// `peer` and return its payload.
async fn read_one_interleaved(peer: &mut tokio::io::DuplexStream) -> Vec<u8> {
    let mut hdr = [0u8; 4];
    peer.read_exact(&mut hdr).await.unwrap();
    assert_eq!(hdr[0], b'$', "interleaved frame must start with $");
    let len = usize::from(u16::from_be_bytes([hdr[2], hdr[3]]));
    let mut payload = vec![0u8; len];
    peer.read_exact(&mut payload).await.unwrap();
    payload
}

/// A `454 Session Not Found` (RFC 2326 §11.3.2) that the server sends in
/// answer to our `GET_PARAMETER` keepalive must be SURFACED to the pusher, not
/// silently discarded by the send-only drain: `send_interleaved` must fail with
/// [`rtsp_runtime::Error::SessionNotFound`] so the pusher learns the session is
/// gone on its next call (instead of only via a later write error).
///
/// PRE-FIX: `drain_inbound` matched every control response into `_ => {}`, so
/// the 454 was swallowed and the pusher kept sending into a dead session.
///
/// Driven over an in-memory duplex with the test itself acting as the server in
/// lockstep (read a request, write its response), so there is no reliance on a
/// real IO driver being polled between a clock advance and a read — the whole
/// exchange is deterministic.
#[tokio::test(start_paused = true)]
async fn a_454_keepalive_response_is_surfaced_not_discarded() {
    // 64 KiB each way — the manually-driven peer never fills them.
    let (client_io, mut peer) = duplex(64 * 1024);
    let mut client = AsyncRtspClient::with_stream_timeouts(
        client_io,
        ClientSession::new(),
        RtspTimeouts::default(),
    );

    // ANNOUNCE.
    let resp = tokio::join!(client.announce(URI, SDP), answer_next(&mut peer, None));
    assert!(matches!(resp.0.unwrap(), ClientEvent::Response { .. }));
    // SETUP: allocate a session with a 2 s timeout so the keepalive is due
    // after 1 s (its half-timeout floor).
    let transport = tcp_interleaved();
    let resp = tokio::join!(
        client.setup(URI, &transport),
        answer_next(&mut peer, Some("Session: 12345678;timeout=2"))
    );
    assert!(matches!(resp.0.unwrap(), ClientEvent::Response { .. }));
    // RECORD.
    let resp = tokio::join!(client.record(URI), answer_next(&mut peer, None));
    assert!(matches!(resp.0.unwrap(), ClientEvent::Response { .. }));

    // Advance past the keepalive half-timeout so the next send emits a
    // GET_PARAMETER (a no-op before this point).
    tokio::time::advance(Duration::from_secs(5)).await;

    // Drive the keepalive explicitly, read its request to learn the CSeq, then
    // queue the 454 the "server" answers with.
    client.poll_keepalive().await.unwrap();
    let cseq = read_cseq_of(&mut peer, "GET_PARAMETER").await;
    peer.write_all(
        format!("RTSP/1.0 454 Session Not Found\r\nCSeq: {cseq}\r\nSession: 12345678\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();

    // The NEXT send must surface the 454 it drains, not swallow it.
    let err = client
        .send_interleaved(0, b"frame")
        .await
        .expect_err("a drained 454 must surface, not be discarded");
    assert!(
        matches!(
            err,
            rtsp_runtime::Error::SessionNotFound {
                method: rtsp_runtime::Method::GetParameter
            }
        ),
        "a 454 to the keepalive must surface as SessionNotFound(GetParameter), got {err:?}"
    );
}

/// Read one RTSP request off `peer` and answer it `200 OK` (the ANNOUNCE
/// round-trip needs a CSeq); `extra_headers` is appended verbatim, e.g. a
/// `Session` header. Returns nothing — used via `join!` alongside the client's
/// own round-trip call.
async fn answer_next(peer: &mut tokio::io::DuplexStream, extra_headers: Option<&str>) {
    let (head, _body) = read_one_request(peer).await;
    let cseq = header_of(&head, "cseq");
    let extra = extra_headers.map(|h| format!("{h}\r\n")).unwrap_or_default();
    peer.write_all(format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n{extra}\r\n").as_bytes())
        .await
        .unwrap();
}

/// Read one request off `peer`, assert its method, and return its `CSeq`.
async fn read_cseq_of(peer: &mut tokio::io::DuplexStream, method: &str) -> String {
    let (head, _body) = read_one_request(peer).await;
    assert!(
        head.starts_with(method),
        "expected a {method} request, got: {head}"
    );
    header_of(&head, "cseq")
}

/// Read one RTSP request (head + `Content-Length` body) off `peer`, returning
/// the head text and the body bytes. Reads byte-by-byte so it never over-reads
/// past this request into a following interleaved frame.
async fn read_one_request(peer: &mut tokio::io::DuplexStream) -> (String, Vec<u8>) {
    const BLANK: [u8; 4] = [0x0D, 0x0A, 0x0D, 0x0A];
    let mut buf: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    let head_end = loop {
        let n = peer.read(&mut byte).await.unwrap();
        assert!(n > 0, "peer closed before a full request");
        buf.push(byte[0]);
        if buf.len() >= 4 && buf[buf.len() - 4..] == BLANK {
            break buf.len();
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let clen = header_of(&head, "content-length")
        .parse::<usize>()
        .unwrap_or(0);
    let mut body = vec![0u8; clen];
    if clen > 0 {
        peer.read_exact(&mut body).await.unwrap();
    }
    (head, body)
}

/// Case-insensitive lookup of a header's value in a raw RTSP head.
fn header_of(head: &str, name: &str) -> String {
    head.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
        .unwrap_or_default()
}

/// A peer that floods interleaved frames must not make `send_interleaved`'s
/// inbound drain loop forever: the drain is capped per call, so the pusher's
/// own send still completes. PRE-FIX `drain_inbound` looped until the socket
/// went quiet, so a continuously-streaming (or hostile RECORD peer) starved the
/// send — this call would never return.
#[tokio::test]
async fn a_flooding_peer_does_not_starve_the_send() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // The peer writes interleaved RTP frames (`$` + channel + len + payload)
    // as fast as it can, forever, until the client drops the connection.
    let http_server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // A 32-byte frame on channel 0: no `\r\n` so it can never be mistaken
        // for a control response.
        let mut frame = vec![b'$', 0u8, 0u8, 30u8];
        frame.extend(std::iter::repeat_n(0xABu8, 30));
        loop {
            if sock.write_all(&frame).await.is_err() {
                return;
            }
        }
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    // `send_interleaved` drains inbound first; against a peer that never stops
    // writing it must still return promptly (the drain is capped per call).
    let sent = tokio::time::timeout(Duration::from_secs(5), client.send_interleaved(0, b"ping")).await;
    assert!(
        sent.is_ok(),
        "send_interleaved must return even while the peer floods interleaved frames"
    );
    http_server.abort();
}
