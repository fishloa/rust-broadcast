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
    assert_eq!(s.buffered_count(), 5, "seqs 0..=4 were sent; an ACK for 1000 acknowledges unsent data");
}
