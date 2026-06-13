use core::time::Duration;

use dvb_common::Serialize;
use dvb_si::descriptors::any::DescriptorLoop;
use dvb_si::mux::SectionPacketizer;
use dvb_si::tables::eit::{EitKind, EitSection};
use dvb_si::tables::nit::{NitKind, NitSection};
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream};
use dvb_si::tables::sdt::{SdtKind, SdtSection};
use dvb_si::tables::tdt::TdtSection;
use dvb_si::ts::{TsHeader, TS_PACKET_SIZE};

use crate::{Config, ConformanceMonitor, Indicator};

// ── Helpers ──────────────────────────────────────────────────────────────────

const PID_PAT: u16 = 0x0000;
const PID_CAT: u16 = 0x0001;
const PID_NIT: u16 = 0x0010;
const PID_SDT_BAT: u16 = 0x0011;
const PID_EIT: u16 = 0x0012;
const PID_TDT_TOT: u16 = 0x0014;
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

/// Build a TS packet with TEI bit set.
fn make_ts_packet_with_tei(pid: u16, cc: u8) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    let header = TsHeader {
        tei: true,
        pusi: false,
        pid,
        scrambling: 0,
        has_adaptation: false,
        has_payload: false,
        continuity_counter: cc & 0x0F,
    };
    header.serialize_into(&mut pkt[..4]).unwrap();
    pkt
}

/// Build a TS packet with scrambling != 0.
fn make_ts_packet_with_scrambling(pid: u16, cc: u8, scrambling: u8) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    let header = TsHeader {
        tei: false,
        pusi: false,
        pid,
        scrambling,
        has_adaptation: false,
        has_payload: true,
        continuity_counter: cc & 0x0F,
    };
    header.serialize_into(&mut pkt[..4]).unwrap();
    pkt[4] = 0xAB; // some payload
    pkt
}

/// Encode a 42-bit PCR value (33-bit base + 9-bit extension) into 6 bytes.
/// ISO/IEC 13818-1 §2.4.3.5 — PCR layout:
///   byte 0-3: base[32:0] (33 bits, MSB first)
///   byte 4: reserved(6) | extension[8:7]
///   byte 5: extension[7:0]
fn encode_pcr(base: u64, extension: u16) -> [u8; 6] {
    let mut out = [0u8; 6];
    out[0] = (base >> 25) as u8;
    out[1] = (base >> 17) as u8;
    out[2] = (base >> 9) as u8;
    out[3] = (base >> 1) as u8;
    out[4] = ((base & 1) << 7) as u8 | 0x7E | ((extension >> 8) & 1) as u8;
    out[5] = (extension & 0xFF) as u8;
    out
}

