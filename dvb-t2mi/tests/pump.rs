//! Integration tests for [`dvb_t2mi::pump`].
//!
//! Mirrors the in-module unit tests but exercises the public API as an
//! external crate consumer would.

#![cfg(feature = "ts")]

use dvb_common::crc32_mpeg2;
use dvb_t2mi::payload::AnyPayload;
use dvb_t2mi::pump::{Stats, T2miPump};

// ── TS constants (duplicated from the private pump module) ───────────────────

const TS_SYNC: u8 = 0x47;
const PUSI_MASK: u8 = 0x40;
const PID_HI_MASK: u8 = 0x1F;
const PAYLOAD_FLAG: u8 = 0x10;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a syntactically valid T2-MI packet with a correct CRC-32 trailer.
fn make_t2mi_packet(packet_type: u8, payload: &[u8]) -> Vec<u8> {
    let payload_len_bits = (payload.len() * 8) as u16;
    let mut pkt = Vec::with_capacity(6 + payload.len() + 4);
    pkt.push(packet_type);
    pkt.push(0x01); // packet_count
    pkt.push(0x00); // superframe_idx=0, rfu=0, t2mi_stream_id=0
    pkt.push(0x00); // rfu byte = 0
    pkt.extend_from_slice(&payload_len_bits.to_be_bytes());
    pkt.extend_from_slice(payload);
    let crc = crc32_mpeg2::compute(&pkt);
    pkt.extend_from_slice(&crc.to_be_bytes());
    pkt
}

/// Wrap T2-MI bytes in a single 188-byte MPEG-TS packet.
fn ts_packet(pid: u16, t2mi_data: &[u8], pusi: bool, pointer_field: u8) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = TS_SYNC;
    pkt[1] = if pusi { PUSI_MASK } else { 0 };
    pkt[1] |= ((pid >> 8) as u8) & PID_HI_MASK;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = PAYLOAD_FLAG;
    if pusi {
        pkt[4] = pointer_field;
        let start = 5 + pointer_field as usize;
        assert!(
            start + t2mi_data.len() <= 188,
            "T2-MI data too large for one TS packet"
        );
        pkt[start..start + t2mi_data.len()].copy_from_slice(t2mi_data);
    } else {
        let start = 4;
        assert!(
            start + t2mi_data.len() <= 188,
            "T2-MI data too large for one TS packet"
        );
        pkt[start..start + t2mi_data.len()].copy_from_slice(t2mi_data);
    }
    pkt
}

// ── (a) valid T2-MI packet in TS → one event, typed payload ─────────────────

/// A hand-built valid BBFrame T2-MI packet wrapped in a TS packet produces
/// one event whose payload dispatches to `AnyPayload::Bbframe`.
#[test]
fn ts_valid_packet_emits_one_event_with_typed_payload() {
    // BbframePayload: frame_idx(1) + plp_id(1) + intl_frame_start+rfu(1) = 3 bytes.
    let bbframe_payload = [0x00u8, 0x05, 0x00]; // frame_idx=0, plp_id=5, no intl
    let t2mi = make_t2mi_packet(0x00, &bbframe_payload);
    let pkt = ts_packet(0x0006, &t2mi, true, 0);

    let mut pump = T2miPump::new(0x0006);
    let events: Vec<_> = pump.feed_ts(&pkt).collect();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].packet_type(), 0x00);

    let payload = events[0].payload().expect("payload parse");
    match payload {
        AnyPayload::Bbframe(bb) => assert_eq!(bb.plp_id, 5),
        other => panic!("expected Bbframe, got {other:?}"),
    }

    let s = pump.stats();
    assert_eq!(s.ts_packets, 1);
    assert_eq!(s.t2mi_packets, 1);
    assert_eq!(s.crc_failures, 0);
    assert_eq!(s.malformed_packets, 0);
}

// ── (b) corrupted CRC → zero events, crc_failures=1 ─────────────────────────

/// Corrupting the CRC trailer causes the pump to drop the packet silently and
/// increment `crc_failures`.
#[test]
fn corrupted_crc_produces_no_event() {
    let payload = [0x00u8, 0x00, 0x00];
    let mut t2mi = make_t2mi_packet(0x00, &payload);
    *t2mi.last_mut().unwrap() ^= 0xFF; // flip one CRC byte

    let pkt = ts_packet(0x0006, &t2mi, true, 0);
    let mut pump = T2miPump::new(0x0006);
    let events: Vec<_> = pump.feed_ts(&pkt).collect();

    assert_eq!(events.len(), 0, "corrupted packet must not emit");
    assert_eq!(pump.stats().crc_failures, 1);
    // The reassembler did produce the packet — it just failed CRC gating.
    assert_eq!(pump.stats().t2mi_packets, 1);
}

// ── (c) feed_raw with packet split across two calls → one event ──────────────

