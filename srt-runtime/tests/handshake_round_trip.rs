//! Full in-memory Caller<->Listener handshake — `draft-sharabayko-srt-01`
//! §4.3.1 (Caller-Listener Handshake). Wires [`CallerHandshake`] and
//! [`ListenerHandshake`] together with no sockets and no bytes ever touching
//! a real network: each side's `HandshakeOutput::Send` bytes are parsed with
//! the existing [`ControlPacket`] codec and fed straight to the other side.
//!
//! This is the "must bite" test from issue #598: it proves the two state
//! machines actually agree — same negotiated version/latency/stream id, and
//! each side's `peer_socket_id` matches the other's `own_socket_id` — not
//! just that each one runs without erroring in isolation.

use srt_runtime::HandshakeConfig;
use srt_runtime::caller::{CallerHandshake, CallerHandshakeState};
use srt_runtime::handshake_sm::{HandshakeOutput, RejectionReason};
use srt_runtime::listener::{ListenerHandshake, ListenerHandshakeState};
use srt_runtime::packet::{ControlPacket, GroupFlags, GroupMembershipExtension, GroupType};

const CALLER_SOCKET_ID: u32 = 0x1111_1111;
const LISTENER_SOCKET_ID: u32 = 0x2222_2222;
const SYN_COOKIE: u32 = 0xC0FF_EE42;

/// Drives `caller` and `listener` to completion by ping-ponging
/// [`HandshakeOutput::Send`] bytes between them, starting from the Caller's
/// INDUCTION. Panics (failing the test) if either side never reaches a
/// terminal state within a small number of round trips.
fn run_handshake(caller: &mut CallerHandshake, listener: &mut ListenerHandshake) {
    let mut next = caller.start().expect("caller start");

    for _ in 0..10 {
        if caller.state() != CallerHandshakeState::Idle
            && caller.state() != CallerHandshakeState::AwaitingInductionResponse
            && caller.state() != CallerHandshakeState::AwaitingConclusionResponse
        {
            // Caller reached a terminal state; nothing left to feed forward.
            return;
        }

        let pkt = ControlPacket::parse(&next).expect("valid wire bytes from caller");
        let listener_outputs = listener.feed(&pkt).expect("listener feed");
        let mut listener_bytes = None;
        for out in listener_outputs {
            if let HandshakeOutput::Send(bytes) = out {
                listener_bytes = Some(bytes);
            }
        }
        let Some(bytes) = listener_bytes else {
            return; // Listener rejected/terminated with no further bytes.
        };

        let pkt = ControlPacket::parse(&bytes).expect("valid wire bytes from listener");
        let caller_outputs = caller.feed(&pkt).expect("caller feed");
        let mut caller_bytes = None;
        for out in caller_outputs {
            if let HandshakeOutput::Send(bytes) = out {
                caller_bytes = Some(bytes);
            }
        }
        match caller_bytes {
            Some(bytes) => next = bytes,
            None => return, // Caller reached Connected/Rejected with no more bytes to send.
        }
    }
    panic!("handshake did not converge within 10 round trips");
}

#[test]
fn caller_and_listener_converge_on_connected_with_matching_params() {
    let stream_id = "live/camera-1".to_string();
    let group = GroupMembershipExtension {
        group_id: 7,
        group_type: GroupType::Broadcast,
        flags: GroupFlags(0x01),
        weight: 3,
    };

    let caller_config = HandshakeConfig {
        latency_ms: 180,
        stream_id: Some(stream_id.clone()),
        group: Some(group),
        ..HandshakeConfig::default()
    };
    let listener_config = HandshakeConfig {
        latency_ms: 250,
        ..HandshakeConfig::default()
    };

    let mut caller = CallerHandshake::new(CALLER_SOCKET_ID, caller_config);
    let mut listener = ListenerHandshake::new(LISTENER_SOCKET_ID, SYN_COOKIE, listener_config);

    run_handshake(&mut caller, &mut listener);

    assert_eq!(caller.state(), CallerHandshakeState::Connected);
    assert_eq!(listener.state(), ListenerHandshakeState::Connected);

    let caller_params = caller.negotiated().expect("caller negotiated params");
    let listener_params = listener.negotiated().expect("listener negotiated params");

    // Both sides agree on the base protocol version.
    assert_eq!(caller_params.version, 5);
    assert_eq!(listener_params.version, 5);

    // Socket IDs cross-match: each side's peer is the other's own id.
    assert_eq!(caller_params.own_socket_id, CALLER_SOCKET_ID);
    assert_eq!(caller_params.peer_socket_id, LISTENER_SOCKET_ID);
    assert_eq!(listener_params.own_socket_id, LISTENER_SOCKET_ID);
    assert_eq!(listener_params.peer_socket_id, CALLER_SOCKET_ID);

    // §4.3.1.2: the greater-of-both-parties latency rule lands on the same
    // number regardless of which side computes it.
    assert_eq!(caller_params.latency_ms, 250);
    assert_eq!(listener_params.latency_ms, 250);
    assert_eq!(caller_params.latency_ms, listener_params.latency_ms);

    // Stream ID and Group Membership (Caller-advertised) are seen identically
    // by both sides.
    assert_eq!(caller_params.stream_id.as_deref(), Some(stream_id.as_str()));
    assert_eq!(
        listener_params.stream_id.as_deref(),
        Some(stream_id.as_str())
    );
    assert_eq!(caller_params.stream_id, listener_params.stream_id);
    assert_eq!(listener_params.group, Some(group));
}