/// Build adaptation field data (WITHOUT the leading length byte —
/// `make_ts_packet` adds that) carrying a PCR, optionally setting the
/// discontinuity_indicator. When `no_payload` is true the adaptation field
/// fills the rest of the packet (af_length = 183) so the data must be padded
/// to 183 bytes.
fn make_pcr_adaptation(
    pcr_base: u64,
    pcr_extension: u16,
    discontinuity: bool,
    no_payload: bool,
) -> Vec<u8> {
    // Flags byte: discontinuity_indicator(bit7) | random_access(bit6) |
    //   ES_priority(bit5) | PCR_flag(bit4) | OPCR_flag(bit3) |
    //   splicing_point(bit2) | transport_private_data(bit1) | extension(bit0)
    let flags: u8 = 0x10 // PCR flag
        | if discontinuity { 0x80 } else { 0x00 };
    let pcr_bytes = encode_pcr(pcr_base, pcr_extension);
    // adaptation_field_data = flags(1) + pcr(6) = 7 bytes
    let data_len: usize = 7;
    let af_data_len = if no_payload {
        // Fill the rest of the 188-byte packet: 188 - 4(header) - 1(af_len byte) = 183
        183usize
    } else {
        data_len
    };
    let mut af = vec![0xFF; af_data_len];
    af[0] = flags;
    af[1..7].copy_from_slice(&pcr_bytes);
    af
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
    let pmt = PmtSection::new(
        program_number,
        0,
        true,
        0,
        0,
        pcr_pid,
        Default::default(),
        es_pids
            .iter()
            .map(|&pid| PmtStream {
                stream_type: dvb_si::tables::pmt::StreamType::Mpeg2Video,
                elementary_pid: pid,
                es_info: Default::default(),
            })
            .collect(),
    );
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

/// Acquire sync by feeding 5 good packets.
fn acquire_sync(monitor: &mut ConformanceMonitor) {
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
}

/// Build a minimal PES header that signals PTS present (PTS_DTS_flags = 0b10).
/// Layout: 00 00 01 [stream_id] [PES_packet_length_hi] [PES_packet_length_lo]
///         [flags_byte: '10' + scrambling + priority + ...]
///         [PTS_DTS_flags_byte + other flags]
///         [5 bytes PTS placeholder]
fn make_pes_header_with_pts(stream_id: u8, pes_packet_length: u16) -> Vec<u8> {
    vec![
        0x00, // prefix byte 0
        0x00, // prefix byte 1
        0x01, // prefix byte 2
        stream_id,
        (pes_packet_length >> 8) as u8,
        (pes_packet_length & 0xFF) as u8,
        // Byte 6: marker bits '10' + PES_scrambling_control(00) +
        //   PES_priority(0) + data_alignment_indicator(0) + copyright(0) +
        //   original_or_copy(0) = 0b1000_0000 = 0x80
        0x80,
        // Byte 7: PTS_DTS_flags(10) + ESCR_flag(0) + ES_rate_flag(0) +
        //   DSM_trick_mode(0) + additional_copy_info(0) + PES_CRC(0) +
        //   PES_extension(0) = 0b1000_0000 = 0x80
        0x80,
        // Byte 8: PES_header_data_length (5 bytes for PTS only)
        5,
        // 5 bytes of PTS placeholder (33-bit PTS encoded across 5 bytes).
        // Use value 0 for simplicity — we only care about presence, not value.
        0x21, // '0010' + PTS[32:30] + marker '1'
        0x00, // PTS[29:22]
        0x01, // PTS[21:15] + marker '1' (partial)
        0x00, // PTS[14:7]
        0x01, // PTS[6:0] + marker '1'
    ]
}

/// Build a PES header WITHOUT PTS (PTS_DTS_flags = 0b00).
fn make_pes_header_without_pts(stream_id: u8, pes_packet_length: u16) -> Vec<u8> {
    vec![
        0x00,
        0x00,
        0x01,
        stream_id,
        (pes_packet_length >> 8) as u8,
        (pes_packet_length & 0xFF) as u8,
        0x80, // '10' marker
        0x00, // PTS_DTS_flags = 00
        0,    // PES_header_data_length = 0
    ]
}

/// Set up a monitor with sync + PAT + PMT referencing ES PID `es_pid`.
/// Returns the PMT PID.
fn setup_monitor_with_es(monitor: &mut ConformanceMonitor, es_pid: u16) -> u16 {
    let pmt_pid: u16 = 0x0100;
    acquire_sync(monitor);

    let pat_section = build_pat_section(&[(1, pmt_pid)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);
    feed_all(monitor, &pat_packets, ms(5), ms(1));

    let pmt_section = build_pmt_section(1, 0x1FFF, &[es_pid]);
    let pmt_packets = packetize_section(pmt_pid, &pmt_section);
    feed_all(monitor, &pmt_packets, ms(10), ms(1));

    pmt_pid
}

// ── 1.2 Sync_byte_error ─────────────────────────────────────────────────────

#[test]
fn sync_byte_error_trips_on_bad_sync() {
    let mut monitor = ConformanceMonitor::new();
    let mut pkt = make_ts_packet(PID_PAT, 0, true, false, &[], &[0x00]);
    pkt[0] = 0x00; // bad sync

    let events = monitor.feed(&pkt, ms(0));
    assert!(has_indicator(events, Indicator::SyncByteError));
}

#[test]
fn sync_byte_error_absent_on_good_sync() {
    let mut monitor = ConformanceMonitor::new();
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

    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, false, false, &[], &[]);
        monitor.feed(&pkt, ms(i as u64));
    }
    assert!(monitor.stats().in_sync);

    let mut bad = [0u8; TS_PACKET_SIZE];
    bad[0] = 0x00;
    let events1 = monitor.feed(&bad, ms(5));
    assert!(!has_indicator(events1, Indicator::TsSyncLoss));
    let events2 = monitor.feed(&bad, ms(6));
    assert!(has_indicator(events2, Indicator::TsSyncLoss));
    assert!(!monitor.stats().in_sync);

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
    acquire_sync(&mut monitor);

    // Establish a payload-bearing run on a fresh PID with correctly incrementing
    // continuity counters: 0 → 1 must NOT trip (control).
    let init = make_ts_packet(0x200, 0, false, false, &[], &[0xAB]);
    monitor.feed(&init, ms(5));
    let ok = make_ts_packet(0x200, 1, false, false, &[], &[0xAB]);
    assert!(
        !has_indicator(monitor.feed(&ok, ms(6)), Indicator::ContinuityCountError),
        "a correct +1 increment on a payload packet must not trip"
    );

    // Now a genuine jump on a payload packet: last_cc = 1, expected 2, got 5.
    let jump = make_ts_packet(0x200, 5, false, false, &[], &[0xAB]);
    let events = monitor.feed(&jump, ms(7));
    assert!(has_indicator(events, Indicator::ContinuityCountError));
}

