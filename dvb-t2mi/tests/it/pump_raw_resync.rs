//! Raw-mode T2-MI framing, resync and statistics — ETSI TS 102 773 §5.1/§6.1.1.
//!
//! Raw mode has no PUSI to resynchronise from, so the pump buffers bytes and
//! scans forward for a CRC-valid packet start after a CRC failure (#1095).

#![cfg(feature = "ts")]

use broadcast_common::crc32_mpeg2;
use dvb_t2mi::pump::T2miPump;
use mpeg_ts::pid::Pid;

const TS_SYNC_BYTE: u8 = 0x47;

fn t2mi_packet(packet_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut pkt = vec![packet_type, 0x01, 0x00, 0x00];
    pkt.extend_from_slice(&((payload.len() * 8) as u16).to_be_bytes());
    pkt.extend_from_slice(payload);
    let crc = crc32_mpeg2::compute(&pkt);
    pkt.extend_from_slice(&crc.to_be_bytes());
    pkt
}

fn ts_packet(pid: Pid, payload: &[u8], pusi: bool, cc: u8, tei: bool) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = TS_SYNC_BYTE;
    pkt[1] = if pusi { 0x40 } else { 0x00 } | ((pid.value() >> 8) as u8 & 0x1F);
    if tei {
        pkt[1] |= 0x80;
    }
    pkt[2] = (pid.value() & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F);
    let start = if pusi {
        pkt[4] = 0x00;
        5
    } else {
        4
    };
    pkt[start..start + payload.len()].copy_from_slice(payload);
    pkt
}

fn deflated_corrupt() -> (Vec<u8>, usize) {
    let mut corrupt = t2mi_packet(0x20, &[0x5A; 300]);
    let bits = u16::from_be_bytes([corrupt[4], corrupt[5]]);
    let deflated = bits - 1024;
    corrupt[4] = (deflated >> 8) as u8;
    corrupt[5] = deflated as u8;
    for byte in corrupt.iter_mut().skip(6) {
        *byte = 0x5A;
    }
    let mis_cut_len = 6 + (deflated as usize).div_ceil(8) + 4;
    (corrupt, mis_cut_len)
}

/// A 10-byte packet the resync scan can never treat as a candidate: neither
/// its own header nor any header straddling it is plausible.
fn implausible_corrupt() -> Vec<u8> {
    vec![0x42, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]
}

#[test]
fn a_packet_split_at_any_offset_yields_one_identical_event() {
    let packet = t2mi_packet(0x20, &[0x00u8; 11]);
    for split in 1..packet.len() {
        let mut pump = T2miPump::raw();
        let first: Vec<_> = pump.feed_raw(&packet[..split]).collect();
        let second: Vec<_> = pump.feed_raw(&packet[split..]).collect();
        assert_eq!(first.len() + second.len(), 1, "split at byte {split}");
        let event = first.into_iter().chain(second).next().unwrap();
        assert_eq!(&event.bytes()[..], &packet[..], "split at byte {split}");
        assert_eq!(pump.stats().t2mi_packets, 1);
        assert_eq!(pump.stats().crc_failures, 0);
    }
}

#[test]
fn a_packet_split_after_a_crc_failing_prefix_is_recovered_whole() {
    let corrupt = implausible_corrupt();
    let good = t2mi_packet(0x20, &[0x00u8; 11]);
    let split = 5;
    let mut stream = corrupt.clone();
    stream.extend_from_slice(&good[..split]);

    let mut pump = T2miPump::raw();
    let first: Vec<_> = pump.feed_raw(&stream).collect();
    assert!(first.is_empty(), "nothing valid may be emitted yet");
    assert_eq!(pump.stats().crc_failures, 1);
    assert_eq!(pump.stats().raw_resyncs, 1, "{:?}", pump.stats());

    let second: Vec<_> = pump.feed_raw(&good[split..]).collect();
    assert_eq!(second.len(), 1);
    assert_eq!(&second[0].bytes()[..], &good[..]);
    assert_eq!(pump.stats().raw_resyncs, 2, "{:?}", pump.stats());
    assert_eq!(pump.stats().crc_failures, 1);
}

#[test]
fn event_packet_type_is_the_wire_header_byte() {
    for packet_type in [0x00u8, 0x20, 0x33, 0x42] {
        let mut pump = T2miPump::raw();
        let packet = t2mi_packet(packet_type, &[0x5A; 4]);
        let events: Vec<_> = pump.feed_raw(&packet).collect();
        assert_eq!(events.len(), 1, "packet_type {packet_type:#04x}");
        assert_eq!(events[0].packet_type(), packet_type);
    }
}

