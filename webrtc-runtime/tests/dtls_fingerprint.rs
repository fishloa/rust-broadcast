//! DTLS peer-certificate fingerprint verification (RFC 8122 §5, RFC 5764
//! §5 "Identity Checks").
//!
//! `MediaTransportConfig::remote_fingerprint` carries the remote SDP's
//! `a=fingerprint` value; the DTLS handshake must fail unless the peer's
//! leaf certificate hashes to exactly that digest. These tests drive two
//! real `MediaTransport`s against each other in-process (the sans-IO
//! pump: drain one side's `poll_transmit`, feed it to the other's
//! `handle_datagram` with the first side's address as the source, and run
//! both timers) — the same shape a caller like `multimux`'s WHIP/WHEP
//! routes drives in production over real sockets.
//!
//! The two tests that need *both* sides to hold each other's genuine
//! fingerprint (`dtls_accepts_matching_fingerprint`,
//! `dtls_from_non_selected_address_is_ignored`) live as unit tests in
//! `src/media/transport.rs` instead: a certificate is generated inside
//! `MediaTransport::new`, so via the public API alone side A can only ever
//! learn B's fingerprint by constructing B, and vice versa — one side's
//! value is necessarily stale. The unit tests break that cycle through
//! same-module access to the stored digest; everything observable from
//! outside (rejection of a wrong fingerprint, malformed fingerprints at
//! `new`, DTLS dropped before/from non-selected addresses) is here.

#![cfg(feature = "media")]

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use webrtc_runtime::media::{
    MediaEvent, MediaTransport, MediaTransportConfig, SetupRole, parse_remote_fingerprint,
};

/// A syntactically valid SHA-256 SDP fingerprint value (RFC 8122 §5: the
/// `sha-256` token followed by 32 colon-separated hex bytes) that matches
/// none of the ephemeral self-signed certificates these tests generate.
const WRONG_FP: &str = "sha-256 \
11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:11:\
11:11:11:11:11:11:11";

/// RFC 5764 §5.1.2's DTLS first-byte band, used here only to count that a
/// handshake was genuinely attempted (rather than ICE stalling).
fn is_dtls_datagram(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|&b| (20..=63).contains(&b))
}

/// "Reserve then drop, hand the exact address to the thing that binds it"
/// — both transports advertise these as their ICE host candidates, and each
/// side delivers datagrams to the other with this as the source address.
fn reserve_udp_addr() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("reserve udp port");
    let addr = socket.local_addr().expect("local addr");
    drop(socket);
    addr
}

fn config(
    local_addr: SocketAddr,
    local_ufrag: &str,
    local_pwd: &str,
    remote_ufrag: &str,
    remote_pwd: &str,
    is_controlling: bool,
    local_setup: SetupRole,
) -> MediaTransportConfig {
    MediaTransportConfig {
        local_addr,
        local_ice_ufrag: local_ufrag.into(),
        local_ice_pwd: local_pwd.into(),
        remote_ice_ufrag: remote_ufrag.into(),
        remote_ice_pwd: remote_pwd.into(),
        is_controlling,
        local_setup,
        stun_server: None,
        // Every test here either checks the wrong-fingerprint rejection or
        // overwrites the parsed digest (unit tests); no integration test
        // needs a *matching* fingerprint — see the module doc for why that
        // is impossible through the public API alone.
        remote_fingerprint: WRONG_FP.to_string(),
    }
}

/// The candidate-attribute body `rtc_ice::candidate::unmarshal_candidate`
/// expects (the SDP line minus its `a=` prefix).
fn host_candidate(addr: SocketAddr) -> String {
    format!("1 1 udp 2130706431 {} {} typ host", addr.ip(), addr.port())
}

/// What one [`pump`] run observed.
struct Pumped {
    a_events: Vec<MediaEvent>,
    b_events: Vec<MediaEvent>,
    /// DTLS-band datagrams delivered A -> B and B -> A (a handshake was at
    /// least *attempted* if both are non-zero).
    dtls_a_to_b: usize,
    dtls_b_to_a: usize,
    /// `handle_datagram` failures along the way — a rejected fingerprint
    /// surfaces as one side's fatal alert making the other's DTLS read
    /// fail, which is expected and must not abort the pump.
    errors: Vec<String>,
}

fn is_handshake_complete(event: &MediaEvent) -> bool {
    matches!(event, MediaEvent::DtlsHandshakeComplete)
}