#[test]
fn cc_correct_increment_no_error() {
    let mut monitor = ConformanceMonitor::new();
    for i in 0u8..5 {
        let pkt = make_ts_packet(0x100, i, true, false, &[], &[0xAB]);
        monitor.feed(&pkt, ms(i as u64));
    }
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
    let pkt1 = make_ts_packet(0x100, 4, true, false, &[], &[0xCD]);
    monitor.feed(&pkt1, ms(5));
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
    let pkt = make_ts_packet(PID_NULL, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::ContinuityCountError));
}

// ── 1.3.a PAT_error_2 ───────────────────────────────────────────────────────

#[test]
fn pat_error_wrong_table_id() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let mut section = build_pat_section(&[(1, 0x100)]);
    section[0] = 0x01; // wrong table_id

    let packets = packetize_section(PID_PAT, &section);
    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(has_indicator(&events, Indicator::PatError2));
}

#[test]
fn pat_error_scrambling() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let mut pkt = make_ts_packet(PID_PAT, 0, true, false, &[], &[0x00]);
    pkt[3] = (pkt[3] & !0xC0) | 0x40; // scrambling = 01

    let events = monitor.feed(&pkt, ms(5));
    assert!(has_indicator(events, Indicator::PatError2));
}

#[test]
fn pat_error_timeout() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(600));
    assert!(has_indicator(events, Indicator::PatError2));
}

#[test]
fn pat_compliant_no_error() {
    let mut monitor = ConformanceMonitor::new();

    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);

    acquire_sync(&mut monitor);

    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(!has_indicator(&events, Indicator::PatError2));

    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(20));
    assert!(!has_indicator(events, Indicator::PatError2));
}

// ── 1.5.a PMT_error_2 ───────────────────────────────────────────────────────

#[test]
fn pmt_error_timeout() {
    let mut monitor = ConformanceMonitor::new();

    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);

    acquire_sync(&mut monitor);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(600));
    assert!(has_indicator(events, Indicator::PmtError2));
}

#[test]
fn pmt_error_scrambling() {
    let mut monitor = ConformanceMonitor::new();

    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);
    acquire_sync(&mut monitor);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    let mut pkt = make_ts_packet(0x100, 5, false, false, &[], &[]);
    pkt[3] = (pkt[3] & !0xC0) | 0x40; // scrambling = 01
    let events = monitor.feed(&pkt, ms(10));
    assert!(has_indicator(events, Indicator::PmtError2));
}

