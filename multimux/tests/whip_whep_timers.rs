#![cfg(all(feature = "whip", feature = "test-hooks"))]
//! Defects 1 and 2 (WHIP): the media driver has a transport-deadline timer
//! arm that never reaps, and a second publisher is admitted while a first
//! reads continuously.

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n";

async fn post_offer(addr: std::net::SocketAddr) -> String {
    post_offer_with_body(addr, OFFER).await
}

async fn post_offer_with_body(addr: std::net::SocketAddr, body: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let raw = format!(
        "POST /whip HTTP/1.1\r\nHost: x\r\nContent-Type: application/sdp\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(raw.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}

/// Defect 1 (structural): the driver has a transport-deadline timer arm that
/// fires `handle_timeout` while the session lives (never as a reap). The
/// full handshake this needs is exercised by `whip_ingest` (a real browser),
/// where the timer arm is the only way ICE/DTLS retransmits fire during an
/// active stream. This test pins the wiring: a freshly-admitted quiet session
/// holds its slot and exposes the timer-fires counter.
#[tokio::test]
async fn a_quiet_session_still_holds_its_slot_and_exposes_the_timer_counter() {
    let (addr, route, token) = multimux::source::whip::serve_for_test().await;

    let resp = post_offer(addr).await;
    assert!(
        resp.starts_with("HTTP/1.1 201"),
        "signalling must succeed: {resp}"
    );
    assert_eq!(route.active_sessions(), 1, "one session admitted");
    assert_eq!(route.timer_fires(), 0, "no timer has fired yet");
    token.cancel();
}

/// Defect 2 (WHIP): a second publisher must be admitted while the first
/// session's read future is continuously ready. `serve_for_test_with_read_load`
/// runs the media driver, and publisher A keeps its media socket busy with a
/// steady trickle of (rejected) datagrams, so the old 20 ms sleep-poll arm
/// would be starved; publisher B is still admitted. The assertion observes the
/// DRIVER side (`admitted_total` — incremented by `poll_accept`, not the HTTP
/// handler), so reverting the I2 admit-drain fix to the sleep-poll would leave
/// it at 0 and fail even though the 201 and `active_sessions` still read 2.
#[tokio::test]
async fn a_second_whip_publisher_is_admitted_while_a_first_reads_continuously() {
    use std::net::UdpSocket as StdUdp;
    let (addr, route, token) = multimux::source::whip::serve_for_test_with_read_load().await;

    // Publisher A.
    let resp = post_offer(addr).await;
    assert!(resp.starts_with("HTTP/1.1 201"), "publisher A: {resp}");
    // Wait for the DRIVER to actually admit publisher A before starting the
    // read-load (not a fixed sleep — a bounded condition wait).
    let admitted_a = route.wait_for_admissions(1, Duration::from_secs(2)).await;
    assert!(admitted_a >= 1, "the driver must admit publisher A");

    // Publisher A's media socket port, from the SDP answer's host candidate.
    let media_port: u16 = resp
        .lines()
        .find_map(|l| l.strip_prefix("a=candidate:"))
        .and_then(|cand| cand.split_whitespace().nth(5))
        .and_then(|p| p.parse().ok())
        .expect("the answer must carry a host candidate with a port");

    // A steady trickle of garbage datagrams to A's media socket keeps its
    // read future continuously ready (each is rejected, but `recv_from`
    // completes and the read is re-armed) — the load that starved the old
    // sleep-poll arm.
    let drivel = tokio::spawn(async move {
        let slog = StdUdp::bind("127.0.0.1:0").unwrap();
        let mut i = 0u8;
        loop {
            let _ = slog.send_to(&[i; 8], ("127.0.0.1", media_port));
            i = i.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // Publisher B must still be admitted within a bound far tighter than the
    // starved old loop.
    let resp = post_offer(addr).await;
    drivel.abort();
    assert!(
        resp.starts_with("HTTP/1.1 201"),
        "the second publisher must be admitted: {resp}"
    );
    // The DRIVER must admit BOTH publishers (this is what defect 2 asserts;
    // `active_sessions` alone is the handler's count and would not bite).
    let admitted_b = route.wait_for_admissions(2, Duration::from_secs(2)).await;
    assert!(
        admitted_b >= 2,
        "the driver must admit both publishers under a steady read (only {admitted_b}/2)"
    );
    assert_eq!(route.active_sessions(), 2, "both publishers admitted");
    token.cancel();
}

/// Buffers the SDP answer's media-level attribute values the loopback client
/// needs to build its transport against the live server: ICE ufrag/pwd, the
/// server's DTLS fingerprint, and its host candidate body.
struct ServerSide {
    ufrag: String,
    pwd: String,
    fingerprint: String,
    candidate: String,
    media_addr: std::net::SocketAddr,
}

fn parse_answer(resp: &str) -> ServerSide {
    let ufrag = resp
        .lines()
        .find_map(|l| l.strip_prefix("a=ice-ufrag:"))
        .expect("answer ice-ufrag")
        .trim_end_matches('\r')
        .to_string();
    let pwd = resp
        .lines()
        .find_map(|l| l.strip_prefix("a=ice-pwd:"))
        .expect("answer ice-pwd")
        .trim_end_matches('\r')
        .to_string();
    let fingerprint = resp
        .lines()
        .find_map(|l| l.strip_prefix("a=fingerprint:"))
        .expect("answer fingerprint")
        .trim_end_matches('\r')
        .to_string();
    let candidate = resp
        .lines()
        .find_map(|l| l.strip_prefix("a=candidate:"))
        .expect("answer host candidate")
        .trim_end_matches('\r')
        .to_string();
    let port: u16 = candidate
        .split_whitespace()
        .nth(5)
        .and_then(|p| p.parse().ok())
        .expect("answer host candidate has a port");
    ServerSide {
        ufrag,
        pwd,
        fingerprint,
        candidate,
        media_addr: format!("127.0.0.1:{port}").parse().expect("media addr"),
    }
}

/// Drives the client-side transport through ICE connectivity checks and the
/// DTLS handshake against the server's live media socket: relay whatever the
/// client wants sent, feed it whatever the server sent back, and fire its own
/// timers, until it has a write SRTP context (`encrypt_rtp` succeeds) or the
/// wall-clock budget runs out. Returns once ready.
async fn drive_client_until_srtp_ready(
    media: &mut webrtc_runtime::media::MediaTransport,
    client: &std::net::UdpSocket,
    _server_media: std::net::SocketAddr,
) {
    let budget = tokio::time::Instant::now() + Duration::from_secs(15);
    let probe = rtp_packet::RtpPacket {
        marker: false,
        payload_type: 96,
        sequence_number: 0,
        timestamp: 0,
        ssrc: 0x1234,
        csrc: vec![],
        extension: None,
        padding: None,
        payload: &[],
    };
    let mut buf = [0u8; 2048];
    loop {
        if media.encrypt_rtp(&probe).is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= budget {
            panic!("loopback DTLS handshake did not complete within budget");
        }
        // Drain outbound (STUN/DTLS) datagrams to the server.
        while let Some(dgram) = media.poll_transmit() {
            client.send_to(&dgram.bytes, dgram.peer).unwrap();
        }
        // Next wake: inbound datagram or the transport's own timer.
        let wake = media
            .poll_timeout()
            .map(tokio::time::Instant::from_std)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_millis(10));
        // Non-blocking receive; if nothing is pending, drive the timer at `wake`.
        loop {
            match client.recv_from(&mut buf) {
                Ok((n, peer)) => {
                    media
                        .handle_datagram(std::time::Instant::now(), peer, &buf[..n])
                        .unwrap();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("client recv: {e}"),
            }
        }
        tokio::time::sleep_until(wake).await;
        media.handle_timeout(std::time::Instant::now());
    }
}

/// Minor review finding: a reaped session drops its `last_datagram` (idle
/// clock) entry, so the driver's idle-clock map does not leak one entry per
/// reaped session. Here a REAL loopback WebRTC peer (same crate's media
/// transport in the active role) completes ICE + DTLS and sends one SRTP
/// packet; the driver decrypts it into the `Events` path, whose feed advances
/// past a short handshake deadline and reaps the session through the exact
/// `if reaped { last_datagram.remove(id) }` line — the read-timeout reap path
/// already removed its entry, so a test that reaped via the idle timeout alone
/// would not bite on that line.
#[tokio::test]
async fn a_reaped_session_drops_its_idle_clock_entry() {
    use rtc_dtls::crypto::Certificate;
    use rtc_dtls::crypto_provider::default_provider;
    use webrtc_runtime::media::{
        MAX_REMOTE_CANDIDATES, MediaTransport, MediaTransportConfig, SetupRole,
        certificate_fingerprint,
    };

    let (addr, route, token) =
        multimux::source::whip::serve_for_test_with_read_load_timeout_and_policy(
            // A long read timeout: this session must be reaped by the
            // handshake deadline via the Events path, NOT the idle timeout.
            Duration::from_secs(60),
            // A handshake deadline that has long passed by the time media
            // flows: the first decrypted feed is at `now >= establish_by`
            // (nanoseconds since admission), so it reaps immediately.
            Duration::from_millis(1),
        )
        .await;

    // Pre-generate the client's DTLS certificate, so its genuine fingerprint
    // can be signalled in the offer BEFORE the transport is built (the server
    // verifies the peer leaf against that exact fingerprint).
    let provider = default_provider().expect("crypto provider");
    let client_cert =
        Certificate::generate_self_signed(vec!["localhost".to_string()], provider.crypto())
            .expect("client certificate");
    let client_fp = certificate_fingerprint(&client_cert);

    // The client's local ICE credentials — also written into the offer, since
    // the server reads the offer's ufrag/pwd as ITS remote (the client's).
    let client_ufrag = "clientuf".to_string();
    let client_pwd = "clientpwd000000000000000000".to_string();

    // POST an offer naming the client's real fingerprint + credentials.
    let offer = format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         t=0 0\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
         c=IN IP4 0.0.0.0\r\n\
         a=ice-ufrag:{client_ufrag}\r\n\
         a=ice-pwd:{client_pwd}\r\n\
         a=fingerprint:sha-256 {client_fp}\r\n\
         a=setup:actpass\r\n\
         a=mid:0\r\n\
         a=rtcp-mux\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n"
    );
    let resp = post_offer_with_body(addr, &offer).await;
    assert!(resp.starts_with("HTTP/1.1 201"), "publisher: {resp}");

    // Wait for the DRIVER to admit the publisher before touching its media
    // socket (a condition wait, not a sleep).
    let admitted = route.wait_for_admissions(1, Duration::from_secs(2)).await;
    assert!(admitted >= 1, "the driver must admit the publisher");
    assert_eq!(
        route.last_datagram_entries(),
        1,
        "one idle-clock entry held"
    );

    // Parse the answer, then build the client transport against the server's
    // real credentials, fingerprint and host candidate.
    let server = parse_answer(&resp);
    let client_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let client_addr = client_socket.local_addr().unwrap();
    // Make the socket non-blocking so the driver loop can poll it.
    client_socket.set_nonblocking(true).unwrap();

    let mut client_media = MediaTransport::with_certificate_for_test(
        MediaTransportConfig {
            local_addr: client_addr,
            local_ice_ufrag: client_ufrag,
            local_ice_pwd: client_pwd,
            remote_ice_ufrag: server.ufrag.clone(),
            remote_ice_pwd: server.pwd.clone(),
            is_controlling: true,
            local_setup: SetupRole::Active,
            stun_server: None,
            remote_fingerprint: server.fingerprint.clone(),
            max_remote_candidates: MAX_REMOTE_CANDIDATES,
        },
        client_cert,
        std::time::Instant::now(),
    )
    .expect("client transport");
    client_media
        .add_remote_candidate(&server.candidate)
        .expect("add server host candidate");

    // Drive ICE + DTLS to completion (client now has an SRTP write context).
    drive_client_until_srtp_ready(&mut client_media, &client_socket, server.media_addr).await;

    // Send one SRTP-protected RTP packet: the server decrypts it, feeds the
    // session, and the short handshake deadline reaps it through the Events
    // path, dropping its idle-clock entry.
    let rtp = rtp_packet::RtpPacket {
        marker: false,
        payload_type: 96,
        sequence_number: 1,
        timestamp: 3000,
        ssrc: 0x1234,
        csrc: vec![],
        extension: None,
        padding: None,
        payload: &[],
    };
    let protected = client_media.encrypt_rtp(&rtp).expect("encrypt RTP");
    client_socket
        .send_to(&protected, server.media_addr)
        .unwrap();

    // Bounded condition wait for the reap to drain the entry (not a sleep).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if route.active_sessions() == 0 && route.last_datagram_entries() == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "a session reaped through the Events path must drop its idle-clock entry: \
                 active_sessions={}, last_datagram_entries={}",
                route.active_sessions(),
                route.last_datagram_entries()
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    token.cancel();
}

/// Minor review finding (N3, WHIP session-end): a fatal transport
/// `MediaEvent::TimerError` surfaced through `read_one` ends the session (the
/// `ReadOutcome::TimerError` arm in `run_whip`), rather than leaving a stuck
/// session to idle out. Deleting that arm's reap leaves the session running
/// past the bounded wait, so this test fails — it bites on the session-end.
#[tokio::test]
async fn a_fatal_transport_timer_error_ends_the_whip_session() {
    // A long read timeout + unbounded handshake deadline: neither the idle
    // reap nor the handshake deadline may end this session, only the staged
    // timer error.
    let (addr, route, token) =
        multimux::source::whip::serve_for_test_with_read_load_timeout_and_policy(
            Duration::from_secs(60),
            Duration::from_secs(10_000),
        )
        .await;

    // Stage a fatal timer error on the next admitted session's transport.
    route.force_next_timer_error("dtls handle_timeout: boom");

    // Admit a publisher (the staged error is applied to its transport).
    let resp = post_offer(addr).await;
    assert!(resp.starts_with("HTTP/1.1 201"), "publisher: {resp}");

    // Wait for the DRIVER to admit it and the staged timer error to end it
    // (a bounded condition wait, not a sleep).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if route.active_sessions() == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "a fatal transport timer error must end the session: active_sessions={}",
                route.active_sessions()
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    token.cancel();
}