/// Drive both transports against each other for at most `budget` of wall
/// clock (rtc-ice's own nomination waits read the real clock), stopping
/// early once either side reports a completed DTLS handshake.
fn pump(
    a: &mut MediaTransport,
    b: &mut MediaTransport,
    a_addr: SocketAddr,
    b_addr: SocketAddr,
    budget: Duration,
) -> Pumped {
    let mut pumped = Pumped {
        a_events: Vec::new(),
        b_events: Vec::new(),
        dtls_a_to_b: 0,
        dtls_b_to_a: 0,
        errors: Vec::new(),
    };
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        let now = Instant::now();
        let mut progressed = false;
        while let Some(dgram) = a.poll_transmit() {
            if is_dtls_datagram(&dgram.bytes) {
                pumped.dtls_a_to_b += 1;
            }
            match b.handle_datagram(now, a_addr, &dgram.bytes) {
                Ok(events) => pumped.b_events.extend(events),
                Err(e) => pumped.errors.push(format!("B: {e}")),
            }
            progressed = true;
        }
        while let Some(dgram) = b.poll_transmit() {
            if is_dtls_datagram(&dgram.bytes) {
                pumped.dtls_b_to_a += 1;
            }
            match a.handle_datagram(now, b_addr, &dgram.bytes) {
                Ok(events) => pumped.a_events.extend(events),
                Err(e) => pumped.errors.push(format!("A: {e}")),
            }
            progressed = true;
        }
        a.handle_timeout(now);
        b.handle_timeout(now);
        if pumped.a_events.iter().any(is_handshake_complete)
            || pumped.b_events.iter().any(is_handshake_complete)
        {
            return pumped;
        }
        if !progressed {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pumped
}

/// A (DTLS client + ICE controlling) configured with a `remote_fingerprint`
/// that is NOT B's certificate digest: the handshake must never complete on
/// A, and no SRTP may decrypt there. Pre-fix this failed by *completing*:
/// `with_insecure_skip_verify(true)` with no verifier accepted any peer
/// certificate, so A reached `DtlsHandshakeComplete` and installed keys.
#[test]
fn dtls_rejects_peer_with_wrong_fingerprint() {
    let a_addr = reserve_udp_addr();
    let b_addr = reserve_udp_addr();
    let mut a = MediaTransport::new(config(
        a_addr,
        "afp0ufrag",
        "a-fp-test-ice-password-00000",
        "bfp0ufrag",
        "b-fp-test-ice-password-00000",
        true,
        SetupRole::Active,
    ))
    .expect("build A");
    let mut b = MediaTransport::new(config(
        b_addr,
        "bfp0ufrag",
        "b-fp-test-ice-password-00000",
        "afp0ufrag",
        "a-fp-test-ice-password-00000",
        false,
        SetupRole::Passive,
    ))
    .expect("build B");
    a.add_remote_candidate(&host_candidate(b_addr))
        .expect("A add B candidate");
    b.add_remote_candidate(&host_candidate(a_addr))
        .expect("B add A candidate");

    let pumped = pump(&mut a, &mut b, a_addr, b_addr, Duration::from_secs(3));

    // The handshake was genuinely attempted — DTLS datagrams flowed both
    // ways (so this is not ICE stalling and calling that "rejection").
    assert!(pumped.dtls_a_to_b > 0, "A must have sent its ClientHello");
    assert!(
        pumped.dtls_b_to_a > 0,
        "B must have answered with its certificate flight"
    );

    // ... but A rejected B's certificate: no handshake completion on either
    // side (A aborts before its Finished, so B cannot complete either).
    assert!(
        !pumped.a_events.iter().any(is_handshake_complete),
        "A must never report DtlsHandshakeComplete for a peer whose \
         fingerprint does not match remote_fingerprint"
    );
    assert!(
        !pumped.b_events.iter().any(is_handshake_complete),
        "B cannot complete once A aborted the handshake with a fatal alert"
    );
    assert!(
        pumped.errors.iter().any(|e| e.contains("dtls")),
        "the rejected handshake must surface as a DTLS failure, got {:?}",
        pumped.errors
    );

    // And no SRTP decrypts on A: it has no read context, so an inbound RTP
    // datagram produces no event (and `encrypt_rtp` still errors).
    let events = a
        .handle_datagram(
            Instant::now(),
            b_addr,
            &[0x80, 96, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3],
        )
        .expect("RTP before any handshake is dropped silently");
    assert!(events.is_empty(), "no SRTP may decrypt on A: {events:?}");
}

/// `MediaTransport::new` must reject a malformed `remote_fingerprint` up
/// front rather than build a transport that can never be verified: a
/// non-`sha-256` hash token, too few hex bytes, and the empty string.
#[test]
fn new_rejects_malformed_fingerprint() {
    for bad in ["md5 00:11", "sha-256 00:11", ""] {
        let mut cfg = config(
            reserve_udp_addr(),
            "ufrag0aaaa",
            "icetestpasswordaaaaaaaaaaaa",
            "ufrag0bbbb",
            "icetestpasswordbbbbbbbbbbbb",
            false,
            SetupRole::Passive,
        );
        cfg.remote_fingerprint = bad.to_string();
        match MediaTransport::new(cfg) {
            Err(webrtc_runtime::Error::Media(_)) => {}
            Err(other) => panic!("expected Error::Media for {bad:?}, got {other:?}"),
            Ok(_) => panic!("remote_fingerprint {bad:?} must be rejected by new()"),
        }
    }
}

/// `parse_remote_fingerprint` returns the media-level `a=fingerprint` value
/// when one is present, else the session-level one, else `None`.
#[test]
fn parse_remote_fingerprint_reads_media_then_session_level() {
    let session_fp = "sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA";
    let media_fp = "sha-256 BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB:BB";

    let both = format!(
        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\na=fingerprint:{session_fp}\r\n\
         m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=fingerprint:{media_fp}\r\n"
    );
    assert_eq!(parse_remote_fingerprint(&both).as_deref(), Some(media_fp));

    let session_only = format!(
        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\na=fingerprint:{session_fp}\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n"
    );
    assert_eq!(
        parse_remote_fingerprint(&session_only).as_deref(),
        Some(session_fp)
    );

    let none = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:0\r\n";
    assert_eq!(parse_remote_fingerprint(none), None);
}