#[test]
fn pmt_compliant_no_error() {
    let mut monitor = ConformanceMonitor::new();

    let pat_section = build_pat_section(&[(1, 0x100)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);

    acquire_sync(&mut monitor);
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

    let pat_section = build_pat_section(&[(1, 0x100)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);

    acquire_sync(&mut monitor);
    feed_all(&mut monitor, &pat_packets, ms(5), ms(1));

    let pmt_section = build_pmt_section(1, 0x1FFF, &[0x200]);
    let pmt_packets = packetize_section(0x100, &pmt_section);
    feed_all(&mut monitor, &pmt_packets, ms(10), ms(1));

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

    let pat_section = build_pat_section(&[(1, 0x100)]);
    let pat_packets = packetize_section(PID_PAT, &pat_section);

    acquire_sync(&mut monitor);
    feed_all(&mut monitor, &pat_packets, ms(5), ms(1));

    let pmt_section = build_pmt_section(1, 0x1FFF, &[0x200]);
    let pmt_packets = packetize_section(0x100, &pmt_section);
    feed_all(&mut monitor, &pmt_packets, ms(10), ms(1));

    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(500));
    assert!(!has_indicator(events, Indicator::PidError));
}

// ── Presence-timeout emit-once semantics ─────────────────────────────────────

#[test]
fn pat_timeout_emits_once_not_per_packet() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

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

    let bad = [0u8; TS_PACKET_SIZE]; // sync byte = 0x00
    for i in 0u8..10 {
        let events = monitor.feed(&bad, ms(i as u64));
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
    assert!(monitor.stats().events >= 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ── Priority 2 tests ─────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

// ── 2.1 Transport_error ──────────────────────────────────────────────────────

#[test]
fn transport_error_trips_on_tei_set() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pkt = make_ts_packet_with_tei(0x100, 5);
    let events = monitor.feed(&pkt, ms(5));
    assert!(has_indicator(events, Indicator::TransportError));
}

#[test]
fn transport_error_absent_on_tei_clear() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pkt = make_ts_packet(0x100, 5, false, false, &[], &[]);
    let events = monitor.feed(&pkt, ms(5));
    assert!(!has_indicator(events, Indicator::TransportError));
}

// ── 2.2 CRC_error ────────────────────────────────────────────────────────────

#[test]
fn crc_error_trips_on_corrupted_pat() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Build a valid PAT section then corrupt a payload byte (not sync/header).
    let mut section = build_pat_section(&[(1, 0x100)]);
    // The section has: header (3) + extension (5) + entries + CRC (4).
    // Corrupt a byte in the entries area (well before the CRC).
    if section.len() > 12 {
        section[10] ^= 0xFF;
    }

    let packets = packetize_section(PID_PAT, &section);
    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(has_indicator(&events, Indicator::CrcError));
}

#[test]
fn crc_error_absent_on_valid_pat() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let section = build_pat_section(&[(1, 0x100)]);
    let packets = packetize_section(PID_PAT, &section);
    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(!has_indicator(&events, Indicator::CrcError));
}

// ── 2.3a PCR_repetition_error ────────────────────────────────────────────────

#[test]
fn pcr_repetition_error_trips_on_large_gap() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pcr_pid: u16 = 0x0200;
    // PCR base values are in 90kHz units; as_27mhz() = base * 300.
    // 1s = 90000 base units, 2s = 180000.
    let af1 = make_pcr_adaptation(90000, 0, false, true);
    let pkt1 = make_ts_packet(pcr_pid, 0, false, true, &af1, &[]);
    let af2 = make_pcr_adaptation(180000, 0, false, true);
    let pkt2 = make_ts_packet(pcr_pid, 1, false, true, &af2, &[]);

    monitor.feed(&pkt1, ms(0));
    // Second PCR at t=150ms — gap of 150ms exceeds 100ms default limit.
    let events = monitor.feed(&pkt2, ms(150));
    assert!(has_indicator(events, Indicator::PcrRepetitionError));
}

#[test]
fn pcr_repetition_error_absent_within_limit() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pcr_pid: u16 = 0x0200;
    let af1 = make_pcr_adaptation(90000, 0, false, true);
    let pkt1 = make_ts_packet(pcr_pid, 0, false, true, &af1, &[]);
    let af2 = make_pcr_adaptation(180000, 0, false, true);
    let pkt2 = make_ts_packet(pcr_pid, 1, false, true, &af2, &[]);

    monitor.feed(&pkt1, ms(0));
    // Second PCR at t=50ms — gap of 50ms within 100ms limit.
    let events = monitor.feed(&pkt2, ms(50));
    assert!(!has_indicator(events, Indicator::PcrRepetitionError));
}

