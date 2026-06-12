use core::time::Duration;

use dvb_common::Serialize;
use dvb_si::mux::SectionPacketizer;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream};
use dvb_si::ts::{TsHeader, TS_PACKET_SIZE};

use crate::{Config, ConformanceMonitor, Indicator};

// ── Helpers ──────────────────────────────────────────────────────────────────

const PID_PAT: u16 = 0x0000;
const PID_NULL: u16 = 0x1FFF;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// Build a single 188-byte TS packet with the given header and payload.
fn make_ts_packet(
    pid: u16,
    cc: u8,
    pusi: bool,
    has_adaptation: bool,
    adaptation: &[u8],
    payload: &[u8],
) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    let header = TsHeader {
        tei: false,
        pusi,
        pid,
        scrambling: 0,
        has_adaptation: has_adaptation || !adaptation.is_empty(),
        has_payload: !payload.is_empty(),
        continuity_counter: cc & 0x0F,
    };
    header.serialize_into(&mut pkt[..4]).unwrap();
    let mut pos = 4usize;
    if has_adaptation || !adaptation.is_empty() {
        let af_len = adaptation.len() as u8;
        pkt[pos] = af_len;
        pos += 1;
        if af_len > 0 {
            pkt[pos..pos + adaptation.len()].copy_from_slice(adaptation);
            pos += adaptation.len();
        }
    }
    if !payload.is_empty() {
        pkt[pos..pos + payload.len().min(TS_PACKET_SIZE - pos)]
            .copy_from_slice(&payload[..payload.len().min(TS_PACKET_SIZE - pos)]);
    }
    pkt
}

/// Build a PAT section's wire bytes.
fn build_pat_section(program_map_pids: &[(u16, u16)]) -> Vec<u8> {
    let pat = PatSection {
        transport_stream_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: program_map_pids
            .iter()
            .map(|&(pn, pid)| PatEntry {
                program_number: pn,
                pid,
            })
            .collect(),
    };
    let mut buf = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut buf).unwrap();
    buf
}

/// Build a PMT section's wire bytes.
fn build_pmt_section(program_number: u16, pcr_pid: u16, es_pids: &[u16]) -> Vec<u8> {
    let pmt = PmtSection {
        program_number,
        version_number: 0,
        current_next_indicator: true,
        pcr_pid,
        program_info: Default::default(),
        streams: es_pids
            .iter()
            .map(|&pid| PmtStream {
                stream_type: dvb_si::tables::pmt::StreamType::Mpeg2Video,
                elementary_pid: pid,
                es_info: Default::default(),
            })
            .collect(),
    };
    let mut buf = vec![0u8; pmt.serialized_len()];
    pmt.serialize_into(&mut buf).unwrap();
    buf
}

/// Packetize a section into 188-byte TS packets.
fn packetize_section(pid: u16, section: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
    let mut pktizer = SectionPacketizer::new(pid);
    pktizer.packetize(&[section])
}

/// Feed packets to the monitor and collect all events.
fn feed_all(
    monitor: &mut ConformanceMonitor,
    packets: &[[u8; TS_PACKET_SIZE]],
    base_t: Duration,
    delta: Duration,
) -> Vec<crate::ConformanceEvent> {
    let mut all = Vec::new();
    for (i, pkt) in packets.iter().enumerate() {
        let t = base_t + delta * i as u32;
        let events = monitor.feed(pkt, t);
        all.extend(events.to_vec());
    }
    all
}

fn has_indicator(events: &[crate::ConformanceEvent], indicator: Indicator) -> bool {
    events.iter().any(|e| e.indicator == indicator)
}

// ── 1.2 Sync_byte_error ─────────────────────────────────────────────────────

#[test]
fn sync_byte_error_trips_on_bad_sync() {
    let mut monitor = ConformanceMonitor::new();
    // Build a valid packet then corrupt the sync byte.
    let mut pkt = make_ts_packet(PID_PAT, 0, true, false, &[], &[0x00]);
    pkt[0] = 0x00; // bad sync

    let events = monitor.feed(&pkt, ms(0));
    assert!(has_indicator(events, Indicator::SyncByteError));
}

#[test]
fn sync_byte_error_absent_on_good_sync() {
    let mut monitor = ConformanceMonitor::new();
    // First acquire sync (5 good packets).
    for i in 0u8..6 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        let events = monitor.feed(&pkt, ms(i as u64));
        assert!(!has_indicator(events, Indicator::SyncByteError));
    }
}

// ── 1.1 TS_sync_loss ────────────────────────────────────────────────────────