#[test]
fn caller_and_listener_converge_without_optional_extensions() {
    let mut caller = CallerHandshake::new(CALLER_SOCKET_ID, HandshakeConfig::default());
    let mut listener =
        ListenerHandshake::new(LISTENER_SOCKET_ID, SYN_COOKIE, HandshakeConfig::default());

    run_handshake(&mut caller, &mut listener);

    assert_eq!(caller.state(), CallerHandshakeState::Connected);
    assert_eq!(listener.state(), ListenerHandshakeState::Connected);
    assert_eq!(
        caller.negotiated().unwrap().stream_id,
        listener.negotiated().unwrap().stream_id
    );
    assert_eq!(caller.negotiated().unwrap().stream_id, None);
}

#[test]
fn listener_rejects_a_caller_with_the_wrong_cookie() {
    // A hostile/confused caller that never actually captured the Listener's
    // cookie sends whatever it likes for CONCLUSION — model this by driving
    // the Caller normally, then tampering with the cookie it captured before
    // it builds CONCLUSION would be intrusive; instead exercise the Listener
    // directly with a forged CONCLUSION carrying the wrong cookie.
    use srt_runtime::packet::{
        EncryptionField, ExtensionType, HandshakeExtensionFlags, HandshakeExtensions,
        HandshakePacket, HandshakeType, HsExtMessage, handshake::build_extension_block,
    };

    let mut listener =
        ListenerHandshake::new(LISTENER_SOCKET_ID, SYN_COOKIE, HandshakeConfig::default());

    let induction = ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 0,
        version: 4,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(2),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Induction,
        srt_socket_id: CALLER_SOCKET_ID,
        syn_cookie: 0,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    });
    listener.feed(&induction).unwrap();
    assert_eq!(listener.state(), ListenerHandshakeState::AwaitingConclusion);

    let hs_msg = HsExtMessage {
        srt_version: 0x0105_0000,
        srt_flags: HandshakeConfig::default().flags,
        receiver_tsbpd_delay_ms: 120,
        sender_tsbpd_delay_ms: 120,
    };
    let ext = build_extension_block(ExtensionType::HsReq, &hs_msg.to_bytes()).unwrap();
    let conclusion = ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: LISTENER_SOCKET_ID,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0x0001),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Conclusion,
        srt_socket_id: CALLER_SOCKET_ID,
        syn_cookie: SYN_COOKIE.wrapping_add(1), // wrong cookie
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&ext),
    });

    let outputs = listener.feed(&conclusion).unwrap();
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, HandshakeOutput::Rejected(RejectionReason::Rogue))),
        "expected a Rogue rejection for the bad cookie, got {outputs:?}"
    );
    assert_eq!(listener.state(), ListenerHandshakeState::Rejected);

    // The rejection packet the Listener sent back must itself carry the
    // Table 7 wire encoding, not just be reported as a typed event.
    let reject_bytes = outputs
        .iter()
        .find_map(|o| match o {
            HandshakeOutput::Send(b) => Some(b),
            _ => None,
        })
        .expect("listener must send a rejection packet");
    let parsed = ControlPacket::parse(reject_bytes).unwrap();
    if let ControlPacket::Handshake(hp) = parsed {
        assert_eq!(
            hp.handshake_type,
            RejectionReason::Rogue.to_handshake_type()
        );
    } else {
        panic!("expected a handshake packet");
    }
}
