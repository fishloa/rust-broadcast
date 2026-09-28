//! Real-socket loopback integration tests for the async IO adapter (RFC 2326).
//!
//! Every test spins up a genuine `127.0.0.1:0` TCP (or TLS) listener in a tokio
//! task and drives a real socket round-trip through
//! [`AsyncRtspClient`]/[`AsyncRtspServer`] — no mocks, no in-memory pipes.
#![cfg(feature = "tokio")]

use rtsp_runtime::client::ClientEvent;
use rtsp_runtime::server::ServerEvent;
use rtsp_runtime::{
    AsyncRtspClient, AsyncRtspServer, ClientSession, Credentials, SessionState, StatusCode,
    Transport, TransportSpec,
};
use tokio::net::{TcpListener, TcpStream};

const URI: &str = "rtsp://127.0.0.1/stream";

fn tcp_interleaved() -> Transport {
    Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))
}

// ---------------------------------------------------------------------------
// 1. Plain-TCP full session over loopback.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_tcp_full_session() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Server task: drive Init -> Ready -> Playing -> Init and record the states.
    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        let mut states = Vec::new();
        // OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN = 5 requests.
        for _ in 0..5 {
            let events = srv.next_request().await.unwrap().expect("request");
            states.push(srv.state());
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, ServerEvent::RequestAccepted { .. }))
            );
        }
        states
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();

    let ev = client.options(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));

    let ev = client.describe(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));

    let ev = client.setup(URI, &tcp_interleaved()).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));
    assert_eq!(client.state(), SessionState::Ready);
    assert!(client.session_id().is_some());

    let ev = client.play(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));
    assert_eq!(client.state(), SessionState::Playing);

    let ev = client.teardown(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));
    assert_eq!(client.state(), SessionState::Init);

    let server_states = server.await.unwrap();
    // OPTIONS + DESCRIBE are state-neutral (Init), SETUP -> Ready, PLAY -> Playing,
    // TEARDOWN -> Init.
    assert_eq!(
        server_states,
        vec![
            SessionState::Init,    // OPTIONS
            SessionState::Init,    // DESCRIBE
            SessionState::Ready,   // SETUP
            SessionState::Playing, // PLAY
            SessionState::Init,    // TEARDOWN
        ]
    );
}

// ---------------------------------------------------------------------------
// 2. Interleaved media over TCP, including a fragmented frame.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn interleaved_media_over_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let payload1: Vec<u8> = (0u8..32).collect();
    let payload2: Vec<u8> = (100u8..140).collect();
    let p1 = payload1.clone();
    let p2 = payload2.clone();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        // SETUP then PLAY.
        srv.next_request().await.unwrap().expect("SETUP");
        srv.next_request().await.unwrap().expect("PLAY");
        // Two interleaved frames sent whole over the real socket. (Split-read
        // reassembly across TCP read boundaries is covered deterministically by
        // the `interleaved::tests::two_frames_plus_partial_returns_two_and_remainder`
        // unit test; forcing a split here with a timing sleep raced under load.)
        srv.send_interleaved(0, &p1).await.unwrap();
        srv.send_interleaved(0, &p2).await.unwrap();
        srv.stream_mut().flush().await.unwrap();
        // Keep the connection open until the client has read both frames.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    client.setup(URI, &tcp_interleaved()).await.unwrap();
    client.play(URI).await.unwrap();

    let f1 = client.recv_interleaved().await.unwrap().expect("frame 1");
    assert!(matches!(f1, ClientEvent::MediaData { channel: 0, ref data } if *data == payload1));

    let f2 = client.recv_interleaved().await.unwrap().expect("frame 2");
    assert!(matches!(f2, ClientEvent::MediaData { channel: 0, ref data } if *data == payload2));

    server.await.unwrap();
}

