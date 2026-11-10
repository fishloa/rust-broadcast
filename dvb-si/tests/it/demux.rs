//! Integration tests for [`dvb_si::demux::SiDemux`] — the plan's 5 scenarios,
//! exercised through the public crate surface (no `super::` internals).
#![cfg(feature = "ts")]

use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::TS_PACKET_SIZE;
use test_fixtures::psi::{crc32_mpeg2, long_section, pat_section, pmt_section, ts_packet};

#[test]
fn scenario1_pat_version_gate() {
    let mut demux = SiDemux::builder().build();
    let v0 = ts_packet(0x0000, &pat_section(0x0001, 0, &[(1, 0x0100)]));
    let v1 = ts_packet(0x0000, &pat_section(0x0001, 1, &[(1, 0x0100)]));

    assert_eq!(demux.feed(&v0).count(), 1);
    assert_eq!(demux.feed(&v0).count(), 0);
    assert_eq!(demux.feed(&v1).count(), 1);

    // Exhaustive pin of all seven Stats fields.
    let stats = demux.stats();
    assert_eq!(stats.packets, 3);
    assert_eq!(stats.sections_completed, 3);
    assert_eq!(stats.emitted, 2);
    assert_eq!(stats.suppressed, 1);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(stats.malformed_packets, 0);
    assert_eq!(stats.gate_evictions, 0);
}

#[test]
fn scenario2_follow_pat_to_typed_pmt() {
    let mut demux = SiDemux::builder().build();
    demux
        .feed(&ts_packet(0x0000, &pat_section(0x0001, 0, &[(1, 0x0100)])))
        .for_each(drop);
    let evts: Vec<_> = demux
        .feed(&ts_packet(0x0100, &pmt_section(1, 0, 0x0100)))
        .collect();
    assert_eq!(evts.len(), 1);
    assert_eq!(evts[0].pid(), Pid::new(0x0100));
    assert!(matches!(
        evts[0].table_section().unwrap(),
        AnyTableSection::PmtSection(_)
    ));
}

