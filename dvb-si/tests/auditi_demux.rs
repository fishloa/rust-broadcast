use dvb_si::demux::SiDemux;
use test_fixtures::psi::{crc32_mpeg2, ts_packet};

fn section(
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
fn multi_section_pat_keeps_pmt_pids_of_every_section() {
    let mut demux = SiDemux::builder().build();
    let s0 = section(0, 1, 0, true, 0, 1, &pat_payload(&[(1, 0x100)]));
    let s1 = section(0, 1, 0, true, 1, 1, &pat_payload(&[(2, 0x200)]));
    assert_eq!(demux.feed(&ts_packet(0, &s0)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(0, &s1)).count(), 1);
    let pmt1 = section(2, 1, 0, true, 0, 0, &[0xE1, 0x01, 0xF0, 0x00]);
    let pmt2 = section(2, 2, 0, true, 0, 0, &[0xE2, 0x01, 0xF0, 0x00]);
    let n1 = demux.feed(&ts_packet(0x100, &pmt1)).count();
    let n2 = demux.feed(&ts_packet(0x200, &pmt2)).count();
    assert_eq!(
        (n1, n2),
        (1, 1),
        "PMT PIDs from both PAT sections must be watched"
    );
}

#[test]
fn alternating_current_and_next_versions_do_not_re_emit_every_cycle() {
    let mut demux = SiDemux::builder().build();
    let cur = section(0x40, 1, 3, true, 0, 0, &[0xAA]);
    let next = section(0x40, 1, 4, false, 0, 0, &[0xBB]);
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
fn next_pat_with_current_next_zero_does_not_change_followed_pmt_pids() {
    let mut demux = SiDemux::builder().build();
    let cur = section(0, 1, 0, true, 0, 0, &pat_payload(&[(1, 0x100)]));
    let next = section(0, 1, 1, false, 0, 0, &pat_payload(&[(1, 0x300)]));
    demux.feed(&ts_packet(0, &cur)).count();
    demux.feed(&ts_packet(0, &next)).count();
    let pmt = section(2, 1, 0, true, 0, 0, &[0xE1, 0x01, 0xF0, 0x00]);
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
fn retransmitted_duplicate_continuation_packet_does_not_drop_the_section() {
    let sec = section(0x4E, 1, 1, true, 0, 0, &[0x5A; 400]);
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