#[test]
fn each_raw_resync_is_counted() {
    let good = t2mi_packet(0x20, &[0x00u8; 11]);
    let (corrupt, _) = deflated_corrupt();

    let mut pump = T2miPump::raw();
    let mut first = corrupt.clone();
    first.extend_from_slice(&good);
    let events: Vec<_> = pump.feed_raw(&first).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(&events[0].bytes()[..], &good[..]);
    assert_eq!(pump.stats().raw_resyncs, 1);

    let mut second = corrupt;
    second.extend_from_slice(&good);
    let events: Vec<_> = pump.feed_raw(&second).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(&events[0].bytes()[..], &good[..]);

    let stats = pump.stats();
    assert_eq!(stats.raw_resyncs, 2);
    assert_eq!(stats.crc_failures, 2);
}

#[test]
fn a_found_packet_leaves_a_private_type_neighbour_to_the_reassembler() {
    let corrupt = implausible_corrupt();
    let public = t2mi_packet(0x20, &[0x11u8; 11]);
    // 0x42 is not an allocated packet_type: the resync scan rejects it as a
    // candidate, but the reassembler still frames it and its CRC validates.
    let private = t2mi_packet(0x42, &[0x22u8; 11]);

    let mut pump = T2miPump::raw();
    let first: Vec<_> = pump.feed_raw(&corrupt).collect();
    assert!(first.is_empty());
    assert_eq!(pump.stats().crc_failures, 1);
    assert_eq!(pump.stats().raw_resyncs, 0);

    let mut stream = public.clone();
    stream.extend_from_slice(&private);
    let events: Vec<_> = pump.feed_raw(&stream).collect();
    assert_eq!(events.len(), 2);
    assert_eq!(&events[0].bytes()[..], &public[..]);
    assert_eq!(&events[1].bytes()[..], &private[..]);
    assert_eq!(events[1].packet_type(), 0x42);
    assert_eq!(pump.stats().raw_resyncs, 1, "{:?}", pump.stats());
    assert_eq!(pump.stats().t2mi_packets, 2, "{:?}", pump.stats());
}

#[test]
fn a_crc_failing_tail_after_a_found_packet_is_counted() {
    let corrupt = implausible_corrupt();
    let public = t2mi_packet(0x20, &[0x11u8; 11]);
    let mut private_bad_crc = t2mi_packet(0x42, &[0x22u8; 11]);
    *private_bad_crc.last_mut().unwrap() ^= 0xFF;

    let mut pump = T2miPump::raw();
    assert!(pump.feed_raw(&corrupt).next().is_none());
    assert_eq!(pump.stats().crc_failures, 1);

    let mut stream = public.clone();
    stream.extend_from_slice(&private_bad_crc);
    let events: Vec<_> = pump.feed_raw(&stream).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(&events[0].bytes()[..], &public[..]);
    assert_eq!(pump.stats().crc_failures, 2, "{:?}", pump.stats());
    assert_eq!(pump.stats().t2mi_packets, 2, "{:?}", pump.stats());
}

#[test]
fn ts_and_raw_mode_statistics_are_distinct_counters() {
    let mut pump = T2miPump::new(Pid::from_const(0x0006));
    let payload = [0x00u8, 0x00, 0x00];
    let t2mi = t2mi_packet(0x00, &payload);

    for _ in 0..3 {
        let pkt = ts_packet(Pid::from_const(0x0006), &t2mi, true, 0, true);
        assert!(pump.feed_ts(&pkt).next().is_none());
    }
    for _ in 0..2 {
        assert!(pump.feed_ts(&[0x00u8; 188]).next().is_none());
    }
    assert!(pump.feed_raw(&[0x00u8; 4]).next().is_none());

    let stats = pump.stats();
    assert_eq!(stats.ts_packets, 5);
    assert_eq!(stats.tei_dropped, 3);
    assert_eq!(stats.malformed_packets, 3);
    assert_eq!(stats.t2mi_packets, 0);
    assert_eq!(stats.crc_failures, 0);

    let mut raw = T2miPump::raw();
    let pkt = ts_packet(Pid::from_const(0x0006), &t2mi, true, 0, false);
    assert!(raw.feed_ts(&pkt).next().is_none());
    assert_eq!(raw.stats().malformed_packets, 1);
    assert_eq!(raw.stats().ts_packets, 0);
}