// ── 2.3b PCR_discontinuity_indicator_error ───────────────────────────────────

#[test]
fn pcr_discontinuity_error_trips_on_large_delta_without_indicator() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pcr_pid: u16 = 0x0200;
    // First PCR (1s on 90kHz = base 90000, as_27mhz = 27000000).
    let af1 = make_pcr_adaptation(90000, 0, false, true);
    let pkt1 = make_ts_packet(pcr_pid, 0, false, true, &af1, &[]);
    // Second PCR (4s on 90kHz = base 360000, as_27mhz = 108000000).
    // Delta on 27MHz = 81000000 = 3000ms >> 100ms, no discontinuity.
    let af2 = make_pcr_adaptation(360000, 0, false, true);
    let pkt2 = make_ts_packet(pcr_pid, 1, false, true, &af2, &[]);

    monitor.feed(&pkt1, ms(0));
    let events = monitor.feed(&pkt2, ms(50));
    assert!(has_indicator(events, Indicator::PcrDiscontinuityError));
}

#[test]
fn pcr_discontinuity_error_suppressed_with_indicator() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pcr_pid: u16 = 0x0200;
    // Same large PCR delta but WITH discontinuity_indicator set — legal.
    let af1 = make_pcr_adaptation(90000, 0, false, true);
    let pkt1 = make_ts_packet(pcr_pid, 0, false, true, &af1, &[]);
    let af2 = make_pcr_adaptation(360000, 0, true, true); // discontinuity
    let pkt2 = make_ts_packet(pcr_pid, 1, false, true, &af2, &[]);

    monitor.feed(&pkt1, ms(0));
    let events = monitor.feed(&pkt2, ms(50));
    assert!(!has_indicator(events, Indicator::PcrDiscontinuityError));
}

#[test]
fn pcr_discontinuity_error_absent_within_range() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    let pcr_pid: u16 = 0x0200;
    // PCR delta of 50ms: base diff = 4500 (90kHz), as_27mhz diff = 1350000.
    let af1 = make_pcr_adaptation(90000, 0, false, true);
    let pkt1 = make_ts_packet(pcr_pid, 0, false, true, &af1, &[]);
    let af2 = make_pcr_adaptation(90000 + 4500, 0, false, true);
    let pkt2 = make_ts_packet(pcr_pid, 1, false, true, &af2, &[]);

    monitor.feed(&pkt1, ms(0));
    let events = monitor.feed(&pkt2, ms(50));
    assert!(!has_indicator(events, Indicator::PcrDiscontinuityError));
}

// ── 2.5 PTS_error ────────────────────────────────────────────────────────────

#[test]
fn pts_error_trips_on_large_gap() {
    let mut monitor = ConformanceMonitor::new();

    let es_pid: u16 = 0x0200;
    setup_monitor_with_es(&mut monitor, es_pid);

    // First PES with PTS at t=20ms.
    let pes1 = make_pes_header_with_pts(0xE0, 10);
    let pkt1 = make_ts_packet(es_pid, 0, true, false, &[], &pes1);
    let events1 = monitor.feed(&pkt1, ms(20));
    // First PTS should not produce an error (arm only).
    assert!(!has_indicator(events1, Indicator::PtsError));

    // Second PES with PTS at t=800ms — gap of 780ms exceeds 700ms limit.
    let pes2 = make_pes_header_with_pts(0xE0, 10);
    let pkt2 = make_ts_packet(es_pid, 1, true, false, &[], &pes2);
    let events2 = monitor.feed(&pkt2, ms(800));
    assert!(has_indicator(events2, Indicator::PtsError));
}

#[test]
fn pts_error_absent_within_limit() {
    let mut monitor = ConformanceMonitor::new();

    let es_pid: u16 = 0x0200;
    setup_monitor_with_es(&mut monitor, es_pid);

    // First PES with PTS.
    let pes1 = make_pes_header_with_pts(0xE0, 10);
    let pkt1 = make_ts_packet(es_pid, 0, true, false, &[], &pes1);
    monitor.feed(&pkt1, ms(20));

    // Second PES with PTS at t=500ms — gap of 480ms within 700ms limit.
    let pes2 = make_pes_header_with_pts(0xE0, 10);
    let pkt2 = make_ts_packet(es_pid, 1, true, false, &[], &pes2);
    let events = monitor.feed(&pkt2, ms(500));
    assert!(!has_indicator(events, Indicator::PtsError));
}

