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
            assert!(events
                .iter()
                .any(|e| matches!(e, ServerEvent::RequestAccepted { .. })));
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
        // Frame 1: sent whole.
        srv.send_interleaved(0, &p1).await.unwrap();
        // Frame 2: sent split across two socket writes to exercise reassembly.
        // Grab the raw stream and write a partial header + tail, then the rest.
        let frame = rtsp_runtime::InterleavedFrame::new(0, p2.clone());
        let bytes = frame.to_bytes().unwrap();
        let split = 3; // mid-header split
        srv.stream_mut().write_all(&bytes[..split]).await.unwrap();
        // Small yield so the two writes land in separate reads on the client.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        srv.stream_mut().write_all(&bytes[split..]).await.unwrap();
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
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("server config")
    };

    let client_config = {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(cert_der))
            .expect("add self-signed root");
        rustls::ClientConfig::builder()
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
