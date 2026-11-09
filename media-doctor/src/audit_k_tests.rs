use crate::diagnostics::codec_common::tests::{TEST_PMT_PID, build_pat_pmt_ts_versioned};
use crate::{Diagnostic, PatPmtVersionCheck, Report};
use dvb_si::tables::pmt::StreamType;

const PACKET: usize = 188;

fn pmt_packet_range(ts: &[u8]) -> std::ops::Range<usize> {
    for i in 0..ts.len() / PACKET {
        let p = &ts[i * PACKET..(i + 1) * PACKET];
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid == TEST_PMT_PID.value() {
            return i * PACKET..(i + 1) * PACKET;
        }
    }
    panic!("no PMT packet");
}

fn pmt_version_findings(ts: &[u8]) -> usize {
    let mut report = Report::new();
    PatPmtVersionCheck.run(ts, &mut report);
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pmt-version")
        .count()
}

#[test]
fn pmt_with_bad_crc_must_not_report_a_version_change() {
    let streams = [(0x0100u16, StreamType::H264)];
    let v0 = build_pat_pmt_ts_versioned(&streams, 0, true);
    let v1 = build_pat_pmt_ts_versioned(&streams, 1, true);

    let mut good = v0.clone();
    good.extend_from_slice(&v1);
    assert_eq!(pmt_version_findings(&good), 1, "control: a CRC-valid version bump is reported");

    let mut bad_v1 = v1.clone();
    let range = pmt_packet_range(&bad_v1);
    let p = &mut bad_v1[range];
    let section_length = (usize::from(p[6] & 0x0F) << 8) | usize::from(p[7]);
    let crc_at = 5 + 3 + section_length - 4;
    for b in &mut p[crc_at..crc_at + 4] {
        *b ^= 0xFF;
    }
    let mut corrupted = v0;
    corrupted.extend_from_slice(&bad_v1);
    assert_eq!(
        pmt_version_findings(&corrupted),
        0,
        "a PMT whose CRC-32 is wrong must be dropped (module docs: 'A corrupted section is dropped'), as the PAT path does"
    );
}

#[test]
fn codec_psi_discovery_must_skip_a_pmt_with_a_bad_crc() {
    use crate::diagnostics::codec_common::collect_pmt_streams;
    let mut ts = build_pat_pmt_ts_versioned(&[(0x0100u16, StreamType::H264)], 0, true);
    let control = collect_pmt_streams(&ts);
    assert_eq!(control.len(), 1, "control: the intact PMT declares one stream");
    let range = pmt_packet_range(&ts);
    let p = &mut ts[range];
    let section_length = (usize::from(p[6] & 0x0F) << 8) | usize::from(p[7]);
    let crc_at = 5 + 3 + section_length - 4;
    for b in &mut p[crc_at..crc_at + 4] {
        *b ^= 0xFF;
    }
    assert!(
        collect_pmt_streams(&ts).is_empty(),
        "a PMT failing its CRC-32 must not declare streams (collect_pmt_streams docs: malformed sections are skipped)"
    );
}

#[test]
fn codec_psi_discovery_must_ignore_a_next_generation_pmt() {
    use crate::diagnostics::codec_common::collect_pmt_streams;
    let ts = build_pat_pmt_ts_versioned(&[(0x0100u16, StreamType::H264)], 0, false);
    assert!(
        collect_pmt_streams(&ts).is_empty(),
        "current_next_indicator=0 is the NEXT table (13818-1 2.4.4.11), not what the stream carries now"
    );
}