// ---------------------------------------------------------------------------
// 3. Digest auth over loopback.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn digest_auth_over_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // A minimal hand-rolled server: first DESCRIBE -> 401 Digest challenge, then
    // the authenticated retry -> 200. We read requests as raw bytes so we can
    // assert the second one carries a valid Authorization header.
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Read the first (unauthenticated) DESCRIBE.
        let req1 = read_one_request(&mut sock).await;
        assert!(req1.contains("DESCRIBE"));
        assert!(
            !req1.contains("Authorization:"),
            "first request must be unauthenticated"
        );
        let cseq1 = cseq_of(&req1);
        let challenge = "Digest realm=\"IP Camera\", \
             nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", qop=\"auth\", algorithm=MD5";
        let resp401 = format!(
            "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq1}\r\n\
             WWW-Authenticate: {challenge}\r\n\r\n"
        );
        sock.write_all(resp401.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();

        // Read the authenticated retry.
        let req2 = read_one_request(&mut sock).await;
        assert!(req2.contains("DESCRIBE"));
        assert!(
            req2.contains("Authorization: Digest "),
            "retry must carry a Digest Authorization: {req2}"
        );
        assert!(req2.contains("response="));
        assert!(req2.contains("nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\""));
        let cseq2 = cseq_of(&req2);
        let ok = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq2}\r\n\r\n");
        sock.write_all(ok.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        cseq2
    });

    let session = ClientSession::new().with_credentials(Credentials::new("admin", "12345"));
    let mut client = AsyncRtspClient::connect_with(addr, session).await.unwrap();
    // describe() must complete transparently through the 401 -> retry -> 200.
    let ev = client.describe(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));

    let cseq2 = server.await.unwrap();
    assert!(cseq2 > 1, "retry used a fresh CSeq");
}

// ---------------------------------------------------------------------------
// 4. TLS (rtsps://) full session over loopback with a self-signed cert.
// ---------------------------------------------------------------------------

#[cfg(feature = "tls")]
#[tokio::test]
async fn tls_full_session_over_loopback() {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    // Committed self-signed localhost cert (DER) — not secret; a test fixture.
    let cert_der = include_bytes!("fixtures/localhost-cert.der").to_vec();
    let key_der = include_bytes!("fixtures/localhost-key.der").to_vec();

    let server_config = {
        let certs = vec![CertificateDer::from(cert_der.clone())];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));
        // Explicit aws-lc-rs provider — see `io::default_tls_client_config`: with
        // `aws-lc-rs` also in the workspace build (via reqwest elsewhere) the
        // plain `::builder()` has no unambiguous default provider and panics.
        rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("aws-lc-rs provider supports the safe default protocol versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config")
    };

    let client_config = {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(cert_der))
            .expect("add self-signed root");
        rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("aws-lc-rs provider supports the safe default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept_tls(tcp, server_config)
            .await
            .expect("TLS handshake (server)");
        let mut states = Vec::new();
        for _ in 0..4 {
            srv.next_request().await.unwrap().expect("request");
            states.push(srv.state());
        }
        states
    });

    let mut client = AsyncRtspClient::connect_tls(addr, "localhost", client_config)
        .await
        .expect("TLS handshake (client)");

    let ev = client.options(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));
    let ev = client.setup(URI, &tcp_interleaved()).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));
    assert_eq!(client.state(), SessionState::Ready);
    let ev = client.play(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));
    assert_eq!(client.state(), SessionState::Playing);
    let ev = client.teardown(URI).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok));

    let states = server.await.unwrap();
    assert_eq!(
        states,
        vec![
            SessionState::Init,    // OPTIONS
            SessionState::Ready,   // SETUP
            SessionState::Playing, // PLAY
            SessionState::Init,    // TEARDOWN
        ]
    );
}

// ---------------------------------------------------------------------------
// 5. A stray response for an abandoned request must not win over the real one.
// ---------------------------------------------------------------------------