#[test]
fn ts_sync_loss_after_bad_run_then_reacquire() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync first.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    assert!(monitor.stats().in_sync);

    // Feed 2 bad-sync packets (default sync_loss_packets = 2).
    let mut bad = [0u8; TS_PACKET_SIZE];
    bad[0] = 0x00;
    let events1 = monitor.feed(&bad, ms(5));
    assert!(!has_indicator(events1, Indicator::TsSyncLoss));
    let events2 = monitor.feed(&bad, ms(6));
    assert!(has_indicator(events2, Indicator::TsSyncLoss));
    assert!(!monitor.stats().in_sync);

    // Re-acquire with 5 good packets.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, (5 + i) & 0x0F, false, false, &[], &[]);
        monitor.feed(&pkt, ms(7 + i as u64));
    }
    assert!(monitor.stats().in_sync);
}

#[test]
fn ts_sync_loss_not_emitted_while_in_sync() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..10 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        let events = monitor.feed(&pkt, ms(i as u64));
        assert!(!has_indicator(events, Indicator::TsSyncLoss));
    }
}

// ── 1.4 Continuity_count_error ──────────────────────────────────────────────

#[test]
fn cc_error_trips_on_jump() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // Feed cc=3, then cc=5 (skipped 4).
    let pkt1 = make_ts_packet(0x100, 3, false, false, &[], &[]);
    let pkt2 = make_ts_packet(0x100, 5, false, false, &[], &[]);
    monitor.feed(&pkt1, ms(5));
    let events = monitor.feed(&pkt2, ms(6));
    assert!(has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_correct_increment_no_error() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, true, false, &[], &[0xAB]);
        monitor.feed(&pkt, ms(i as u64));
    }
    // Sequential increment: last_cc=4, next with payload should be 5.
    let pkt = make_ts_packet(0x100, 5, true, false, &[], &[0xAB]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_single_duplicate_is_legal() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, true, false, &[], &[0xAB]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // cc=4 again (single duplicate with payload).
    let pkt = make_ts_packet(0x100, 4, true, false, &[], &[0xCD]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_double_duplicate_is_error() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, true, false, &[], &[0xAB]);
        monitor.feed(&pkt, ms(i as u64));
    }
    // First duplicate — legal.
    let pkt1 = make_ts_packet(0x100, 4, true, false, &[], &[0xCD]);
    monitor.feed(&pkt1, ms(5));
    // Second duplicate — error.
    let pkt2 = make_ts_packet(0x100, 4, true, false, &[], &[0xCD]);
    let events = monitor.feed(&pkt2, ms(6));
    assert!(has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_no_payload_holds_cc() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    // No-payload packet: CC must not advance.
    // Adaptation-only: has_adaptation=true, has_payload=false.
    let pkt = make_ts_packet(0x100, 4, false, true, &[0x00], &[]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_discontinuity_indicator_skips_check() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    // Packet with discontinuity_indicator=1: adaptation field flags byte 0x80.
    let af = [0x80]; // discontinuity_indicator set
    let pkt = make_ts_packet(0x100, 7, false, true, &af, &[]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_null_pid_skipped() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    // Null packets can have any CC — should not trigger.
    let pkt = make_ts_packet(PID_NULL, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::ContinuityCountError));
}

// ── 1.3.a PAT_error_2 ───────────────────────────────────────────────────────

#[test]
fn pat_error_wrong_table_id() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // Build a section with wrong table_id on PID 0x0000.
    // Use a CAT table_id (0x01) section on the PAT PID.
    let mut section = build_pat_section(&[(1, 0x100)]);
    section[0] = 0x01; // wrong table_id

    let packets = packetize_section(PID_PAT, &section);
    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(has_indicator(&events, Indicator::PatError2));
}

#[test]
fn pat_error_scrambling() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // Build a PAT packet with scrambling != 0.
    let mut pkt = make_ts_packet(PID_PAT, 0, true, false, &[], &[0x00]);
    pkt[3] = (pkt[3] & !0xC0) | 0x40; // scrambling = 01

    let events = monitor.feed(&pkt, ms(5));
    assert!(has_indicator(events, Indicator::PatError2));
}

#[test]
fn pat_error_timeout() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // Feed a packet on another PID past the PAT timeout (500 ms).
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(600));
    assert!(has_indicator(events, Indicator::PatError2));
}

#[test]
fn pat_compliant_no_error() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync + feed a PAT section.
    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);

    // Acquire sync.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // Feed the PAT packets.
    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(!has_indicator(&events, Indicator::PatError2));

    // Feed another packet well within timeout.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(20));
    assert!(!has_indicator(events, Indicator::PatError2));
}

// ── 1.5.a PMT_error_2 ───────────────────────────────────────────────────────

#[test]
fn pmt_error_timeout() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync + feed a PAT that references PMT PID 0x100.
    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);

    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Now the monitor knows about PMT PID 0x100. Feed a packet on another PID
    // past the PMT timeout.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(600));
    assert!(has_indicator(events, Indicator::PmtError2));
}

