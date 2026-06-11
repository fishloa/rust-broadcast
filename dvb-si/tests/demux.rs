//! Integration tests for [`dvb_si::demux::SiDemux`] — the plan's 5 scenarios,
//! exercised through the public crate surface (no `super::` internals).
#![cfg(feature = "ts")]

use dvb_si::demux::SiDemux;
use dvb_si::pid::Pid;
use dvb_si::tables::AnyTableSection;
use dvb_si::ts::{TsHeader, TS_PACKET_SIZE};

const LONG_FORM_EXTRA: usize = 5;
const CRC_LEN: usize = 4;

/// One PUSI TS packet on `pid`, pointer_field 0, 0xFF stuffing tail.
fn ts_packet(pid: u16, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    let header = TsHeader {
        tei: false,
        pusi: true,
        pid,
        scrambling: 0,
        has_adaptation: false,
        has_payload: true,
        continuity_counter: 0,
    };
    header.serialize_into(&mut pkt).unwrap();
    pkt[4] = 0x00; // pointer_field
    let start = 5;
    assert!(start + section.len() <= TS_PACKET_SIZE, "section too big");
    pkt[start..start + section.len()].copy_from_slice(section);
    pkt
}

/// Long-form section with a correct trailing CRC-32.
fn long_section(
    table_id: u8,
    ext: u16,
    version: u8,
    section_number: u8,
    payload: &[u8],
) -> Vec<u8> {
    let section_length = (LONG_FORM_EXTRA + payload.len() + CRC_LEN) as u16;
    let mut v = vec![
        table_id,
        0x80 | 0x30 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        (ext >> 8) as u8,
        (ext & 0xFF) as u8,
        0xC0 | ((version & 0x1F) << 1) | 0x01,
        section_number,
        section_number,
    ];
    v.extend_from_slice(payload);
    let crc = dvb_common::crc32_mpeg2::compute(&v);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

fn pat_section(tsid: u16, version: u8, entries: &[(u16, u16)]) -> Vec<u8> {
    let mut body = Vec::new();
    for &(pn, pid) in entries {
        body.extend_from_slice(&pn.to_be_bytes());
        body.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
        body.push((pid & 0xFF) as u8);
    }
    long_section(0x00, tsid, version, 0, &body)
}

fn pmt_section(program_number: u16, version: u8, pcr_pid: u16) -> Vec<u8> {
    let body = [
        0xE0 | ((pcr_pid >> 8) as u8 & 0x1F),
        (pcr_pid & 0xFF) as u8,
        0xF0,
        0x00,
        0x02,
        0xE0 | (((pcr_pid + 1) >> 8) as u8 & 0x1F),
        ((pcr_pid + 1) & 0xFF) as u8,
        0xF0,
        0x00,
    ];
    long_section(0x02, program_number, version, 0, &body)
}

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
fn scenario4_gate_eviction_and_reemit() {
    let mut demux = SiDemux::builder().gate_capacity(2).build();
    let a = long_section(0x4E, 0x0001, 0, 0, &[0x01]);
    let b = long_section(0x4E, 0x0002, 0, 0, &[0x02]);
    let c = long_section(0x4E, 0x0003, 0, 0, &[0x03]);
    assert_eq!(demux.feed(&ts_packet(0x0012, &a)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(0x0012, &b)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(0x0012, &c)).count(), 1);
    assert_eq!(demux.stats().gate_evictions, 1);
    // a was evicted -> re-emits.
    assert_eq!(demux.feed(&ts_packet(0x0012, &a)).count(), 1);
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
    let crc = dvb_common::crc32_mpeg2::compute(&v);
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
