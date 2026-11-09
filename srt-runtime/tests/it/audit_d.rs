//! Bug-audit D reproductions (srt-runtime).

use core::time::Duration;

use srt_runtime::arq::{Receiver, Sender};
use srt_runtime::packet::{AckCif, AckPacket};

const PEER: u32 = 0xBBBB;
const WINDOW: u32 = 8192;

/// A DROPREQ for a range AHEAD of the ack point must still be honoured once
/// the gap before it fills: the ack point has to jump the dropped range.
#[test]
fn receiver_ack_point_crosses_a_dropreq_range_announced_ahead_of_it() {
    let mut r = Receiver::new(PEER, 0, WINDOW);
    let t = Duration::ZERO;
    r.feed_data(0, t);
    r.feed_data(5, t);
    r.skip_range(3, 4);
    r.feed_data(1, t);
    r.feed_data(2, t);
    assert_eq!(
        r.ack_point(),
        6,
        "0,1,2 delivered, 3..=4 dropped by the sender, 5 held: ack point must reach 6"
    );
}

/// An ACK naming a sequence number the sender never sent must not free data.
#[test]
fn sender_ignores_an_ack_beyond_the_highest_sent_sequence() {
    let mut s = Sender::new(PEER);
    for seq in 0..5u32 {
        s.on_data(seq, seq, b"x", Duration::ZERO).unwrap();
    }
    let ack = AckPacket {
        ack_number: 0,
        timestamp: 0,
        dest_socket_id: PEER,
        cif: AckCif::Light { last_ack_seq: 1000 },
    };
    s.on_ack(&ack, Duration::ZERO);
    assert_eq!(
        s.buffered_count(),
        5,
        "seqs 0..=4 were sent; an ACK for 1000 acknowledges unsent data"
    );
}

/// Caller/Listener reject an Initial Packet Sequence Number wider than 31 bits
/// as Rogue; a Rendezvous peer's ISN must get the same check.
#[test]
fn rendezvous_rejects_a_peer_isn_wider_than_31_bits() {
    use srt_runtime::handshake_sm::{HandshakeConfig, HandshakeOutput};
    use srt_runtime::packet::ControlPacket;
    use srt_runtime::rendezvous::RendezvousHandshake;

    let mut a = RendezvousHandshake::new(0x1111, 0xAAAA_0002, HandshakeConfig::default());
    let mut b = RendezvousHandshake::new(
        0x2222,
        0x1111_0001,
        HandshakeConfig {
            initial_seq_number: 0xFFFF_FFFF,
            ..HandshakeConfig::default()
        },
    );
    let _ = a.start().unwrap();
    let wave = b.start().unwrap();
    let outputs = a.feed(&ControlPacket::parse(&wave).unwrap()).unwrap();
    let mut connected = false;
    let mut rejected = false;
    let mut pending = outputs;
    for _ in 0..6 {
        let mut next = Vec::new();
        for o in pending.drain(..) {
            match o {
                HandshakeOutput::Send(bytes) => {
                    for r in b.feed(&ControlPacket::parse(&bytes).unwrap()).unwrap() {
                        if let HandshakeOutput::Send(back) = r {
                            next.extend(a.feed(&ControlPacket::parse(&back).unwrap()).unwrap());
                        }
                    }
                }
                HandshakeOutput::Connected(_) => connected = true,
                HandshakeOutput::Rejected(_) => rejected = true,
                _ => {}
            }
        }
        pending = next;
    }
    assert!(
        rejected && !connected,
        "peer ISN 0xFFFF_FFFF accepted: connected={connected} rejected={rejected}"
    );
}

/// MAX_BW arithmetic must saturate, not overflow, on a large configured rate.
#[test]
fn livecc_input_based_max_bw_does_not_overflow() {
    use srt_runtime::livecc::MaxBwConfig;
    let bw = MaxBwConfig::InputBased {
        input_bw: u64::MAX / 4,
        overhead: 25,
    }
    .max_bw_bytes_per_sec()
    .unwrap();
    assert!(bw >= u64::MAX / 4);
}

/// A DROPREQ naming a range far wider than any flow window must not move the
/// ack point 2^30 packets ahead (every later packet would then be ignored as
/// "already delivered").
#[test]
fn receiver_ignores_a_dropreq_range_wider_than_the_flow_window() {
    let mut r = Receiver::new(PEER, 0, WINDOW);
    r.feed_data(0, Duration::ZERO);
    r.skip_range(1, 0x3FFF_FFFF);
    let out = r.feed_data(1, Duration::ZERO);
    assert_eq!(
        out.delivered,
        vec![1],
        "ack point jumped to {:#x}; seq 1 was discarded as a duplicate",
        r.ack_point()
    );
}
