//! `TsResync` accounting and the stateless datagram resync helpers.
//!
//! Pins the public `resync`/`resync_confirmed`/`unconfirmed_candidate`
//! results, the exact `ResyncLoss` spans and `ResyncStats` counters a
//! TS-over-UDP/file reader reports, the 204-byte Reed-Solomon and 192-byte
//! M2TS strides, the end-of-input truncated tail, and `feed_into`'s return count.

use mpeg_ts::resync::{
    LOCK_CONFIRMATIONS, M2TS_EXTRA_HEADER_LEN, M2TS_PACKET_SIZE, PacketStride, RS_PACKET_SIZE,
    RS_PARITY_LEN, ResyncLoss, TsResync, resync, resync_confirmed, unconfirmed_candidate,
};
use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};

fn assert_loss(loss: &ResyncLoss, offset: u64, dropped_bytes: u64, packet_index: u64, tail: bool) {
    assert_eq!(loss.offset, offset);
    assert_eq!(loss.dropped_bytes, dropped_bytes);
    assert_eq!(loss.packet_index, packet_index);
    assert_eq!(loss.truncated_tail, tail);
}

fn ts_packet(tag: u8) -> [u8; TS_PACKET_SIZE] {
    assert_ne!(tag, TS_SYNC_BYTE);
    let mut pkt = [tag; TS_PACKET_SIZE];
    pkt[0] = TS_SYNC_BYTE;
    pkt
}

fn rs_packet(tag: u8) -> [u8; RS_PACKET_SIZE] {
    let mut pkt = [0xEEu8; RS_PACKET_SIZE];
    pkt[0] = TS_SYNC_BYTE;
    pkt[1..TS_PACKET_SIZE].fill(tag);
    pkt
}

fn concat(packets: &[[u8; TS_PACKET_SIZE]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(packets.len() * TS_PACKET_SIZE);
    for p in packets {
        v.extend_from_slice(p);
    }
    v
}

#[test]
fn packet_size_constants_are_the_wire_values() {
    assert_eq!(TS_PACKET_SIZE, 188);
    assert_eq!(RS_PACKET_SIZE, 204);
    assert_eq!(RS_PARITY_LEN, 16);
    assert_eq!(LOCK_CONFIRMATIONS, 5);
}

#[test]
fn resync_finds_a_confirmed_pair_after_a_decoy_sync_byte() {
    let mut buf = [0u8; 400];
    buf[0] = TS_SYNC_BYTE;
    buf[3] = TS_SYNC_BYTE;
    buf[3 + TS_PACKET_SIZE] = TS_SYNC_BYTE;
    assert_eq!(resync(&buf), Some(3));
    assert_eq!(resync_confirmed(&buf), Some(3));
}

#[test]
fn resync_accepts_a_lone_sync_byte_but_resync_confirmed_does_not() {
    let mut buf = [0u8; 100];
    buf[5] = TS_SYNC_BYTE;
    assert_eq!(resync(&buf), Some(5));
    assert_eq!(resync_confirmed(&buf), None);
    assert_eq!(unconfirmed_candidate(&buf), Some(5));

    let empty: [u8; 0] = [];
    assert_eq!(resync(&empty), None);
    assert_eq!(resync_confirmed(&empty), None);
    assert_eq!(unconfirmed_candidate(&empty), None);
    let no_sync = [0u8; 10];
    assert_eq!(resync(&no_sync), None);
}

#[test]
fn resync_accepts_a_lone_sync_byte_exactly_one_packet_from_the_end() {
    let mut buf = [0u8; 5 + TS_PACKET_SIZE];
    buf[5] = TS_SYNC_BYTE;
    assert_eq!(resync(&buf), Some(5));
    assert_eq!(resync_confirmed(&buf), None);
}

#[test]
fn unconfirmed_candidate_reports_the_position_past_the_confirmation_window() {
    let mut buf = [0u8; 200];
    buf[190] = TS_SYNC_BYTE;
    assert_eq!(unconfirmed_candidate(&buf), Some(190));

    let mut early = [0u8; 200];
    early[5] = TS_SYNC_BYTE;
    assert_eq!(unconfirmed_candidate(&early), None);
}

#[test]
fn a_confirmed_pair_in_a_large_buffer_is_reported_at_its_offset() {
    let mut buf = [0u8; 2 * TS_PACKET_SIZE];
    buf[1] = TS_SYNC_BYTE;
    buf[1 + TS_PACKET_SIZE] = TS_SYNC_BYTE;
    assert_eq!(resync(&buf), Some(1));
    assert_eq!(resync_confirmed(&buf), Some(1));
}

#[test]
fn a_mid_stream_sync_loss_reports_one_seven_byte_span() {
    let packets: Vec<_> = (1u8..=10).map(ts_packet).collect();
    let mut bytes = concat(&packets[..5]);
    bytes.extend_from_slice(&[0x99; 7]);
    bytes.extend_from_slice(&concat(&packets[5..]));

    let mut r = TsResync::new();
    let got = r.feed(&bytes);
    assert_eq!(got.len(), 10);
    assert_eq!(got[0], packets[0]);
    assert_eq!(got[9], packets[9]);
    assert_eq!(r.stride(), Some(PacketStride::Ts188));

    let stats = r.stats();
    assert_eq!(stats.packets, 10);
    assert_eq!(stats.resyncs, 1);
    assert_eq!(stats.dropped_bytes, 8);

    let losses = r.take_losses();
    assert_eq!(losses.len(), 1);
    assert_loss(&losses[0], 5 * TS_PACKET_SIZE as u64, 7, 5, false);
}

#[test]
fn a_204_byte_stride_is_detected_and_parity_stripped() {
    let packets: Vec<_> = (1u8..=10).map(rs_packet).collect();
    let mut bytes = Vec::new();
    for p in &packets {
        bytes.extend_from_slice(p);
    }

    let mut r = TsResync::new();
    let got = r.feed(&bytes);
    assert_eq!(got.len(), 10);
    assert_eq!(r.stride(), Some(PacketStride::Rs204));
    assert_eq!(r.stats().packets, 10);
    assert_eq!(r.stats().dropped_bytes, 0);
    for (i, pkt) in got.iter().enumerate() {
        assert_eq!(&pkt[..], &packets[i][..TS_PACKET_SIZE]);
    }
}

#[test]
fn a_192_byte_m2ts_stride_is_detected_and_extra_header_stripped() {
    let packets: Vec<_> = (1u8..=10).map(ts_packet).collect();
    let mut bytes = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        bytes.extend_from_slice(&[0x40, 0x00, 0x12, i as u8]);
        bytes.extend_from_slice(p);
    }

    let mut r = TsResync::new();
    let got = r.feed(&bytes[1..]);
    assert_eq!(r.stride(), Some(PacketStride::M2ts192));
    assert_eq!(PacketStride::M2ts192.packet_len(), M2TS_PACKET_SIZE);
    assert_eq!(got.len(), 10);
    assert_eq!(got, packets);
    assert_eq!(r.stats().packets, 10);

    let losses = r.take_losses();
    assert_eq!(losses.len(), 1);
    assert_loss(&losses[0], 0, (M2TS_EXTRA_HEADER_LEN - 1) as u64, 0, false);
}