#[test]
fn scenario3_corrupted_crc_dropped() {
    let mut demux = SiDemux::builder().build();
    let mut sdt = long_section(0x42, 0x0001, 0, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
    sdt[8] ^= 0xFF;
    assert_eq!(demux.feed(&ts_packet(0x0011, &sdt)).count(), 0);
    assert_eq!(demux.stats().crc_failures, 1);
}

#[test]
fn scenario5_garbage_packet_no_panic() {
    let mut demux = SiDemux::builder().build();
    assert_eq!(demux.feed(&[0u8; TS_PACKET_SIZE]).count(), 0);
    assert_eq!(demux.stats().malformed_packets, 1);
}

/// Build a TOT section (table_id 0x73) with a real CRC-32/MPEG-2,
/// no descriptors, and a known UTC time. SSI=0 per EN 300 468 §5.2.6.
fn tot_section() -> Vec<u8> {
    let utc_time: [u8; 5] = [0xE4, 0x09, 0x12, 0x34, 0x56];
    let section_length: u16 = 11;
    let mut v = Vec::new();
    v.push(0x73);
    v.push(0x70 | ((section_length >> 8) as u8 & 0x0F));
    v.push((section_length & 0xFF) as u8);
    v.extend_from_slice(&utc_time);
    v.push(0xF0);
    v.push(0x00);
    let crc = broadcast_common::crc32_mpeg2::compute(&v);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

#[test]
fn tot_crc_validated_emitted_and_corrupted_dropped() {
    let mut demux = SiDemux::builder().build();

    // Valid TOT with canonical CRC → emitted.
    let valid = tot_section();
    assert_eq!(
        demux.feed(&ts_packet(0x0014, &valid)).count(),
        1,
        "valid TOT must emit"
    );
    assert_eq!(demux.stats().crc_failures, 0);

    // Verify the emitted event is a TotSection with correct UTC time.
    let valid_pkt = ts_packet(0x0014, &valid);
    let events: Vec<_> = SiDemux::builder().build().feed(&valid_pkt).collect();
    assert_eq!(events.len(), 1);
    match events[0].table_section().unwrap() {
        AnyTableSection::TotSection(tot) => {
            assert_eq!(tot.utc_time_raw(), [0xE4, 0x09, 0x12, 0x34, 0x56]);
        }
        other => panic!("expected TotSection, got {other:?}"),
    }
    assert_eq!(events[0].pid(), Pid::new(0x0014));

    // Corrupt the CRC → dropped, crc_failures incremented.
    let mut bad = valid;
    let crc_pos = bad.len() - 4;
    bad[crc_pos + 3] ^= 0xFF;
    assert_eq!(
        demux.feed(&ts_packet(0x0014, &bad)).count(),
        0,
        "corrupted TOT must not emit"
    );
    assert_eq!(demux.stats().crc_failures, 1);
    // emitted still 1 (only the valid one counted).
    assert_eq!(demux.stats().emitted, 1);
    assert_eq!(demux.stats().sections_completed, 2);
}

fn flagged_section(
    table_id: u8,
    ext: u16,
    version: u8,
    current: bool,
    section_number: u8,
    last: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&ext.to_be_bytes());
    body.push(0xC0 | ((version & 0x1F) << 1) | u8::from(current));
    body.push(section_number);
    body.push(last);
    body.extend_from_slice(payload);
    let len = body.len() + 4;
    let mut v = vec![table_id, 0xB0 | (len >> 8) as u8, len as u8];
    v.extend_from_slice(&body);
    let crc = crc32_mpeg2(&v);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

fn pat_payload(entries: &[(u16, u16)]) -> Vec<u8> {
    let mut b = Vec::new();
    for &(pn, pid) in entries {
        b.extend_from_slice(&pn.to_be_bytes());
        b.push(0xE0 | (pid >> 8) as u8);
        b.push(pid as u8);
    }
    b
}

#[test]
fn multi_section_pat_watches_the_pmt_pids_of_every_section() {
    let mut demux = SiDemux::builder().build();
    let s0 = flagged_section(0, 1, 0, true, 0, 1, &pat_payload(&[(1, 0x100)]));
    let s1 = flagged_section(0, 1, 0, true, 1, 1, &pat_payload(&[(2, 0x200)]));
    assert_eq!(demux.feed(&ts_packet(0, &s0)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(0, &s1)).count(), 1);
    let pmt1 = flagged_section(2, 1, 0, true, 0, 0, &[0xE1, 0x01, 0xF0, 0x00]);
    let pmt2 = flagged_section(2, 2, 0, true, 0, 0, &[0xE2, 0x01, 0xF0, 0x00]);
    let n1 = demux.feed(&ts_packet(0x100, &pmt1)).count();
    let n2 = demux.feed(&ts_packet(0x200, &pmt2)).count();
    assert_eq!(
        (n1, n2),
        (1, 1),
        "PMT PIDs from both PAT sections must be watched"
    );
}

#[test]
fn alternating_current_and_next_versions_are_each_emitted_once() {
    let mut demux = SiDemux::builder().build();
    let cur = flagged_section(0x40, 1, 3, true, 0, 0, &[0xAA]);
    let next = flagged_section(0x40, 1, 4, false, 0, 0, &[0xBB]);
    let pid = 0x10;
    let mut emitted = 0;
    for _ in 0..5 {
        emitted += demux.feed(&ts_packet(pid, &cur)).count();
        emitted += demux.feed(&ts_packet(pid, &next)).count();
    }
    assert_eq!(
        emitted, 2,
        "current v3 and next v4 are each new once; repeats are unchanged"
    );
}

#[test]
fn a_next_pat_does_not_change_the_followed_pmt_pids() {
    let mut demux = SiDemux::builder().build();
    let cur = flagged_section(0, 1, 0, true, 0, 0, &pat_payload(&[(1, 0x100)]));
    let next = flagged_section(0, 1, 1, false, 0, 0, &pat_payload(&[(1, 0x300)]));
    demux.feed(&ts_packet(0, &cur)).count();
    demux.feed(&ts_packet(0, &next)).count();
    let pmt = flagged_section(2, 1, 0, true, 0, 0, &[0xE1, 0x01, 0xF0, 0x00]);
    assert_eq!(
        demux.feed(&ts_packet(0x100, &pmt)).count(),
        1,
        "a not-yet-applicable PAT must not unwatch the current PMT PID"
    );
}

fn raw_packet(pid: u16, pusi: bool, cc: u8, payload: &[u8]) -> [u8; 188] {
    let mut p = [0xFFu8; 188];
    p[0] = 0x47;
    p[1] = (if pusi { 0x40 } else { 0 }) | (pid >> 8) as u8;
    p[2] = pid as u8;
    p[3] = 0x10 | (cc & 0x0F);
    let mut at = 4;
    if pusi {
        p[4] = 0;
        at = 5;
    }
    p[at..at + payload.len()].copy_from_slice(payload);
    p
}

#[test]
fn a_retransmitted_continuation_packet_does_not_drop_the_section() {
    let sec = flagged_section(0x4E, 1, 1, true, 0, 0, &[0x5A; 400]);
    let first = &sec[..183];
    let second = &sec[183..183 + 184];
    let third = &sec[183 + 184..];
    let mut demux = SiDemux::builder().build();
    let mut n = 0;
    n += demux.feed(&raw_packet(0x12, true, 0, first)).count();
    n += demux.feed(&raw_packet(0x12, false, 1, second)).count();
    n += demux.feed(&raw_packet(0x12, false, 1, second)).count();
    n += demux.feed(&raw_packet(0x12, false, 2, third)).count();
    assert_eq!(
        n, 1,
        "H.222.0 2.4.3.3 permits one duplicate packet (same continuity_counter); it must be discarded, not appended"
    );
}

#[test]
fn a_table_id_zero_section_on_another_pid_does_not_change_the_followed_pmt_pids() {
    let mut demux = SiDemux::builder().build();
    let stray = flagged_section(0, 1, 0, true, 0, 0, &pat_payload(&[(1, 0x300)]));
    demux.feed(&ts_packet(0x12, &stray)).count();
    let pmt = flagged_section(2, 1, 0, true, 0, 0, &[0xE3, 0x01, 0xF0, 0x00]);
    assert_eq!(
        demux.feed(&ts_packet(0x300, &pmt)).count(),
        0,
        "only the PAT on PID 0 may add PMT PIDs to the watch set"
    );
}

#[test]
fn a_next_pat_does_not_unwatch_a_multi_section_current_pat() {
    use broadcast_common::Serialize;
    use dvb_si::tables::pat::{PatEntry, PatSection};
    fn pat_bytes(v: u8, cur: bool, sn: u8, last: u8, pid: u16) -> Vec<u8> {
        let pat = PatSection::new(0x1111, v, cur, sn, last, vec![PatEntry::new(u16::from(sn) + 1, Pid::new(pid))]);
        let mut b = vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut b).unwrap();
        b
    }
    fn ts_packet(pid: u16, section: &[u8]) -> Vec<u8> {
        let mut p = vec![0xFFu8; 188];
        p[0] = 0x47; p[1] = 0x40 | ((pid >> 8) as u8 & 0x1F); p[2] = (pid & 0xFF) as u8; p[3] = 0x10; p[4] = 0x00;
        p[5..5 + section.len()].copy_from_slice(section);
        p
    }
    let mut d = SiDemux::builder().build();
    d.feed(&ts_packet(0, &pat_bytes(0, true, 0, 1, 0x0100))).for_each(drop);
    d.feed(&ts_packet(0, &pat_bytes(0, true, 1, 1, 0x0101))).for_each(drop);
    let sec = pmt_section(1, 0, 0x0111);
    assert_eq!(d.feed(&ts_packet(0x0100, &sec)).count(), 1);
    d.feed(&ts_packet(0, &pat_bytes(1, false, 0, 0, 0x0300))).for_each(drop);
    let sec2 = pmt_section(2, 0, 0x0111);
    for &p in &[0x0100u16, 0x0101] {
        let evts: Vec<_> = d.feed(&ts_packet(p, &sec2)).collect();
        assert_eq!(evts.len(), 1, "PMT PID 0x{:04X} still followed after a next PAT", p);
    }
}
