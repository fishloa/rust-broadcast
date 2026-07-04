//! ARQ recovery integration test — wires a [`Sender`] and [`Receiver`] in
//! memory (no sockets, no threads) and drives them through data loss,
//! NAK-triggered retransmission, and the ACK/ACKACK exchange, per
//! `specs/rules/srt-arq.md` (`draft-sharabayko-srt-01` §4.8/§4.8.1/§4.8.2/
//! §4.10).

use core::time::Duration;

use srt_runtime::arq::{FULL_ACK_PERIOD, Receiver, Sender};
use srt_runtime::packet::{AckAckPacket, AckCif, AckPacket, ControlPacket, DataPacket, NakPacket};

const SENDER_ID: u32 = 0x1111_1111;
const RECEIVER_ID: u32 = 0x2222_2222;

fn as_ack(bytes: &[u8]) -> Option<AckPacket> {
    match ControlPacket::parse(bytes).unwrap() {
        ControlPacket::Ack(a) => Some(a),
        _ => None,
    }
}

fn as_ackack(bytes: &[u8]) -> Option<AckAckPacket> {
    match ControlPacket::parse(bytes).unwrap() {
        ControlPacket::AckAck(a) => Some(a),
        _ => None,
    }
}

fn as_nak(bytes: &[u8]) -> Option<NakPacket<'_>> {
    match ControlPacket::parse(bytes).unwrap() {
        ControlPacket::Nak(n) => Some(n),
        _ => None,
    }
}