// Regression (audit run-09 W7): `AsyncRtspClient::exchange` used to keep
// whichever `Response` event was LAST in a decoded batch, with no CSeq check.
// If a previous `exchange` call's future was dropped (e.g. a caller-side
// `tokio::time::timeout`) after its request was already written, that
// request's `Pending` entry stays in the session; a late response for it can
// then arrive coalesced with the response actually being awaited. Here the
// server deliberately answers CSeq 2 (the real, currently-awaited request)
// FIRST and CSeq 1 (the abandoned one) SECOND in one write, so the pre-fix
// "last Response event wins" bug would return CSeq 1's response instead.
#[tokio::test]
async fn stray_response_for_an_abandoned_request_does_not_win() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Wait for BOTH requests (OPTIONS CSeq 1, DESCRIBE CSeq 2) to fully
        // arrive before answering either.
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if buf.windows(4).filter(|w| *w == b"\r\n\r\n").count() >= 2 {
                break;
            }
            let n = sock.read(&mut chunk).await.unwrap();
            assert!(n > 0, "peer closed before both requests arrived");
            buf.extend_from_slice(&chunk[..n]);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: 0\r\n\r\n");
        out.extend_from_slice(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Length: 0\r\n\r\n");
        sock.write_all(&out).await.unwrap();
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();

    // CSeq 1: written to the socket, then abandoned before any response
    // arrives (the server above never answers until CSeq 2 is also sent).
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), client.options(URI)).await;

    // CSeq 2: the request actually under test.
    let ev = client.describe(URI).await.unwrap();
    match ev {
        ClientEvent::Response { cseq, .. } => assert_eq!(
            cseq, 2,
            "returned the stray CSeq-1 response instead of the awaited CSeq-2 one"
        ),
        other => panic!("unexpected event: {other:?}"),
    }

    server.await.unwrap();
}

// ---------------------------------------------------------------------------
// 6. The server must accept an interleaved `$` frame during PLAY, not error.
// ---------------------------------------------------------------------------

// Regression (audit run-09 W8): RFC 2326 §10.12 lets a client send `$`-framed
// data on the same TCP connection (an RTCP receiver report during PLAY, or
// media during RECORD) — ffmpeg, VLC and GStreamer all do this. Before the
// fix, `AsyncRtspServer::next_request` treated a leading `$` as a malformed
// request and errored, dropping any TCP-interleaved PLAY client at its first
// RTCP RR.
#[tokio::test]
async fn server_receives_interleaved_frame_during_play() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        let setup_events = srv.next_request().await.unwrap().expect("SETUP");
        assert!(
            setup_events
                .iter()
                .any(|e| matches!(e, ServerEvent::SessionSetup { .. }))
        );
        let play_events = srv.next_request().await.unwrap().expect("PLAY");
        assert!(
            play_events
                .iter()
                .any(|e| matches!(e, ServerEvent::RequestAccepted { .. }))
        );
        // The peer now sends an interleaved RTCP receiver report — this must
        // surface as `MediaData`, not fail the connection.
        srv.next_request()
            .await
            .unwrap()
            .expect("interleaved frame")
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    client
        .write_all(
            b"SETUP rtsp://127.0.0.1/stream RTSP/1.0\r\n\
              CSeq: 1\r\n\
              Transport: RTP/AVP/TCP;interleaved=0-1\r\n\r\n",
        )
        .await
        .unwrap();
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]).to_string();
    assert!(resp.contains("200"), "SETUP failed: {resp}");
    let sid = resp
        .lines()
        .find_map(|l| l.strip_prefix("Session:"))
        .expect("Session header")
        .split(';')
        .next()
        .unwrap()
        .trim()
        .to_string();

    client
        .write_all(
            format!("PLAY rtsp://127.0.0.1/stream RTSP/1.0\r\nCSeq: 2\r\nSession: {sid}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let n = client.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).contains("200"));

    // An interleaved frame on channel 1 (odd = RTCP by convention).
    let rtcp_payload = vec![0xAB_u8; 24];
    let mut frame = vec![0x24u8, 1];
    frame.extend_from_slice(&(rtcp_payload.len() as u16).to_be_bytes());
    frame.extend_from_slice(&rtcp_payload);
    client.write_all(&frame).await.unwrap();

    let events = server.await.unwrap();
    assert!(
        events.iter().any(
            |e| matches!(e, ServerEvent::MediaData { channel: 1, data } if *data == rtcp_payload)
        ),
        "expected a MediaData event for the interleaved RTCP frame: {events:?}"
    );
}

// Regression (audit run-09 W8): an unterminated request header must not grow
// `AsyncRtspServer`'s read buffer without bound.
#[tokio::test]
async fn server_read_buffer_is_capped_against_an_unterminated_header() {
    use tokio::io::AsyncWriteExt;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        srv.next_request().await
    });

    let client = TcpStream::connect(addr).await.unwrap();
    let mut client = client;
    // A header line with no CRLFCRLF terminator, well past the 2 MiB cap.
    let chunk = vec![b'A'; 64 * 1024];
    for _ in 0..40 {
        if client.write_all(&chunk).await.is_err() {
            break;
        }
    }
    // Keep `client` open: the cap must trip on its own, not via the peer
    // closing the connection (a clean EOF is a different, already-handled
    // error path) — otherwise this test can't tell a capped buffer from an
    // unbounded one that just happens to see EOF.
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .expect("server task must reject the oversized buffer instead of blocking forever")
        .unwrap();
    assert!(
        result.is_err(),
        "expected the oversized request buffer to be rejected, not grown forever"
    );
    drop(client);
}