#[test]
fn pmt_error_scrambling() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync + feed PAT.
    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed a packet on the PMT PID with scrambling.
    let mut pkt = make_ts_packet(0x100, 5, false, false, &[], &[]);
    pkt[3] = (pkt[3] & !0xC0) | 0x40; // scrambling = 01
    let events = monitor.feed(&pkt, ms(10));
    assert!(has_indicator(events, Indicator::PmtError2));
}

#[test]
fn pmt_compliant_no_error() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync + feed PAT + PMT.
    let pat_section = build_pat_section(&[(1, 0x100)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);

    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    feed_all(&mut monitor, &pat_packets, ms(5), ms(1));

    let pmt_section = build_pmt_section(1, 0x1FFF, &[]);
    let pmt_packets = packetize_section(0x100, &pmt_section);
    let events = feed_all(&mut monitor, &pmt_packets, ms(10), ms(1));
    assert!(!has_indicator(&events, Indicator::PmtError2));
}

// ── 1.6 PID_error ────────────────────────────────────────────────────────────

#[test]
fn pid_error_timeout() {
    let config = Config {
        pid_error_period: secs(1),
        ..Config::default()
    };
    let mut monitor = ConformanceMonitor::with_config(config);

    // Acquire sync + feed PAT + PMT referencing ES PID 0x200.
    let pat_section = build_pat_section(&[(1, 0x100)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);

    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    feed_all(&mut monitor, &pat_packets, ms(5), ms(1));

    let pmt_section = build_pmt_section(1, 0x1FFF, &[0x200]);
    let pmt_packets = packetize_section(0x100, &pmt_section);
    feed_all(&mut monitor, &pmt_packets, ms(10), ms(1));

    // Feed a packet on another PID past the pid_error_period (1 s).
    let pkt = make_ts_packet(0x300, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, secs(2));
    assert!(has_indicator(events, Indicator::PidError));
}

#[test]
fn pid_compliant_no_error() {
    let config = Config {
        pid_error_period: secs(5),
        ..Config::default()
    };
    let mut monitor = ConformanceMonitor::with_config(config);

    // Acquire sync + feed PAT + PMT referencing ES PID 0x200.
    let pat_section = build_pat_section(&[(1, 0x100)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);

    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    feed_all(&mut monitor, &pat_packets, ms(5), ms(1));

    let pmt_section = build_pmt_section(1, 0x1FFF, &[0x200]);
    let pmt_packets = packetize_section(0x100, &pmt_section);
    feed_all(&mut monitor, &pmt_packets, ms(10), ms(1));

    // Feed the referenced ES PID well within the period.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(500));
    assert!(!has_indicator(events, Indicator::PidError));
}

// ── Presence-timeout emit-once semantics ─────────────────────────────────────

#[test]
fn pat_timeout_emits_once_not_per_packet() {
    let mut monitor = ConformanceMonitor::new();

    // Acquire sync.
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }

    // Feed packets past the PAT timeout — only the FIRST should emit.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let e1 = monitor.feed(&pkt, ms(600)).to_vec();
    let e2 = monitor.feed(&pkt, ms(700)).to_vec();
    let e3 = monitor.feed(&pkt, ms(800)).to_vec();

    assert_eq!(
        e1.iter()
            .filter(|e| e.indicator == Indicator::PatError2)
            .count(),
        1
    );
    assert!(!has_indicator(&e2, Indicator::PatError2));
    assert!(!has_indicator(&e3, Indicator::PatError2));
}

// ── Sync suppression: other indicators suppressed while not in sync ──────────

#[test]
fn other_indicators_suppressed_while_not_in_sync() {
    let mut monitor = ConformanceMonitor::new();

    // Never acquire sync — all packets have bad sync byte.
    let bad = [0u8; TS_PACKET_SIZE]; // sync byte = 0x00
    for i in 0u8..10 {
        let events = monitor.feed(&bad, ms(i as u64));
        // Only SyncByteError and TsSyncLoss should appear, never CC/PAT/PMT/PID.
        for e in events {
            assert!(
                e.indicator == Indicator::SyncByteError || e.indicator == Indicator::TsSyncLoss,
                "unexpected indicator {:?} while not in sync",
                e.indicator,
            );
        }
    }
}

// ── Stats ────────────────────────────────────────────────────────────────────

#[test]
fn stats_track_packets_and_events() {
    let mut monitor = ConformanceMonitor::new();
    assert_eq!(monitor.stats().packets, 0);
    assert_eq!(monitor.stats().events, 0);

    let bad = [0u8; TS_PACKET_SIZE];
    monitor.feed(&bad, ms(0));
    assert_eq!(monitor.stats().packets, 1);
    // At least one SyncByteError.
    assert!(monitor.stats().events >= 1);
}