#[test]
fn pts_error_not_armed_without_first_pts() {
    let mut monitor = ConformanceMonitor::new();

    let es_pid: u16 = 0x0200;
    setup_monitor_with_es(&mut monitor, es_pid);

    // Feed a PES packet WITHOUT PTS — should not arm the check.
    let pes_no_pts = make_pes_header_without_pts(0xE0, 10);
    let pkt1 = make_ts_packet(es_pid, 0, true, false, &[], &pes_no_pts);
    monitor.feed(&pkt1, ms(20));

    // Feed another PES WITH PTS at t=800ms — first PTS, so just arms.
    let pes2 = make_pes_header_with_pts(0xE0, 10);
    let pkt2 = make_ts_packet(es_pid, 1, true, false, &[], &pes2);
    let events = monitor.feed(&pkt2, ms(800));
    assert!(!has_indicator(events, Indicator::PtsError));
}

// ── 2.6 CAT_error ────────────────────────────────────────────────────────────

#[test]
fn cat_error_wrong_table_id_on_pid_cat() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Build a section with table_id 0x02 (PMT) on PID 0x0001 (CAT PID).
    // Take a PAT section and change the PID to CAT PID.
    let mut section = build_pat_section(&[(1, 0x100)]);
    section[0] = 0x02; // table_id = PMT (not CAT)

    let packets = packetize_section(PID_CAT, &section);
    let events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(has_indicator(&events, Indicator::CatError));
}

#[test]
fn cat_error_scrambled_without_cat() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed a scrambled packet when no CAT has been seen.
    let pkt = make_ts_packet_with_scrambling(0x0100, 0, 0x03);
    let events = monitor.feed(&pkt, ms(10));
    assert!(has_indicator(events, Indicator::CatError));
}

/// Build a CRC-valid minimal CAT section (table_id 0x01, empty descriptor
/// loop). Long-form: 3-byte header + table_id_extension(2) + version/cni(1) +
/// section_number(1) + last_section_number(1) + CRC-32/MPEG-2(4) = 12 bytes.
fn build_valid_cat_section() -> Vec<u8> {
    let mut cat = vec![
        0x01, // table_id = CAT
        0xB0, 0x09, // ssi=1, '0', reserved=11, section_length=9
        0xFF, 0xFF, // table_id_extension (reserved for CAT)
        0xC1, // reserved=11, version=0, current_next_indicator=1
        0x00, // section_number
        0x00, // last_section_number
    ];
    let crc = dvb_common::crc32_mpeg2::compute(&cat);
    cat.extend_from_slice(&crc.to_be_bytes());
    cat
}