// Regression (audit run-09 W7): `pending_media` must not grow without bound
// while `exchange` waits on a response — e.g. a camera that keeps streaming
// media but is slow to answer `GET_PARAMETER`. Once full, the OLDEST frame is
// dropped in favour of newer ones.
#[tokio::test]
async fn pending_media_queue_is_capped_while_exchange_waits() {
    const OVERFLOW: usize = 1024 + 50; // > MAX_PENDING_MEDIA_FRAMES (private, io.rs)

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        srv.next_request().await.unwrap().expect("SETUP");
        srv.next_request().await.unwrap().expect("PLAY");
        // Flush every media frame before ever reading/answering the
        // GET_PARAMETER that follows, so the client decodes them all while
        // still waiting on that response.
        for i in 0..OVERFLOW {
            let payload = (i as u32).to_be_bytes().to_vec();
            srv.send_interleaved(0, &payload).await.unwrap();
        }
        srv.next_request().await.unwrap().expect("GET_PARAMETER");
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    client.setup(URI, &tcp_interleaved()).await.unwrap();
    client.play(URI).await.unwrap();
    client.get_parameter(URI, &[]).await.unwrap();

    let mut indices = Vec::new();
    while indices.len() < 1024 {
        match client.recv_interleaved().await.unwrap() {
            Some(ClientEvent::MediaData { data, .. }) => {
                indices.push(u32::from_be_bytes(data.try_into().unwrap()));
            }
            Some(other) => panic!("unexpected event: {other:?}"),
            None => break,
        }
    }
    assert_eq!(
        indices.len(),
        1024,
        "expected exactly the capped number of frames, got {}",
        indices.len()
    );
    assert_eq!(
        indices[0],
        (OVERFLOW - 1024) as u32,
        "expected the oldest frames to have been dropped, not the newest"
    );
    assert_eq!(*indices.last().unwrap(), (OVERFLOW - 1) as u32);

    server.await.unwrap();
}

// --- helpers ---------------------------------------------------------------

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Reads bytes off `sock` until a complete RTSP message (terminated by the
/// blank line) is buffered, returning it as text. Test-only.
async fn read_one_request(sock: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        // A request with no body ends at the CRLFCRLF.
        if let Some(pos) = find_header_end(&buf) {
            return String::from_utf8_lossy(&buf[..pos]).to_string();
        }
        let n = sock.read(&mut chunk).await.unwrap();
        assert!(n > 0, "peer closed before a full request");
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn cseq_of(request: &str) -> u32 {
    for line in request.lines() {
        if let Some(rest) = line.strip_prefix("CSeq:") {
            return rest.trim().parse().unwrap();
        }
    }
    panic!("no CSeq in request: {request}");
}