/// Positive case: RTT converges, a NAK triggers retransmission of dropped
/// packets, every packet is ultimately delivered in order, and the ACK/
/// ACKACK exchange advances the sender's acknowledged sequence.
#[test]
fn nak_retransmit_recovers_all_packets_in_order_and_rtt_converges() {
    let mut sender = Sender::new(RECEIVER_ID);
    let mut receiver = Receiver::new(SENDER_ID, 0);

    let mut now = Duration::ZERO;
    const TARGET_RTT: Duration = Duration::from_millis(30);

    // --- Phase 1: drive 40 ACK/ACKACK round trips at a fixed simulated
    // network delay, and check RTT converges toward it on both sides
    // (srt-arq.md rules 26-31, 33).
    for _ in 0..40 {
        now += Duration::from_millis(50);
        let outputs = receiver.tick(now);
        let ack = outputs
            .iter()
            .find_map(|b| as_ack(b))
            .expect("Full ACK due every 10ms of elapsed time");
        let ackack_bytes = sender
            .on_ack(&ack, now)
            .expect("a Full ACK must trigger an ACKACK (rules 3, 9)");
        let ackack = as_ackack(&ackack_bytes).expect("ACKACK control type");
        // The ACKACK "arrives back" at the receiver one injected RTT later.
        receiver.on_ackack(&ackack, now + TARGET_RTT);
    }

    let receiver_rtt_ms = receiver.rtt().as_millis() as i64;
    assert!(
        (receiver_rtt_ms - 30).abs() <= 5,
        "receiver RTT should converge near the injected 30ms, got {receiver_rtt_ms}ms"
    );
    let sender_rtt_ms = sender.rtt().as_millis() as i64;
    assert!(
        (sender_rtt_ms - 30).abs() <= 15,
        "sender RTT (derived from each ACK's carried value, rule 33) should \
         also converge toward 30ms, got {sender_rtt_ms}ms"
    );

    now += Duration::from_millis(50);

    // --- Phase 2: send 10 data packets, drop #3 and #7 "in transit".
    const TOTAL: u32 = 10;
    const DROPPED: [u32; 2] = [3, 7];
    let mut in_flight: Vec<Vec<u8>> = Vec::new();
    for seq in 0..TOTAL {
        let payload = alloc_payload(seq);
        let bytes = sender.on_data(seq, seq, &payload, now);
        if !DROPPED.contains(&seq) {
            in_flight.push(bytes);
        }
        now += Duration::from_millis(1);
    }
    assert_eq!(sender.buffered_count(), TOTAL as usize);

    let mut delivered_order: Vec<u32> = Vec::new();
    let mut naks: Vec<Vec<u8>> = Vec::new();
    for bytes in &in_flight {
        let dp = DataPacket::parse(bytes).unwrap();
        let outcome = receiver.feed_data(dp.seq_number, now);
        delivered_order.extend(outcome.delivered);
        if let Some(nak) = outcome.nak {
            naks.push(nak);
        }
        now += Duration::from_millis(1);
    }
    assert_eq!(
        naks.len(),
        2,
        "each dropped packet opens exactly one new gap (rules 4, 14)"
    );
    assert_eq!(receiver.loss_list_len(), 2);
    assert!(
        delivered_order.len() < TOTAL as usize,
        "in-order delivery must stall at the first gap"
    );

    // Feed the NAKs to the sender; it must prioritize retransmitting
    // exactly the two dropped packets (rules 5, 15, 16, 18).
    for nak in &naks {
        let n = as_nak(nak).expect("NAK control type");
        sender.on_nak(&n);
    }
    let retransmits = sender.tick(now);
    assert_eq!(
        retransmits.len(),
        2,
        "the receiver's NAK must trigger sender retransmission of exactly \
         the dropped packets"
    );
    for bytes in &retransmits {
        let dp = DataPacket::parse(bytes).unwrap();
        assert!(
            dp.retransmitted,
            "a retransmitted data packet must set the R flag"
        );
        assert!(DROPPED.contains(&dp.seq_number));
        let outcome = receiver.feed_data(dp.seq_number, now);
        delivered_order.extend(outcome.delivered);
        assert!(
            outcome.nak.is_none(),
            "filling an already-known gap must not open a new one"
        );
        now += Duration::from_millis(1);
    }

    assert_eq!(
        delivered_order,
        (0..TOTAL).collect::<Vec<u32>>(),
        "every packet must ultimately be delivered, in order"
    );
    assert_eq!(receiver.ack_point(), TOTAL);
    assert_eq!(receiver.loss_list_len(), 0);

    // --- Phase 3: the next Full ACK/ACKACK exchange advances the sender's
    // acknowledged sequence and frees its send buffer (rules 1, 7, 8).
    now += FULL_ACK_PERIOD + Duration::from_millis(1);
    let outputs = receiver.tick(now);
    let ack = outputs
        .iter()
        .find_map(|b| as_ack(b))
        .expect("Full ACK due");
    match ack.cif {
        AckCif::Full { last_ack_seq, .. } => assert_eq!(last_ack_seq, TOTAL),
        other => panic!("expected a Full ACK CIF, got {other:?}"),
    }
    let ackack_bytes = sender
        .on_ack(&ack, now)
        .expect("Full ACK triggers an ACKACK");
    assert_eq!(
        sender.buffered_count(),
        0,
        "the ACK must free every acknowledged packet from the send buffer"
    );
    let ackack = as_ackack(&ackack_bytes).unwrap();
    receiver.on_ackack(&ackack, now + Duration::from_millis(5));
}

/// Negative case: a loss-free run must never emit a NAK, either from the
/// immediate gap-detection path or the periodic-NAK timer.
#[test]
fn zero_loss_run_emits_no_spurious_naks() {
    let mut sender = Sender::new(RECEIVER_ID);
    let mut receiver = Receiver::new(SENDER_ID, 0);
    let mut now = Duration::ZERO;

    for seq in 0..50u32 {
        let payload = alloc_payload(seq);
        let bytes = sender.on_data(seq, seq, &payload, now);
        let dp = DataPacket::parse(&bytes).unwrap();
        let outcome = receiver.feed_data(dp.seq_number, now);
        assert!(
            outcome.nak.is_none(),
            "no loss occurred; feed_data must not emit a NAK for seq {seq}"
        );
        now += Duration::from_millis(1);
    }

    // Drive well past several NAKInterval/Full-ACK periods; still nothing,
    // because the loss list stays empty (srt-arq.md rules 21-22).
    for _ in 0..20 {
        now += Duration::from_millis(50);
        let outputs = receiver.tick(now);
        for bytes in &outputs {
            assert!(
                as_nak(bytes).is_none(),
                "a zero-loss run must never emit a NAK"
            );
        }
    }

    assert_eq!(receiver.ack_point(), 50);
    assert_eq!(receiver.loss_list_len(), 0);
}

fn alloc_payload(seq: u32) -> Vec<u8> {
    format!("payload-{seq}").into_bytes()
}
