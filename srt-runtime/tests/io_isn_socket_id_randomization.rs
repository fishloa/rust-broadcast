//! Regression coverage for the tokio UDP adapter's Caller identifiers:
//! every new connection must get a freshly generated Initial Sequence
//! Number and SRT Socket ID, not a reused fixed value.
//! `draft-sharabayko-srt-01` §3 treats the ISN and the Socket ID as
//! independent, per-connection values; a peer that can predict either from
//! one observed connection can also predict them for the next one made from
//! the same process.

#![cfg(feature = "tokio")]

use core::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::SrtSocket;
use srt_runtime::packet::{ControlPacket, SrtPacket};
use tokio::net::UdpSocket;

/// SRT Initial Sequence Numbers occupy the legal 31-bit sequence-number
/// range (`draft-sharabayko-srt-01` §3, mirroring `SEQ_NUMBER_MASK`); a
/// generated ISN must never set the reserved top bit.
const ISN_UPPER_BOUND: u32 = 0x7FFF_FFFF;

/// Fires `SrtSocket::connect` at a bound-but-silent UDP socket, captures the
/// INDUCTION handshake packet it sends (the Caller's own Socket ID and ISN
/// are both carried in that first packet), then aborts the connect task
/// before it can retry or time out.
async fn capture_induction(config: HandshakeConfig) -> (u32, u32) {
    let fake_peer = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind fake peer");
    let peer_addr = fake_peer.local_addr().expect("fake peer addr");

    let connect_task = tokio::spawn(async move {
        let _ = SrtSocket::connect(peer_addr, config).await;
    });

    let mut buf = [0u8; 1500];
    let (len, _src) = tokio::time::timeout(Duration::from_secs(2), fake_peer.recv_from(&mut buf))
        .await
        .expect("timed out waiting for the INDUCTION packet")
        .expect("recv_from");

    connect_task.abort();

    match SrtPacket::parse(&buf[..len]).expect("parse induction bytes") {
        SrtPacket::Control(ControlPacket::Handshake(hp)) => {
            (hp.srt_socket_id, hp.initial_seq_number)
        }
        other => panic!("expected an INDUCTION handshake packet, got {other:?}"),
    }
}

#[tokio::test]
async fn two_connects_use_different_socket_ids_and_isns() {
    let (socket_id_a, isn_a) = capture_induction(HandshakeConfig::default()).await;
    let (socket_id_b, isn_b) = capture_induction(HandshakeConfig::default()).await;

    assert_ne!(
        socket_id_a, socket_id_b,
        "two independent connects must not reuse the same SRT Socket ID"
    );
    assert_ne!(
        isn_a, isn_b,
        "two independent connects must not reuse the same Initial Sequence Number"
    );

    for isn in [isn_a, isn_b] {
        assert!(
            isn <= ISN_UPPER_BOUND,
            "generated ISN {isn:#010x} exceeds the legal 31-bit range"
        );
    }
}