#[test]
fn cat_error_absent_when_cat_seen_then_scrambled() {
    let mut monitor = ConformanceMonitor::new();
    acquire_sync(&mut monitor);

    // Feed a CRC-VALID CAT section on PID 0x0001 — this must mark the CAT as
    // seen (and must NOT itself raise a CRC error).
    let packets = packetize_section(PID_CAT, &build_valid_cat_section());
    let cat_events = feed_all(&mut monitor, &packets, ms(5), ms(1));
    assert!(
        !has_indicator(&cat_events, Indicator::CrcError),
        "a CRC-valid CAT must not raise CRC_error"
    );
    assert!(
        !has_indicator(&cat_events, Indicator::CatError),
        "a valid CAT on PID 0x0001 must not raise CAT_error"
    );

    // Now a scrambled packet arrives — because a CAT has been seen, the
    // "scrambled packet but no CAT" condition (2.6) must NOT fire.
    let pkt = make_ts_packet_with_scrambling(0x0100, 0, 0x03);
    let events = monitor.feed(&pkt, ms(10));
    assert!(
        !has_indicator(events, Indicator::CatError),
        "CAT_error must not fire for a scrambled packet once a CAT has been seen"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// ── Priority 3 tests — SI_repetition_error (3.2) ────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// Build an SDT_actual section's wire bytes.
fn build_sdt_actual_section() -> Vec<u8> {
    let sdt = SdtSection {
        kind: SdtKind::Actual,
        transport_stream_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        original_network_id: 1,
        services: vec![],
    };
    let mut buf = vec![0u8; sdt.serialized_len()];
    sdt.serialize_into(&mut buf).unwrap();
    buf
}

/// Build a NIT_actual section's wire bytes.
fn build_nit_actual_section() -> Vec<u8> {
    let nit = NitSection {
        kind: NitKind::Actual,
        network_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        network_descriptors: DescriptorLoop::new(&[]),
        transport_streams: vec![],
    };
    let mut buf = vec![0u8; nit.serialized_len()];
    nit.serialize_into(&mut buf).unwrap();
    buf
}

/// Build an EIT P/F actual section's wire bytes.
fn build_eit_pf_actual_section() -> Vec<u8> {
    let eit = EitSection {
        kind: EitKind::PresentFollowingActual,
        table_id: dvb_si::tables::eit::TABLE_ID_PF_ACTUAL,
        service_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        transport_stream_id: 1,
        original_network_id: 1,
        segment_last_section_number: 0,
        last_table_id: dvb_si::tables::eit::TABLE_ID_PF_ACTUAL,
        events: vec![],
    };
    let mut buf = vec![0u8; eit.serialized_len()];
    eit.serialize_into(&mut buf).unwrap();
    buf
}

/// Build a TDT section's wire bytes.
fn build_tdt_section() -> Vec<u8> {
    let tdt = TdtSection::new([0x00, 0x00, 0x00, 0x00, 0x00]);
    let mut buf = vec![0u8; tdt.serialized_len()];
    tdt.serialize_into(&mut buf).unwrap();
    buf
}

// ── 3.2 SDT_actual repetition ────────────────────────────────────────────────

#[test]
fn si_repetition_sdt_trips_on_timeout() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed an SDT_actual section.
    let section = build_sdt_actual_section();
    let packets = packetize_section(PID_SDT_BAT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed a packet on another PID past the 2 s SDT interval.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, secs(3));
    assert!(has_indicator(events, Indicator::SiRepetitionError));
    let si_events: Vec<_> = events
        .iter()
        .filter(|e| e.indicator == Indicator::SiRepetitionError)
        .collect();
    assert_eq!(si_events.len(), 1);
    assert_eq!(si_events[0].pid, Some(PID_SDT_BAT));
    assert!(si_events[0].detail.contains("SDT_actual"));
}

#[test]
fn si_repetition_sdt_compliant_within_interval() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed an SDT_actual section.
    let section = build_sdt_actual_section();
    let packets = packetize_section(PID_SDT_BAT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed another SDT_actual within 2 s.
    let packets2 = packetize_section(PID_SDT_BAT, &section);
    let events = feed_all(&mut monitor, &packets2, ms(1000), ms(1));
    assert!(!has_indicator(&events, Indicator::SiRepetitionError));
}

// ── 3.2 TDT repetition ──────────────────────────────────────────────────────

#[test]
fn si_repetition_tdt_trips_on_timeout() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed a TDT section.
    let section = build_tdt_section();
    let packets = packetize_section(PID_TDT_TOT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed a packet past the 30 s TDT interval.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, secs(35));
    assert!(has_indicator(events, Indicator::SiRepetitionError));
    let si_events: Vec<_> = events
        .iter()
        .filter(|e| e.indicator == Indicator::SiRepetitionError)
        .collect();
    assert_eq!(si_events.len(), 1);
    assert_eq!(si_events[0].pid, Some(PID_TDT_TOT));
    assert!(si_events[0].detail.contains("TDT"));
}

#[test]
fn si_repetition_tdt_compliant_within_interval() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed a TDT section.
    let section = build_tdt_section();
    let packets = packetize_section(PID_TDT_TOT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed another TDT within 30 s.
    let packets2 = packetize_section(PID_TDT_TOT, &section);
    let events = feed_all(&mut monitor, &packets2, secs(20), ms(1));
    assert!(!has_indicator(&events, Indicator::SiRepetitionError));
}

