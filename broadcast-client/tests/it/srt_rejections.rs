//! Every `REJ_*` code a listener can answer the caller's induction with is
//! surfaced as a non-fatal `SrtRejected` error naming that code, closes the
//! socket and is retried after the backoff (the wrong-passphrase path in
//! `srt.rs` covers only `REJ_BADSECRET`, which arrives at the conclusion step).

use crate::support::srt_peer::{Cipher, SrtPeer};
use crate::support::srt_sim::{SrtSim, config_with_seed};
use broadcast_client::{Config, ErrorKind, Event, Status};
use srt_runtime::RejectionReason;

use srt_runtime::packet::{
    ControlPacket, EncryptionField, HandshakeExtensionFlags, HandshakeExtensions, HandshakePacket,
    HandshakeType, SrtPacket,
};

const URL: &str = "srt://127.0.0.1:9000";
const LATENCY_MS: u16 = 120;
const SECOND_US: u64 = 1_000_000;

fn rejection_for(reason: RejectionReason, own_socket_id: u32) -> Vec<u8> {
    let packet = ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: own_socket_id,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Reserved(reason.to_bits()),
        srt_socket_id: 0,
        syn_cookie: 0,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    });
    let mut bytes = vec![0u8; packet.serialized_len()];
    packet
        .serialize_into(&mut bytes)
        .expect("serialize rejection");
    bytes
}

fn rejected_reasons(sim: &SrtSim) -> Vec<&'static str> {
    sim.events
        .iter()
        .filter_map(|t| match &t.event {
            Event::Error {
                kind: ErrorKind::SrtRejected { reason },
                fatal,
            } => {
                assert!(!fatal, "a rejection is retried, never fatal");
                Some(*reason)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn each_induction_rejection_reason_is_reported_by_name_and_retried_after_a_backoff() {
    for reason in [
        RejectionReason::System,
        RejectionReason::Peer,
        RejectionReason::Resource,
        RejectionReason::Rogue,
        RejectionReason::Backlog,
        RejectionReason::Ipe,
        RejectionReason::Close,
        RejectionReason::Version,
        RejectionReason::MessageApi,
        RejectionReason::Congestion,
        RejectionReason::Filter,
    ] {
        let mut sim = SrtSim::open(
            URL,
            config_with_seed(Config::default()),
            SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
        );
        let (_, induction) = sim.from_session.first().expect("induction request").clone();
        let Ok(SrtPacket::Control(ControlPacket::Handshake(hp))) = SrtPacket::parse(&induction)
        else {
            panic!("{reason:?}: the first datagram is a handshake");
        };
        sim.raw_to_session(rejection_for(reason, hp.srt_socket_id));
        sim.advance_by(SECOND_US / 2);

        assert_eq!(rejected_reasons(&sim), vec![reason.name()], "{reason:?}");
        assert_eq!(sim.closes.len(), 1, "{reason:?}: socket closed");
        assert!(
            sim.count(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))) >= 1,
            "{reason:?}"
        );
        assert_eq!(
            sim.binds_count(),
            1,
            "{reason:?}: the retry waits for the backoff"
        );
        sim.advance_by(3 * SECOND_US);
        assert_eq!(
            sim.binds_count(),
            2,
            "{reason:?}: retried after the backoff"
        );
    }
}
