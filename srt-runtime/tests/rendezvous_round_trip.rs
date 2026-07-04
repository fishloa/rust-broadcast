//! Full in-memory Rendezvous handshake — `draft-sharabayko-srt-01` §4.3.2,
//! curated at `specs/rules/srt-rendezvous.md`. Wires two
//! [`RendezvousHandshake`] peers together with no sockets and no bytes ever
//! touching a real network: every emitted [`HandshakeOutput::Send`] from one
//! side is parsed with the existing [`ControlPacket`] codec and fed straight
//! to the other side, round by round, until both reach `Connected`.
//!
//! This is the "must bite" test for issue #609, mirroring the Caller-Listener
//! `tests/handshake_round_trip.rs`: it proves the two (identical, symmetric)
//! engines actually agree on a real cookie contest outcome — one becomes
//! Initiator, the other Responder — and land on cross-matching negotiated
//! parameters, not just that each one runs without erroring in isolation.

use srt_runtime::handshake_sm::{HandshakeOutput, RejectionReason};
use srt_runtime::packet::{
    ControlPacket, EncryptionField, HandshakeExtensionFlags, HandshakePacket, HandshakeType,
};
use srt_runtime::rendezvous::{RendezvousHandshake, RendezvousHandshakeState};
use srt_runtime::{HandshakeConfig, RendezvousRole};

const PEER_A_SOCKET_ID: u32 = 0x1111_1111;
const PEER_B_SOCKET_ID: u32 = 0x2222_2222;
/// Greater than `PEER_B_COOKIE` — A wins the cookie contest and becomes
/// Initiator (`draft-sharabayko-srt-01` §4.3.2, L2133-2135).
const PEER_A_COOKIE: u32 = 0xAAAA_0002;
const PEER_B_COOKIE: u32 = 0x1111_0001;

/// Drives `a` and `b` to completion by delivering every `HandshakeOutput::Send`
/// each side emits to the other, one synchronous round at a time (both
/// sides' round-N outputs are delivered before round N+1 is processed).
/// Panics (failing the test) if both sides never reach
/// [`RendezvousHandshakeState::Connected`] within a small number of rounds.
fn run_rendezvous(a: &mut RendezvousHandshake, b: &mut RendezvousHandshake) {
    let mut to_a: Vec<Vec<u8>> = vec![b.start().expect("b start")];
    let mut to_b: Vec<Vec<u8>> = vec![a.start().expect("a start")];

    for _ in 0..20 {
        if a.state() == RendezvousHandshakeState::Connected
            && b.state() == RendezvousHandshakeState::Connected
        {
            return;
        }

        let mut next_to_a = Vec::new();
        let mut next_to_b = Vec::new();

        for msg in to_a.drain(..) {
            let pkt = ControlPacket::parse(&msg).expect("valid wire bytes fed to a");
            for out in a.feed(&pkt).expect("a.feed") {
                if let HandshakeOutput::Send(bytes) = out {
                    next_to_b.push(bytes);
                }
            }
        }
        for msg in to_b.drain(..) {
            let pkt = ControlPacket::parse(&msg).expect("valid wire bytes fed to b");
            for out in b.feed(&pkt).expect("b.feed") {
                if let HandshakeOutput::Send(bytes) = out {
                    next_to_a.push(bytes);
                }
            }
        }

        to_a = next_to_a;
        to_b = next_to_b;

        if to_a.is_empty() && to_b.is_empty() {
            break;
        }
    }
}

