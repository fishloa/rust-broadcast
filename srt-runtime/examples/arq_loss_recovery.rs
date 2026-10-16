//! `arq_loss_recovery` — SRT's ARQ engine recovering dropped packets, in memory
//! (`draft-sharabayko-srt-01` §4.8 / §4.10).
//!
//! A [`Sender`] transmits ten data packets; two are "lost in transit". The
//! [`Receiver`] notices each gap and emits a NAK, the sender retransmits
//! exactly the missing packets (R flag set), and in-order delivery resumes.
//! A final ACK / ACKACK exchange frees the sender's buffer.
//!
//! ```text
//! cargo run -p srt-runtime --example arq_loss_recovery
//! ```

use core::time::Duration;

use srt_runtime::arq::{FULL_ACK_PERIOD, Receiver, Sender};
use srt_runtime::packet::{ControlPacket, DataPacket};

const SENDER_ID: u32 = 0x1111_1111;
const RECEIVER_ID: u32 = 0x2222_2222;
/// `HandshakeConfig::default().max_flow_window_size`; supplied by hand because
/// this example wires the receiver directly, without a handshake.
const MAX_FLOW_WINDOW: u32 = 8192;
const TOTAL: u32 = 10;
const DROPPED: [u32; 2] = [3, 7];

fn main() {
    let mut sender = Sender::new(RECEIVER_ID);
    let mut receiver = Receiver::new(SENDER_ID, 0, MAX_FLOW_WINDOW);
    let mut now = Duration::ZERO;

    // Send everything; the network drops two packets.
    let mut in_flight = Vec::new();
    for seq in 0..TOTAL {
        let payload = vec![seq as u8; 16];
        let bytes = sender
            .on_data(seq, seq, &payload, now)
            .expect("in-range sequence/message numbers");
        if DROPPED.contains(&seq) {
            println!("seq {seq}: dropped in transit");
        } else {
            in_flight.push(bytes);
        }
        now += Duration::from_millis(1);
    }

    // The receiver sees the survivors; each gap triggers one NAK.
    let mut delivered: Vec<u32> = Vec::new();
    let mut naks = Vec::new();
    for bytes in &in_flight {
        let dp = DataPacket::parse(bytes).expect("data packet");
        let outcome = receiver.feed_data(dp.seq_number, now);
        delivered.extend(outcome.delivered);
        if let Some(nak) = outcome.nak {
            naks.push(nak);
        }
        now += Duration::from_millis(1);
    }
    println!(
        "after first pass: delivered in order {delivered:?}, {} NAK(s), loss list {}",
        naks.len(),
        receiver.loss_list_len()
    );

    // NAKs go back to the sender, which retransmits just the missing packets.
    for nak in &naks {
        if let ControlPacket::Nak(n) = ControlPacket::parse(nak).expect("NAK") {
            sender.on_nak(&n);
        }
    }
    for bytes in sender.tick(now) {
        let dp = DataPacket::parse(&bytes).expect("retransmission");
        println!(
            "seq {}: retransmitted (R flag = {})",
            dp.seq_number, dp.retransmitted
        );
        delivered.extend(receiver.feed_data(dp.seq_number, now).delivered);
        now += Duration::from_millis(1);
    }
    println!("final delivery order: {delivered:?}");
    assert_eq!(delivered, (0..TOTAL).collect::<Vec<u32>>());

    // A Full ACK / ACKACK exchange acknowledges everything and frees the buffer.
    println!(
        "sender buffer before ACK: {} packets",
        sender.buffered_count()
    );
    now += FULL_ACK_PERIOD + Duration::from_millis(1);
    for bytes in receiver.tick(now) {
        if let ControlPacket::Ack(ack) = ControlPacket::parse(&bytes).expect("control packet") {
            let ackack = sender.on_ack(&ack, now).expect("Full ACK -> ACKACK");
            if let ControlPacket::AckAck(a) = ControlPacket::parse(&ackack).expect("ACKACK") {
                receiver.on_ackack(&a, now + Duration::from_millis(5));
            }
        }
    }
    println!(
        "sender buffer after ACK:  {} packets",
        sender.buffered_count()
    );
    assert_eq!(sender.buffered_count(), 0);
}
