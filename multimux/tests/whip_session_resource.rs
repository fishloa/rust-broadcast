#![cfg(all(feature = "whip", feature = "test-hooks"))]
//! Issue #1149 (WHIP): the `Location` of the `201` is a live session resource
//! (RFC 9725 sections 4.2/4.3) — `DELETE` ends the session and frees its
//! capacity slot, `PATCH` trickles ICE candidates — exercised over real HTTP
//! against the real route (`run_whip` driving a real media socket), and over
//! a real ICE + DTLS handshake with a loopback WebRTC peer.
//!
//! The browser-driven WHIP test (`whip_ingest.rs`) is separate and skips
//! without Node/Chromium; nothing in this file needs either.

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

const TRICKLE: &str = "application/trickle-ice-sdpfrag";

/// A parsed HTTP/1.1 response: status code, header map (lower-cased names) and body.
struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Resp {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {text:?}"));
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Resp {
        status,
        headers,
        body: body.to_string(),
    }
}

async fn post_offer(addr: std::net::SocketAddr) -> Resp {
    post_offer_body(addr, OFFER).await
}

async fn post_offer_body(addr: std::net::SocketAddr, body: &str) -> Resp {
    http(
        addr,
        "POST",
        "/whip",
        &[("Content-Type", "application/sdp")],
        body,
    )
    .await
}

/// The resource path from a `201`'s `Location` (asserting its shape).
fn resource_path(resp: &Resp) -> String {
    assert_eq!(resp.status, 201, "{}", resp.body);
    let loc = resp.header("location").expect("201 carries a Location");
    let id = loc
        .strip_prefix("/whip/session/")
        .unwrap_or_else(|| panic!("Location must be a per-session resource: {loc}"));
    assert_eq!(id.len(), 32, "an unguessable 32-char resource id: {loc}");
    assert!(
        id.chars().all(|c| c.is_ascii_alphanumeric()),
        "resource id alphabet: {id}"
    );
    loc.to_string()
}

fn etag_of(resp: &Resp) -> String {
    resp.header("etag")
        .expect("201 carries an ETag")
        .to_string()
}

/// Bounded condition wait (not a sleep).
async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn fragment(ufrag: &str, candidate: &str) -> String {
    format!(
        "a=ice-ufrag:{ufrag}\r\na=ice-pwd:whatever-is-ignored-here\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:0\r\na=candidate:{candidate}\r\n"
    )
}

const CANDIDATE: &str = "2 1 udp 2130706431 10.0.0.9 5555 typ host";

#[tokio::test]
async fn the_201_location_is_a_distinct_resource_per_session() {
    let (addr, route, token) = multimux::source::whip::serve_for_test_with_read_load().await;
    let a = post_offer(addr).await;
    let b = post_offer(addr).await;
    let (la, lb) = (resource_path(&a), resource_path(&b));
    assert_ne!(la, lb, "each session gets its own resource URL");
    assert_ne!(etag_of(&a), etag_of(&b));
    assert_eq!(route.live_resources(), 2);
    token.cancel();
}

/// DELETE ends the session: 200 (RFC 9725 section 4.2 and the repo's WHIP
/// transcription), the capacity slot is reusable, the driver's per-session
/// state is gone, and a second DELETE is 404.
#[tokio::test]
async fn delete_ends_the_session_frees_its_slot_and_is_not_repeatable() {
    let (addr, route, token) =
        multimux::source::whip::serve_for_test_with_read_load_and_max_sessions(1).await;

    let first = post_offer(addr).await;
    let loc = resource_path(&first);
    assert!(route.wait_for_admissions(1, Duration::from_secs(2)).await >= 1);
    assert_eq!(route.active_sessions(), 1);
    assert_eq!(route.live_resources(), 1);
    assert_eq!(route.last_datagram_entries(), 1);

    // At capacity: a second publisher is refused.
    assert_eq!(post_offer(addr).await.status, 503, "cap of 1 is full");

    let del = http(addr, "DELETE", &loc, &[], "").await;
    assert_eq!(
        del.status, 200,
        "DELETE terminates the session: {}",
        del.body
    );

    // The driver ends the session through its release-then-reap path.
    wait_until("capacity slot released", || route.active_sessions() == 0).await;
    wait_until("idle-clock entry dropped", || {
        route.last_datagram_entries() == 0
    })
    .await;
    assert_eq!(route.live_resources(), 0, "the resource is unregistered");

    // Second DELETE: the resource is gone.
    assert_eq!(http(addr, "DELETE", &loc, &[], "").await.status, 404);
    // PATCH on a deleted resource: also gone.
    assert_eq!(
        http(
            addr,
            "PATCH",
            &loc,
            &[("Content-Type", TRICKLE)],
            &fragment("abcd", CANDIDATE)
        )
        .await
        .status,
        404
    );

    // The slot is reusable: the same capacity-1 route admits a new publisher.
    let again = post_offer(addr).await;
    resource_path(&again);
    assert!(route.wait_for_admissions(2, Duration::from_secs(2)).await >= 2);
    token.cancel();
}