#[test]
fn an_m2ts_stream_fed_one_byte_at_a_time_yields_the_same_packets() {
    let packets: Vec<_> = (1u8..=8).map(ts_packet).collect();
    let mut bytes = Vec::new();
    for p in &packets {
        bytes.extend_from_slice(&[0x40, 0x00, 0x12, 0x34]);
        bytes.extend_from_slice(p);
    }

    let mut r = TsResync::new();
    let mut got = Vec::new();
    for byte in &bytes {
        got.extend(r.feed(core::slice::from_ref(byte)));
    }
    assert_eq!(got, packets);
    assert_eq!(r.stats().resyncs, 0);
    assert_eq!(r.stats().dropped_bytes, M2TS_EXTRA_HEADER_LEN as u64);
}

#[test]
fn finish_reports_a_truncated_tail_and_counts_it_as_dropped() {
    let packets: Vec<_> = (1u8..=5).map(ts_packet).collect();
    let mut bytes = concat(&packets);
    bytes.extend_from_slice(&[0x33; 50]);

    let mut r = TsResync::new();
    assert_eq!(r.feed(&bytes).len(), 5);
    r.finish();

    assert_eq!(r.stats().dropped_bytes, 50);
    let losses = r.take_losses();
    assert_eq!(losses.len(), 1);
    assert_loss(&losses[0], 5 * TS_PACKET_SIZE as u64, 50, 5, true);
}

#[test]
fn finish_emits_a_stream_shorter_than_the_lock_window() {
    let packets: Vec<_> = (1u8..=3).map(ts_packet).collect();
    let mut r = TsResync::new();
    assert!(r.feed(&concat(&packets)).is_empty());
    r.finish();
    let tail = r.take_tail_packets();
    assert_eq!(tail.len(), 3);
    assert_eq!(tail, packets);
    assert!(r.take_tail_packets().is_empty());
}

#[test]
fn finish_closes_a_sync_loss_that_never_relocked() {
    let packets: Vec<_> = (1u8..=5).map(ts_packet).collect();
    let mut bytes = concat(&packets);
    bytes.extend_from_slice(&[0x00; 300]);

    let mut r = TsResync::new();
    assert_eq!(r.feed(&bytes).len(), 5);
    r.finish();

    assert_eq!(r.stats().dropped_bytes, 300);
    let losses = r.take_losses();
    assert_eq!(losses.len(), 1);
    assert_loss(&losses[0], 5 * TS_PACKET_SIZE as u64, 300, 5, false);
}

#[test]
fn feed_into_returns_the_number_appended_not_the_total() {
    let packets: Vec<_> = (1u8..=5).map(ts_packet).collect();
    let mut out = vec![ts_packet(0xAA); 2];
    let mut r = TsResync::new();
    let n = r.feed_into(&concat(&packets), &mut out);
    assert_eq!(n, 5);
    assert_eq!(out.len(), 7);
    assert_eq!(out[2], packets[0]);
    assert!(out.capacity() < 10_000);
}

#[test]
fn feed_into_returns_zero_before_lock_even_with_a_prefilled_output() {
    let mut out = vec![ts_packet(0xAA); 2];
    let mut r = TsResync::new();
    let n = r.feed_into(&[0x00u8; 10], &mut out);
    assert_eq!(n, 0);
    assert_eq!(out.len(), 2);
}

#[test]
fn bulk_junk_before_lock_is_dropped_and_later_accounted_as_one_span() {
    let junk = [0x00u8; 2000];
    let packets: Vec<_> = (1u8..=10).map(ts_packet).collect();

    let mut r = TsResync::new();
    assert!(r.feed(&junk).is_empty());
    assert_eq!(
        r.stats().dropped_bytes,
        (2000 - LOCK_CONFIRMATIONS * RS_PACKET_SIZE) as u64
    );

    assert_eq!(r.feed(&concat(&packets)).len(), 10);
    assert_eq!(r.stats().resyncs, 0);
    assert_eq!(r.stats().dropped_bytes, 2000);
    let window = (LOCK_CONFIRMATIONS * RS_PACKET_SIZE) as u64;
    let losses = r.take_losses();
    assert_eq!(losses.len(), 1);
    assert_loss(&losses[0], 2000 - window, window, 0, false);
}