#[test]
fn two_rendezvous_peers_converge_on_connected_with_matching_params() {
    let config_a = HandshakeConfig {
        latency_ms: 180,
        ..HandshakeConfig::default()
    };
    let config_b = HandshakeConfig {
        latency_ms: 250,
        ..HandshakeConfig::default()
    };

    let mut a = RendezvousHandshake::new(PEER_A_SOCKET_ID, PEER_A_COOKIE, config_a);
    let mut b = RendezvousHandshake::new(PEER_B_SOCKET_ID, PEER_B_COOKIE, config_b);

    run_rendezvous(&mut a, &mut b);

    assert_eq!(a.state(), RendezvousHandshakeState::Connected);
    assert_eq!(b.state(), RendezvousHandshakeState::Connected);

    // The cookie contest resolved deterministically and oppositely on both
    // sides (`draft-sharabayko-srt-01` §4.3.2, L2133-2135): A's cookie is
    // greater, so A is Initiator and B is Responder.
    assert_eq!(a.role(), Some(RendezvousRole::Initiator));
    assert_eq!(b.role(), Some(RendezvousRole::Responder));

    let a_params = a.negotiated().expect("a negotiated params");
    let b_params = b.negotiated().expect("b negotiated params");

    assert_eq!(a_params.version, 5);
    assert_eq!(b_params.version, 5);

    // Socket IDs cross-match: each side's peer is the other's own id.
    assert_eq!(a_params.own_socket_id, PEER_A_SOCKET_ID);
    assert_eq!(a_params.peer_socket_id, PEER_B_SOCKET_ID);
    assert_eq!(b_params.own_socket_id, PEER_B_SOCKET_ID);
    assert_eq!(b_params.peer_socket_id, PEER_A_SOCKET_ID);

    // The greater-of-both-parties latency rule (reused from §4.3.1.2 — see
    // rendezvous.rs module docs "Resolved ambiguities") lands on the same
    // number regardless of which side computes it.
    assert_eq!(a_params.latency_ms, 250);
    assert_eq!(b_params.latency_ms, 250);
    assert_eq!(a_params.latency_ms, b_params.latency_ms);
}

#[test]
fn cookie_contest_greater_cookie_wins_initiator() {
    let mut a = RendezvousHandshake::new(1, PEER_A_COOKIE, HandshakeConfig::default());
    let mut b = RendezvousHandshake::new(2, PEER_B_COOKIE, HandshakeConfig::default());

    let wave_a = a.start().unwrap();
    let wave_b = b.start().unwrap();

    let pkt_a = ControlPacket::parse(&wave_a).unwrap();
    let pkt_b = ControlPacket::parse(&wave_b).unwrap();

    a.feed(&pkt_b).unwrap();
    b.feed(&pkt_a).unwrap();

    assert_eq!(a.role(), Some(RendezvousRole::Initiator));
    assert_eq!(b.role(), Some(RendezvousRole::Responder));
}

#[test]
fn malformed_packet_fed_mid_flow_yields_structured_error_not_panic() {
    let mut a = RendezvousHandshake::new(1, PEER_A_COOKIE, HandshakeConfig::default());
    a.start().unwrap();

    // Get `a` into Attention (Initiator) first.
    let peer_wave = ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 0,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Wavehand,
        srt_socket_id: 2,
        syn_cookie: PEER_B_COOKIE,
        peer_ip: [0; 4],
        extensions: srt_runtime::packet::HandshakeExtensions(&[]),
    });
    a.feed(&peer_wave).unwrap();
    assert_eq!(a.state(), RendezvousHandshakeState::Attention);

    // A CONCLUSION carrying a declared extension length that overruns the
    // bytes actually present — must be rejected cleanly, never panic.
    let bad_ext: &'static [u8] = &[0x00, 0x01, 0xFF, 0xFF];
    let bad = ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 1,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(1),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Conclusion,
        srt_socket_id: 2,
        syn_cookie: PEER_B_COOKIE,
        peer_ip: [0; 4],
        extensions: srt_runtime::packet::HandshakeExtensions(bad_ext),
    });

    let outputs = a.feed(&bad).expect("feed must not panic or hard-error");
    assert_eq!(
        outputs,
        vec![HandshakeOutput::Rejected(RejectionReason::Rogue)]
    );
    assert_eq!(a.state(), RendezvousHandshakeState::Rejected);

    // Once rejected, further feeds are a structured out-of-sequence error,
    // not a panic.
    let err = a.feed(&bad);
    assert!(err.is_err());
}