/// A T2-MI packet split across two `feed_raw` calls is reassembled correctly
/// and produces exactly one event.
#[test]
fn feed_raw_packet_split_yields_one_event() {
    // Timestamp payload: 11 bytes, all zeros, packet_type=0x20.
    let ts_payload = [0x00u8; 11];
    let t2mi = make_t2mi_packet(0x20, &ts_payload);

    // Split after the 6-byte header.
    let (first, second) = t2mi.split_at(6);

    let mut pump = T2miPump::raw();

    let ev1: Vec<_> = pump.feed_raw(first).collect();
    assert_eq!(ev1.len(), 0, "first half: no complete packet yet");

    let ev2: Vec<_> = pump.feed_raw(second).collect();
    assert_eq!(ev2.len(), 1, "second half: completes the packet");

    let s = pump.stats();
    assert_eq!(s.t2mi_packets, 1);
    assert_eq!(s.crc_failures, 0);
    assert_eq!(s.malformed_packets, 0);
}

// ── (d) garbage TS packet → malformed counted, no panic ──────────────────────

/// Feeding a 188-byte slice whose first byte is not 0x47 (bad sync) counts
/// as malformed and does not panic.
#[test]
fn garbage_ts_packet_counted_no_panic() {
    let mut pump = T2miPump::new(0x0006);
    let garbage = [0x00u8; 188]; // 0x00 ≠ 0x47 → bad sync
    let events: Vec<_> = pump.feed_ts(&garbage).collect();
    assert_eq!(events.len(), 0);
    assert_eq!(pump.stats().malformed_packets, 1);
    assert_eq!(pump.stats().ts_packets, 1);
}

// ── (e) wrong-PID TS packet → ignored cheaply ────────────────────────────────

/// A valid TS packet on the wrong PID is ignored: only `ts_packets` moves.
#[test]
fn wrong_pid_packet_ignored_no_stats_movement() {
    let payload = [0x00u8, 0x00, 0x00];
    let t2mi = make_t2mi_packet(0x00, &payload);
    let pkt = ts_packet(0x0200, &t2mi, true, 0); // pump listens on 0x0006

    let mut pump = T2miPump::new(0x0006);
    let events: Vec<_> = pump.feed_ts(&pkt).collect();

    assert_eq!(events.len(), 0);
    let s = pump.stats();
    assert_eq!(s.ts_packets, 1);
    assert_eq!(s.t2mi_packets, 0);
    assert_eq!(s.crc_failures, 0);
    assert_eq!(s.malformed_packets, 0);
}

// ── additional: Stats default is all-zero ────────────────────────────────────

#[test]
fn stats_default_is_zero() {
    let pump = T2miPump::new(0x0100);
    assert_eq!(pump.stats(), Stats::default());
}

// ── additional: T2miEvent::header() + payload() lazy parsing ─────────────────

/// `header()` and `payload()` on an event both succeed and agree on fields.
#[test]
fn event_header_and_payload_agree() {
    // L1Current payload: frame_idx(1) + freq_source(2bits)+rfu(6bits)(1) = 2 bytes.
    let l1_payload = [0x07u8, 0x00]; // frame_idx=7, no freq_source, rfu=0
    let t2mi = make_t2mi_packet(0x10, &l1_payload);
    let pkt = ts_packet(0x0006, &t2mi, true, 0);

    let mut pump = T2miPump::new(0x0006);
    let events: Vec<_> = pump.feed_ts(&pkt).collect();
    assert_eq!(events.len(), 1);

    let hdr = events[0].header().expect("header");
    assert_eq!(hdr.packet_type as u8, 0x10);
    assert_eq!(hdr.payload_len_bits, 16); // 2 bytes * 8

    let payload = events[0].payload().expect("payload");
    assert!(
        matches!(payload, AnyPayload::L1Current(_)),
        "expected L1Current, got {payload:?}"
    );
}

// ── additional: multiple packets in one TS payload ───────────────────────────

/// Two back-to-back T2-MI packets in a single TS payload both emit events.
#[test]
fn two_t2mi_packets_in_one_ts_payload_both_emit() {
    let p1 = make_t2mi_packet(0x00, &[0x01u8, 0x00, 0x00]); // BBFrame
    let p2 = make_t2mi_packet(0x00, &[0x02u8, 0x00, 0x00]); // BBFrame

    let mut combined = Vec::new();
    combined.extend_from_slice(&p1);
    combined.extend_from_slice(&p2);

    let pkt = ts_packet(0x0006, &combined, true, 0);
    let mut pump = T2miPump::new(0x0006);
    let events: Vec<_> = pump.feed_ts(&pkt).collect();

    assert_eq!(events.len(), 2, "both packets must emit");
    assert_eq!(pump.stats().t2mi_packets, 2);
}

// ── additional: truncated TS input counted, no panic ──────────────────────────

/// Feeding a truncated TS packet (10 bytes, not a full 188-byte packet) to a
/// TS-mode pump counts as malformed and does not panic.
#[test]
fn truncated_ts_input_counted_no_panic() {
    let mut pump = T2miPump::new(0x0006);
    let truncated = [0x47u8; 10]; // good sync but too short
    let events: Vec<_> = pump.feed_ts(&truncated).collect();

    assert_eq!(events.len(), 0, "truncated packet must not emit");
    assert_eq!(pump.stats().ts_packets, 1);
    assert_eq!(pump.stats().malformed_packets, 1);
}