#[tokio::test]
async fn unknown_resources_are_404_for_patch_and_delete() {
    let (addr, _route, token) = multimux::source::whip::serve_for_test_with_read_load().await;
    let bogus = "/whip/session/does-not-exist";
    assert_eq!(http(addr, "DELETE", bogus, &[], "").await.status, 404);
    assert_eq!(
        http(
            addr,
            "PATCH",
            bogus,
            &[("Content-Type", TRICKLE)],
            &fragment("abcd", CANDIDATE)
        )
        .await
        .status,
        404
    );
    // The old, id-less resource path no longer exists.
    assert_eq!(
        http(addr, "DELETE", "/whip/session", &[], "").await.status,
        404
    );
    token.cancel();
}

/// PATCH carrying a trickle candidate is 204, and the candidate actually
/// reaches the session's transport (the read loop applied it).
#[tokio::test]
async fn patch_trickles_a_candidate_into_the_live_transport() {
    let (addr, route, token) = multimux::source::whip::serve_for_test_with_read_load().await;
    let created = post_offer(addr).await;
    let loc = resource_path(&created);
    let etag = etag_of(&created);
    assert!(route.wait_for_admissions(1, Duration::from_secs(2)).await >= 1);
    assert_eq!(route.trickled_candidates(), 0);

    let patched = http(
        addr,
        "PATCH",
        &loc,
        &[("Content-Type", TRICKLE), ("If-Match", &etag)],
        &fragment("abcd", CANDIDATE),
    )
    .await;
    assert_eq!(patched.status, 204, "{}", patched.body);
    assert!(patched.body.is_empty());
    wait_until("candidate handed to the transport", || {
        route.trickled_candidates() == 1
    })
    .await;

    // No If-Match is a plain trickle update too (the state machine's rule).
    let again = http(
        addr,
        "PATCH",
        &loc,
        &[("Content-Type", &format!("{TRICKLE}; charset=utf-8"))],
        &fragment("abcd", "3 1 udp 2130706431 10.0.0.10 5556 typ host"),
    )
    .await;
    assert_eq!(again.status, 204);
    wait_until("second candidate applied", || {
        route.trickled_candidates() == 2
    })
    .await;
    token.cancel();
}

#[tokio::test]
async fn patch_rejections_follow_rfc_9725() {
    let (addr, route, token) = multimux::source::whip::serve_for_test_with_read_load().await;
    let created = post_offer(addr).await;
    let loc = resource_path(&created);
    let etag = etag_of(&created);
    let good = fragment("abcd", CANDIDATE);

    // Wrong Content-Type: 415.
    for ct in ["application/sdp", "text/plain"] {
        let r = http(addr, "PATCH", &loc, &[("Content-Type", ct)], &good).await;
        assert_eq!(r.status, 415, "{ct}");
    }
    // No Content-Type at all: 415.
    assert_eq!(http(addr, "PATCH", &loc, &[], &good).await.status, 415);

    // If-Match naming another entity-tag: 412.
    let r = http(
        addr,
        "PATCH",
        &loc,
        &[("Content-Type", TRICKLE), ("If-Match", "\"not-this-one\"")],
        &good,
    )
    .await;
    assert_eq!(r.status, 412);

    // If-Match: * is an ICE restart, which this transport cannot do: 422.
    let r = http(
        addr,
        "PATCH",
        &loc,
        &[("Content-Type", TRICKLE), ("If-Match", "*")],
        &good,
    )
    .await;
    assert_eq!(r.status, 422);

    // A fragment for another ICE generation, an empty fragment, and a garbage
    // body are 400.
    let other_gen = fragment("zzzz", CANDIDATE);
    for body in [other_gen.as_str(), "a=ice-pwd:x\r\n", "this is not sdp"] {
        let r = http(
            addr,
            "PATCH",
            &loc,
            &[("Content-Type", TRICKLE), ("If-Match", &etag)],
            body,
        )
        .await;
        assert_eq!(r.status, 400, "{body:?}");
    }

    // None of the rejected requests reached the transport, and the session
    // is still alive and deletable.
    assert_eq!(route.trickled_candidates(), 0);
    assert_eq!(route.active_sessions(), 1);
    assert_eq!(http(addr, "DELETE", &loc, &[], "").await.status, 200);
    token.cancel();
}

/// A resource whose session ended on its own (idle reap) is not a stale 200.
#[tokio::test]
async fn a_session_reaped_by_idle_timeout_unregisters_its_resource() {
    let (addr, route, token) = multimux::source::whip::serve_for_test_with_read_load_and_timeout(
        Duration::from_millis(150),
    )
    .await;
    let created = post_offer(addr).await;
    let loc = resource_path(&created);
    wait_until("idle reap", || {
        route.active_sessions() == 0 && route.live_resources() == 0
    })
    .await;
    assert_eq!(http(addr, "DELETE", &loc, &[], "").await.status, 404);
    token.cancel();
}

// ---- A real WebRTC peer: ICE + DTLS to the live media socket, then DELETE --