// ── 3.2 NIT_actual repetition ────────────────────────────────────────────────

#[test]
fn si_repetition_nit_trips_on_timeout() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed a NIT_actual section.
    let section = build_nit_actual_section();
    let packets = packetize_section(PID_NIT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed a packet past the 10 s NIT interval.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, secs(12));
    assert!(has_indicator(events, Indicator::SiRepetitionError));
    let si_events: Vec<_> = events
        .iter()
        .filter(|e| e.indicator == Indicator::SiRepetitionError)
        .collect();
    assert_eq!(si_events.len(), 1);
    assert_eq!(si_events[0].pid, Some(PID_NIT));
    assert!(si_events[0].detail.contains("NIT_actual"));
}

#[test]
fn si_repetition_nit_compliant_within_interval() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed a NIT_actual section.
    let section = build_nit_actual_section();
    let packets = packetize_section(PID_NIT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed another NIT within 10 s.
    let packets2 = packetize_section(PID_NIT, &section);
    let events = feed_all(&mut monitor, &packets2, secs(5), ms(1));
    assert!(!has_indicator(&events, Indicator::SiRepetitionError));
}

// ── 3.2 EIT P/F actual repetition ───────────────────────────────────────────

#[test]
fn si_repetition_eit_pf_trips_on_timeout() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed an EIT P/F actual section.
    let section = build_eit_pf_actual_section();
    let packets = packetize_section(PID_EIT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed a packet past the 2 s EIT interval.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let events = monitor.feed(&pkt, secs(3));
    assert!(has_indicator(events, Indicator::SiRepetitionError));
    let si_events: Vec<_> = events
        .iter()
        .filter(|e| e.indicator == Indicator::SiRepetitionError)
        .collect();
    assert_eq!(si_events.len(), 1);
    assert_eq!(si_events[0].pid, Some(PID_EIT));
    assert!(si_events[0].detail.contains("EIT_P/F_actual"));
}

#[test]
fn si_repetition_eit_pf_compliant_within_interval() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed an EIT P/F actual section.
    let section = build_eit_pf_actual_section();
    let packets = packetize_section(PID_EIT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed another EIT within 2 s.
    let packets2 = packetize_section(PID_EIT, &section);
    let events = feed_all(&mut monitor, &packets2, secs(1), ms(1));
    assert!(!has_indicator(&events, Indicator::SiRepetitionError));
}

// ── 3.2 Lazy arming — absent table not flagged ───────────────────────────────

#[test]
fn si_repetition_not_armed_before_first_section() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed packets on unrelated PIDs well past all SI intervals.
    // No SI table has been seen, so no timer is armed.
    for i in 0..5u8 {
        let pkt = make_ts_packet(0x200, i, false, false, &[], &[]);
        let events = monitor.feed(&pkt, secs(60 + i as u64));
        assert!(
            !has_indicator(events, Indicator::SiRepetitionError),
            "SiRepetitionError should not fire before any SI section is seen"
        );
    }
}

// ── 3.2 Emit-once semantics ──────────────────────────────────────────────────

#[test]
fn si_repetition_emits_once_not_per_packet() {
    let mut monitor = ConformanceMonitor::new();

    acquire_sync(&mut monitor);

    // Feed an SDT_actual section.
    let section = build_sdt_actual_section();
    let packets = packetize_section(PID_SDT_BAT, &section);
    feed_all(&mut monitor, &packets, ms(5), ms(1));

    // Feed packets past the 2 s interval — only the FIRST should emit.
    let pkt = make_ts_packet(0x200, 0, false, false, &[], &[]);
    let e1 = monitor.feed(&pkt, secs(3)).to_vec();
    let e2 = monitor.feed(&pkt, secs(4)).to_vec();
    let e3 = monitor.feed(&pkt, secs(5)).to_vec();

    assert_eq!(
        e1.iter()
            .filter(|e| e.indicator == Indicator::SiRepetitionError)
            .count(),
        1
    );
    assert!(!has_indicator(&e2, Indicator::SiRepetitionError));
    assert!(!has_indicator(&e3, Indicator::SiRepetitionError));
}
