//! `InnerTsRecovery` mode gating and statistics — EN 302 755 §5.1.7 / §5.1.8.

#![cfg(feature = "ts")]

use broadcast_common::crc32_mpeg2;
use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs};
use dvb_t2mi::inner_ts::InnerTsRecovery;
use mpeg_ts::pid::Pid;

const TS_LEN: usize = 188;
const T2MI_PID: u16 = 0x1000;

fn inner_packet() -> [u8; TS_LEN] {
    let mut p = [0xAAu8; TS_LEN];
    p[0] = 0x47;
    p[1] = 0x41;
    p[2] = 0x00;
    p[3] = 0x10;
    p[4] = 0x5A;
    p
}

fn hem_bbframe(inner: &[u8; TS_LEN], npd: bool) -> Vec<u8> {
    let hdr = Bbheader {
        matype: Matype {
            ts_gs: TsGs::Ts,
            sis: true,
            ccm: true,
            issyi: false,
            npd,
            ext: 0,
            isi: 0,
        },
        upl: 0,
        sync: 0,
        dfl: (187 * 8) as u16,
        syncd: 0,
        mode: Mode::HighEfficiency,
        issy_in_header: None,
    };
    let mut frame = hdr.serialize().unwrap().to_vec();
    frame.extend_from_slice(&inner[1..]);
    frame
}

fn t2mi_bbframe(bbframe: &[u8], plp_id: u8) -> Vec<u8> {
    let mut payload = vec![0x00, plp_id, 0x80];
    payload.extend_from_slice(bbframe);
    let mut pkt = vec![0x00u8, 0x01, 0x00, 0x00];
    pkt.extend_from_slice(&((payload.len() * 8) as u16).to_be_bytes());
    pkt.extend_from_slice(&payload);
    let crc = crc32_mpeg2::compute(&pkt);
    pkt.extend_from_slice(&crc.to_be_bytes());
    pkt
}

fn outer_ts(data: &[u8]) -> Vec<[u8; TS_LEN]> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut first = true;
    while offset < data.len() {
        let mut pkt = [0xFFu8; TS_LEN];
        pkt[0] = 0x47;
        pkt[1] = if first { 0x40 } else { 0x00 } | ((T2MI_PID >> 8) as u8 & 0x1F);
        pkt[2] = (T2MI_PID & 0xFF) as u8;
        pkt[3] = 0x10;
        let header_len = if first {
            pkt[4] = 0x00;
            5
        } else {
            4
        };
        first = false;
        let capacity = TS_LEN - header_len;
        let n = (data.len() - offset).min(capacity);
        pkt[header_len..header_len + n].copy_from_slice(&data[offset..offset + n]);
        out.push(pkt);
        offset += n;
    }
    out
}

#[test]
fn hem_with_npd_is_skipped_not_mis_decoded() {
    let inner = inner_packet();
    let t2mi = t2mi_bbframe(&hem_bbframe(&inner, true), 5);
    let outer = outer_ts(&t2mi);

    let mut recovery = InnerTsRecovery::new(Pid::from_wire_masked(T2MI_PID));
    let mut recovered = Vec::new();
    for pkt in &outer {
        recovered.extend_from_slice(recovery.feed(pkt));
    }
    assert!(
        recovered.is_empty(),
        "HEM frames with MATYPE.NPD set are not recoverable"
    );
    assert_eq!(recovery.plps_seen().collect::<Vec<_>>(), vec![5]);
}

#[test]
fn hem_without_npd_still_recovers_the_inner_packet() {
    let inner = inner_packet();
    let t2mi = t2mi_bbframe(&hem_bbframe(&inner, false), 5);
    let outer = outer_ts(&t2mi);

    let mut recovery = InnerTsRecovery::new(Pid::from_wire_masked(T2MI_PID));
    let mut recovered = Vec::new();
    for pkt in &outer {
        recovered.extend_from_slice(recovery.feed(pkt));
    }
    assert_eq!(recovered.len(), 1);
    assert_eq!(&recovered[0][..], &inner[..]);
}

#[test]
fn stats_pass_through_the_pump() {
    let inner = inner_packet();
    let t2mi = t2mi_bbframe(&hem_bbframe(&inner, false), 3);
    let outer = outer_ts(&t2mi);

    let mut recovery = InnerTsRecovery::new(Pid::from_wire_masked(T2MI_PID));
    let mut seen = 0u64;
    for pkt in &outer {
        recovery.feed(pkt);
        seen += 1;
    }
    let stats = recovery.stats();
    assert_eq!(stats.ts_packets, seen);
    assert_eq!(stats.t2mi_packets, 1);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(stats.malformed_packets, 0);
}