struct ServerSide {
    ufrag: String,
    pwd: String,
    fingerprint: String,
    candidate: String,
    media_addr: std::net::SocketAddr,
}

fn parse_answer(body: &str) -> ServerSide {
    let attr = |name: &str| -> String {
        body.lines()
            .find_map(|l| l.strip_prefix(name))
            .unwrap_or_else(|| panic!("answer has {name}"))
            .trim_end_matches('\r')
            .to_string()
    };
    let candidate = attr("a=candidate:");
    let port: u16 = candidate
        .split_whitespace()
        .nth(5)
        .and_then(|p| p.parse().ok())
        .expect("candidate port");
    ServerSide {
        ufrag: attr("a=ice-ufrag:"),
        pwd: attr("a=ice-pwd:"),
        fingerprint: attr("a=fingerprint:"),
        candidate,
        media_addr: format!("127.0.0.1:{port}").parse().unwrap(),
    }
}

async fn drive_client_until_srtp_ready(
    media: &mut webrtc_runtime::media::MediaTransport,
    client: &std::net::UdpSocket,
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
        assert!(
            tokio::time::Instant::now() < budget,
            "loopback DTLS handshake did not complete within budget"
        );
        while let Some(d) = media.poll_transmit() {
            client.send_to(&d.bytes, d.peer).unwrap();
        }
        let wake = media
            .poll_timeout()
            .map(tokio::time::Instant::from_std)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_millis(10));
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

/// The full flow with a real peer: POST (offer carries the peer's genuine DTLS
/// fingerprint), ICE + DTLS to the live media socket, a trickle PATCH on the
/// established session, then DELETE ends it.
#[tokio::test]
async fn a_real_peer_session_is_trickled_and_deleted() {
    use rtc_dtls::crypto::Certificate;
    use rtc_dtls::crypto_provider::default_provider;
    use webrtc_runtime::media::{
        MAX_REMOTE_CANDIDATES, MediaTransport, MediaTransportConfig, SetupRole,
        certificate_fingerprint,
    };

    let (addr, route, token) = multimux::source::whip::serve_for_test_with_read_load().await;

    let provider = default_provider().expect("crypto provider");
    let client_cert =
        Certificate::generate_self_signed(vec!["localhost".to_string()], provider.crypto())
            .expect("client certificate");
    let client_fp = certificate_fingerprint(&client_cert);
    let client_ufrag = "clientuf".to_string();
    let client_pwd = "clientpwd000000000000000000".to_string();
    let offer = OFFER
        .replace("a=ice-ufrag:abcd", &format!("a=ice-ufrag:{client_ufrag}"))
        .replace(
            "a=ice-pwd:abcdefghijklmnopqrstuvwx",
            &format!("a=ice-pwd:{client_pwd}"),
        )
        .replace(
            "sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff",
            &format!("sha-256 {client_fp}"),
        );
    let created = post_offer_body(addr, &offer).await;
    let loc = resource_path(&created);
    let etag = etag_of(&created);
    assert!(route.wait_for_admissions(1, Duration::from_secs(2)).await >= 1);

    let server = parse_answer(&created.body);
    let client_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let client_addr = client_socket.local_addr().unwrap();
    client_socket.set_nonblocking(true).unwrap();
    let mut client_media = MediaTransport::with_certificate_for_test(
        MediaTransportConfig {
            local_addr: client_addr,
            local_ice_ufrag: client_ufrag.clone(),
            local_ice_pwd: client_pwd,
            remote_ice_ufrag: server.ufrag,
            remote_ice_pwd: server.pwd,
            is_controlling: true,
            local_setup: SetupRole::Active,
            stun_server: None,
            remote_fingerprint: server.fingerprint,
            max_remote_candidates: MAX_REMOTE_CANDIDATES,
        },
        client_cert,
        std::time::Instant::now(),
    )
    .expect("client transport");
    client_media
        .add_remote_candidate(&server.candidate)
        .unwrap();
    drive_client_until_srtp_ready(&mut client_media, &client_socket).await;
    assert_eq!(server.media_addr.ip().to_string(), "127.0.0.1");

    // Trickle on the established session.
    let patched = http(
        addr,
        "PATCH",
        &loc,
        &[("Content-Type", TRICKLE), ("If-Match", &etag)],
        &fragment(&client_ufrag, CANDIDATE),
    )
    .await;
    assert_eq!(patched.status, 204);
    wait_until("trickled candidate applied on a live session", || {
        route.trickled_candidates() == 1
    })
    .await;

    // DELETE the established session: it ends and releases everything.
    assert_eq!(route.active_sessions(), 1);
    assert_eq!(http(addr, "DELETE", &loc, &[], "").await.status, 200);
    wait_until("established session ended", || {
        route.active_sessions() == 0 && route.last_datagram_entries() == 0
    })
    .await;
    assert_eq!(route.live_resources(), 0);
    assert_eq!(http(addr, "DELETE", &loc, &[], "").await.status, 404);
    token.cancel();
}
