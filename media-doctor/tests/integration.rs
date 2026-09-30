//! Integration tests for `media-doctor`.

use std::fs;

use media_doctor::Diagnostic;
use media_doctor::{
    CcAnomalyCheck, Finding, Location, PatPmtVersionCheck, PcrCheck, Report, Severity,
    SyncByteCheck,
};

/// Path helper: fixture TS file.
fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/ts/{}", env!("CARGO_MANIFEST_DIR"), name);
    fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
}

/// A clean buffer of 2 good TS packets should produce zero findings.
#[test]
fn sync_byte_clean_packets() {
    let mut ts = Vec::new();
    for _ in 0..2 {
        let mut pkt = vec![0x47u8; 188];
        // minimal valid header: sync=0x47, pid=0x1FFF, TSC=00, AFC=01, CC=0
        pkt[3] = 0x10; // AFC=01 (no adaptation), CC=0
        ts.extend_from_slice(&pkt);
    }
    let mut report = Report::new();
    SyncByteCheck.run(&ts, &mut report);
    assert!(
        report.is_empty(),
        "expected no findings, got {}",
        report.len()
    );
}

/// A buffer with one bad sync byte should produce exactly one Error finding.
#[test]
fn sync_byte_one_bad_packet() {
    let mut ts = Vec::new();
    // First packet: good
    let mut pkt1 = vec![0x47u8; 188];
    pkt1[3] = 0x10;
    ts.extend_from_slice(&pkt1);
    // Second packet: bad sync byte (0x00 instead of 0x47)
    let mut pkt2 = vec![0x00u8; 188];
    pkt2[1] = 0x12;
    pkt2[2] = 0x34;
    pkt2[3] = 0x10;
    ts.extend_from_slice(&pkt2);
    // Third packet: good
    let mut pkt3 = vec![0x47u8; 188];
    pkt3[3] = 0x10;
    ts.extend_from_slice(&pkt3);

    let mut report = Report::new();
    SyncByteCheck.run(&ts, &mut report);
    assert_eq!(report.len(), 1);
    let f = &report.findings()[0];
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.location.packet, 1);
    assert_eq!(f.rule_id, "sync-byte");
}

/// Report text rendering produces expected output for empty and populated reports.
#[test]
fn report_text_format() {
    // Empty
    let r = Report::new();
    let text = r.to_string();
    assert!(text.contains("No issues found"));

    // Populated
    let mut r = Report::new();
    r.push(Finding::new(
        Severity::Error,
        Location::new(0, 0x0100),
        "sync-byte",
        "bad sync",
    ));
    r.push(Finding::new(
        Severity::Warning,
        Location::new(5, 0x0010),
        "test-rule",
        "warning msg",
    ));
    let text = r.to_string();
    assert!(text.contains("1 error(s), 1 warning(s), 0 info(s)"));
    assert!(text.contains("bad sync"));
    assert!(text.contains("warning msg"));
}

/// JSON round-trip: serialize a Report to JSON and deserialize back.
#[cfg(feature = "serde")]
#[test]
fn report_json_roundtrip() {
    let mut report = Report::new();
    report.push(Finding::new(
        Severity::Error,
        Location::new(42, 0x0100),
        "sync-byte",
        "bad sync byte",
    ));
    report.push(Finding::new(
        Severity::Info,
        Location::new(99, 0),
        "test",
        "info message",
    ));

    let json = serde_json::to_string_pretty(&report).expect("serialize report");
    let deser: Report = serde_json::from_str(&json).expect("deserialize report");
    assert_eq!(report, deser);
}

/// Severity::name() and Display work correctly.
#[test]
fn severity_name_display() {
    assert_eq!(Severity::Error.name(), "error");
    assert_eq!(Severity::Warning.name(), "warning");
    assert_eq!(Severity::Info.name(), "info");
    assert_eq!(Severity::Error.to_string(), "error");
    assert_eq!(Severity::Warning.to_string(), "warning");
    assert_eq!(Severity::Info.to_string(), "info");
}

// ── CcAnomalyCheck tests ─────────────────────────────────────────────────────

/// A clean stream with correct +1 CCs should produce zero CC findings.
#[test]
fn cc_anomaly_clean_stream() {
    let mut ts = Vec::new();
    let pid = 0x0100u16;
    for cc in 0u8..16 {
        let mut pkt = vec![0x47u8; 188];
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x10 | cc; // AFC=01 (payload only), CC=cc
        ts.extend_from_slice(&pkt);
    }

    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();
    assert!(
        cc_findings.is_empty(),
        "expected no CC anomalies on clean stream, got {}: {:?}",
        cc_findings.len(),
        cc_findings
    );
}

/// A stream with a wrong CC should produce an Error finding.
#[test]
fn cc_anomaly_wrong_cc() {
    let mut ts = Vec::new();
    let pid = 0x0100u16;
    let mut pkt = vec![0x47u8; 188];
    pkt[1] = ((pid >> 8) as u8) & 0x1F;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // CC=0
    ts.extend_from_slice(&pkt);

    let mut pkt2 = vec![0x47u8; 188];
    pkt2[1] = ((pid >> 8) as u8) & 0x1F;
    pkt2[2] = (pid & 0xFF) as u8;
    pkt2[3] = 0x10 | 5; // CC=5 (expected 1) — anomaly
    ts.extend_from_slice(&pkt2);

    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();
    assert_eq!(cc_findings.len(), 1);
    assert_eq!(cc_findings[0].severity, Severity::Error);
    assert_eq!(cc_findings[0].location.pid, u32::from(pid));
}

/// A legal duplicate (same CC + identical payload) must NOT be flagged.
#[test]
fn cc_anomaly_legal_duplicate_not_flagged() {
    let mut ts = Vec::new();
    let pid = 0x0100u16;
    // First packet: CC=0
    let mut pkt = vec![0x47u8; 188];
    pkt[1] = ((pid >> 8) as u8) & 0x1F;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // AFC=01, CC=0
    pkt[4..].fill(0xAB); // payload content
    ts.extend_from_slice(&pkt);

    // Second packet: duplicate (same CC=0, identical payload)
    let mut pkt2 = vec![0x47u8; 188];
    pkt2[1] = ((pid >> 8) as u8) & 0x1F;
    pkt2[2] = (pid & 0xFF) as u8;
    pkt2[3] = 0x10; // AFC=01, CC=0 (same)
    pkt2[4..].fill(0xAB); // same payload
    ts.extend_from_slice(&pkt2);

    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();
    assert!(
        cc_findings.is_empty(),
        "legal duplicate should not be flagged: {cc_findings:?}"
    );
}

/// A discontinuity signalled via discontinuity_indicator must NOT be flagged.
#[test]
fn cc_anomaly_discontinuity_not_flagged() {
    let mut ts = Vec::new();
    let pid = 0x0100u16;
    // First packet: CC=0, no adaptation
    let mut pkt = vec![0x47u8; 188];
    pkt[1] = ((pid >> 8) as u8) & 0x1F;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // AFC=01, CC=0
    ts.extend_from_slice(&pkt);

    // Second packet: CC=8 (jump), adaptation with discontinuity_indicator=1
    let mut pkt2 = vec![0x47u8; 188];
    pkt2[1] = ((pid >> 8) as u8) & 0x1F;
    pkt2[2] = (pid & 0xFF) as u8;
    pkt2[3] = 0x30 | 8; // AFC=11 (adaptation+payload), CC=8
    pkt2[4] = 1; // adaptation_field_length = 1
    pkt2[5] = 0x80; // discontinuity_indicator = 1, no other flags
    pkt2[6] = 0xFF; // stuffing
    ts.extend_from_slice(&pkt2);

    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();
    assert!(
        cc_findings.is_empty(),
        "signalled discontinuity should not be flagged: {cc_findings:?}"
    );
}

/// m6-single.ts has exactly 879 CC anomalies (§2.4.3.3, via
/// `broadcast_common::ts_dup`: every same-CC repeat that is not
/// byte-identical to its predecessor outside the PCR field, and not a
/// signalled discontinuity, is a genuine continuity fault) concentrated on
/// PIDs 0x82/0x83/0x84.
#[test]
fn cc_anomaly_m6_single_has_many_errors() {
    let ts = fixture("m6-single.ts");
    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();
    assert_eq!(
        cc_findings.len(),
        879,
        "expected exactly 879 CC anomalies on m6-single.ts, got {}",
        cc_findings.len()
    );
    for f in &cc_findings {
        assert_eq!(f.severity, Severity::Error);
    }
    // Count findings on PID 0x82, 0x83, 0x84.
    let pids_82_83_84: Vec<_> = cc_findings
        .iter()
        .filter(|f| matches!(f.location.pid, 0x82..=0x84))
        .collect();
    assert!(
        pids_82_83_84.len() >= 70,
        "expected ≥70 CC anomalies on PIDs 0x82/0x83/0x84, got {}",
        pids_82_83_84.len()
    );
}

/// m6-duplicate.ts has exactly 5 true legal duplicates under the §2.4.3.3
/// byte-identical-except-PCR rule (cross-checked directly against
/// `broadcast_common::ts_dup::check_duplicate` below, independently of
/// `CcAnomalyCheck` itself) and zero illegal third-consecutive repeats.
/// Assert the legal duplicates are NOT flagged as errors, and that the
/// total anomaly count matches m6-single.ts exactly (the duplicates are
/// purely additional packets layered onto the same underlying stream, so
/// they must not change what's flagged).
#[test]
fn cc_anomaly_m6_duplicate_legal_dups_not_flagged() {
    let ts = fixture("m6-duplicate.ts");

    // Ground truth, computed independently of `CcAnomalyCheck` via the same
    // shared primitive it delegates to.
    let (legal, illegal_third) = count_legal_duplicates(&ts);
    assert_eq!(legal, 5, "expected 5 legal duplicates in m6-duplicate.ts");
    assert_eq!(
        illegal_third, 0,
        "expected 0 illegal third-repeats in m6-duplicate.ts"
    );

    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();

    // The fixture's duplicate packets are purely additional (layered on top
    // of m6-single.ts's own stream), so the flagged-anomaly count must be
    // identical to m6-single.ts's: none of the 5 legal duplicates should
    // themselves be flagged.
    assert_eq!(
        cc_findings.len(),
        879,
        "expected exactly 879 CC anomalies on m6-duplicate.ts, got {}",
        cc_findings.len()
    );
    let third_repeat_findings = cc_findings
        .iter()
        .filter(|f| f.message.contains("third consecutive"))
        .count();
    assert_eq!(
        third_repeat_findings, 0,
        "m6-duplicate.ts should not contain any illegal third-repeat findings"
    );
}

/// Count legal duplicates / illegal third-repeats on `ts` directly via
/// `broadcast_common::ts_dup`, independently of `CcAnomalyCheck`'s own use
/// of it — the oracle the assertions above cross-check against.
fn count_legal_duplicates(ts: &[u8]) -> (usize, usize) {
    use broadcast_common::ts_dup::{DuplicateVerdict, check_duplicate};
    use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};
    use std::collections::BTreeMap;

    struct St {
        last: Vec<u8>,
        dup_used: bool,
        initialised: bool,
    }

    let mut states: BTreeMap<u16, St> = BTreeMap::new();
    let mut legal = 0usize;
    let mut illegal_third = 0usize;
    let n = ts.len() / TS_PACKET_SIZE;
    for i in 0..n {
        let raw = &ts[i * TS_PACKET_SIZE..(i + 1) * TS_PACKET_SIZE];
        let Ok(pkt) = TsPacket::parse(raw) else {
            continue;
        };
        let pid = pkt.header.pid;
        if pid == 0x1FFF || !pkt.header.has_payload {
            continue;
        }
        let st = states.entry(pid).or_insert(St {
            last: Vec::new(),
            dup_used: false,
            initialised: false,
        });
        if !st.initialised {
            st.initialised = true;
            st.last = raw.to_vec();
            continue;
        }
        match check_duplicate(&st.last, raw, st.dup_used) {
            DuplicateVerdict::Legal => {
                legal += 1;
                st.dup_used = true;
            }
            DuplicateVerdict::IllegalThirdRepeat => {
                illegal_third += 1;
                st.dup_used = true;
            }
            DuplicateVerdict::NotDuplicate => {
                st.dup_used = false;
                st.last = raw.to_vec();
            }
            _ => unreachable!("unhandled DuplicateVerdict variant"),
        }
    }
    (legal, illegal_third)
}

/// Non-payload-bearing packets (AFC=10) should not advance CC for validation,
/// and a subsequent payload-bearing packet should continue from the last
/// payload-bearing CC.
#[test]
fn cc_anomaly_non_payload_does_not_advance_cc() {
    let mut ts = Vec::new();
    let pid = 0x0100u16;
    // Packet 1: payload-only, CC=0
    let mut pkt = vec![0x47u8; 188];
    pkt[1] = ((pid >> 8) as u8) & 0x1F;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // AFC=01, CC=0
    ts.extend_from_slice(&pkt);

    // Packet 2: adaptation-only (AFC=10), CC=1 — does NOT advance CC
    // (CC is technically undefined for non-payload; but the packet still has
    // a CC value in the header — we just don't use it for the next expected).
    let mut pkt2 = vec![0x47u8; 188];
    pkt2[1] = ((pid >> 8) as u8) & 0x1F;
    pkt2[2] = (pid & 0xFF) as u8;
    pkt2[3] = 0x20 | 5; // AFC=10 (adaptation only), CC=5
    pkt2[4] = 0; // adaptation_field_length = 0 (just one stuffing byte)
    ts.extend_from_slice(&pkt2);

    // Packet 3: payload-only, CC=1 (expected from packet 1's CC=0)
    let mut pkt3 = vec![0x47u8; 188];
    pkt3[1] = ((pid >> 8) as u8) & 0x1F;
    pkt3[2] = (pid & 0xFF) as u8;
    pkt3[3] = 0x10 | 1; // AFC=01, CC=1
    ts.extend_from_slice(&pkt3);

    let mut report = Report::new();
    CcAnomalyCheck.run(&ts, &mut report);
    let cc_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "cc-anomaly")
        .collect();
    assert!(
        cc_findings.is_empty(),
        "non-payload packets should not cause CC anomalies: {cc_findings:?}"
    );
}

// ── PatPmtVersionCheck tests ─────────────────────────────────────────────────

/// A clean stream with a constant PAT/PMT generation produces no version
/// findings — and, crucially, the sections really are parsed: the assertion
/// is paired with a positive one below, so a check that silently saw nothing
/// cannot pass this.
#[test]
fn pat_pmt_version_no_changes() {
    use dvb_si::tables::pmt::StreamType;

    let one = pat_pmt_ts(&[(0x0101, StreamType::H264)], 0, true);
    let mut ts = Vec::new();
    for _ in 0..3 {
        ts.extend_from_slice(&one);
    }

    let mut report = Report::new();
    PatPmtVersionCheck.run(&ts, &mut report);
    assert!(
        report
            .findings()
            .iter()
            .all(|f| f.rule_id != "pat-version" && f.rule_id != "pmt-version"),
        "a constant generation must produce no version findings, got {:?}",
        report.findings(),
    );

    // Positive control on the SAME builder: a version bump IS reported, which
    // proves the sections above were really parsed and tracked.
    let mut changed = Vec::new();
    changed.extend_from_slice(&one);
    changed.extend_from_slice(&one);
    changed.extend_from_slice(&pat_pmt_ts(&[(0x0101, StreamType::H264)], 1, true));
    let mut report2 = Report::new();
    PatPmtVersionCheck.run(&changed, &mut report2);
    let versions: Vec<_> = report2
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pmt-version")
        .collect();
    assert_eq!(
        versions.len(),
        1,
        "the same fixture builder with a version bump must be reported, proving the PAT/PMT were seen; got {:?}",
        report2.findings(),
    );
    assert!(
        versions[0].message.contains("0x0001"),
        "the finding names the program: {:?}",
        versions[0].message,
    );
}

/// Build a single-program TS carrying a real (CRC-correct, `dvb-si`-
/// serialized) PAT + PMT, with the given generation `version_number` and
/// `current_next_indicator`, declaring `(elementary_pid, stream_type)` pairs.
///
/// Built through `dvb-si`'s own builders and `mpeg-ts`'s packetiser — never
/// hand-rolled bytes. The PMT PID matches TSDuck's report for
/// `fixtures/ts/m6-single.ts` so the real-capture test can use the same
/// constant.
fn pat_pmt_ts(
    streams: &[(u16, dvb_si::tables::pmt::StreamType)],
    version: u8,
    current_next: bool,
) -> Vec<u8> {
    use broadcast_common::Serialize;
    use dvb_si::descriptors::any::DescriptorLoop;
    use dvb_si::tables::pat::{PatEntry, PatSection};
    use dvb_si::tables::pmt::{PmtSection, PmtStream};
    use mpeg_ts::mux::SectionPacketiser;
    use mpeg_ts::ts::TS_PACKET_SIZE;

    let pat = PatSection {
        transport_stream_id: 1,
        version_number: version,
        current_next_indicator: current_next,
        section_number: 0,
        last_section_number: 0,
        entries: vec![PatEntry {
            program_number: 1,
            pid: PAT_PMT_PID,
        }],
    };
    let mut pat_bytes = vec![0u8; pat.serialized_len()];
    let n = pat.serialize_into(&mut pat_bytes).expect("serialize PAT");
    pat_bytes.truncate(n);

    let pmt = PmtSection::new(
        1,
        version,
        current_next,
        0,
        0,
        streams.first().map(|&(pid, _)| pid).unwrap_or(0x1FFF),
        DescriptorLoop::new(&[]),
        streams
            .iter()
            .map(|&(pid, stream_type)| PmtStream {
                stream_type,
                elementary_pid: pid,
                es_info: DescriptorLoop::new(&[]),
            })
            .collect(),
    );
    let mut pmt_bytes = vec![0u8; pmt.serialized_len()];
    let n = pmt.serialize_into(&mut pmt_bytes).expect("serialize PMT");
    pmt_bytes.truncate(n);

    let mut ts = Vec::new();
    for packet in SectionPacketiser::new(dvb_si::tables::pat::PID).packetise(&[&pat_bytes]) {
        ts.extend_from_slice(&packet);
    }
    for packet in SectionPacketiser::new(PAT_PMT_PID).packetise(&[&pmt_bytes]) {
        ts.extend_from_slice(&packet);
    }
    assert_eq!(ts.len() % TS_PACKET_SIZE, 0);
    ts
}

/// The PMT PID `pat_pmt_ts` declares, and the one TSDuck reports for
/// `fixtures/ts/m6-single.ts`.
const PAT_PMT_PID: u16 = 0x0064;

/// A real capture: the check must *track* the PMT PID TSDuck reports, not
/// merely fail to complain.
///
/// Cross-checked with TSDuck 3.44 (and `tstables` as a second read):
///
/// ```text
/// $ tsanalyze fixtures/ts/m6-single.ts
/// |  0x0000  PAT .......................................... C   Unknown |
/// |  Service: 0x0401 (1025), TS: 0x0001 (1)                             |
/// |  PMT PID: 0x0064 (100), PCR PID: 0x0078 (120)                       |
/// $ tstables fixtures/ts/m6-single.ts --pid 0x0064
/// ```
#[test]
fn pat_pmt_version_real_capture_tracks_the_tsduck_pmt_pid() {
    // TSDuck's `tsanalyze` reports PID 0x0064 as the PMT PID of the single
    // program in m6-single.ts.
    const TSDUCK_PMT_PID: u16 = 0x0064;

    let ts = fixture("m6-single.ts");
    let mut report = Report::new();
    PatPmtVersionCheck.run(&ts, &mut report);
    assert!(
        report
            .findings()
            .iter()
            .all(|f| f.rule_id != "pat-version" && f.rule_id != "pmt-version"),
        "TSDuck reports a steady PAT/PMT for m6-single.ts; got {:?}",
        report.findings(),
    );

    // Positive proof the PAT was parsed and the PMT PID tracked: append a
    // second generation of the PAT's own PMT with a bumped version. If the
    // check never registered PID 0x0064, no `pmt-version` finding appears.
    let original = ts.clone();
    // Rewrite every PMT-bearing packet's version_number (the section spans
    // several packets, so only the PUSI packet carries the header) and
    // recompute the section CRC-32 (ISO/IEC 13818-1 Annex B). Mutating a
    // whole generation, not one packet, is what the version gate compares.
    let mut mutated = original.clone();
    let mut bumped = 0usize;
    for i in (0..mutated.len()).step_by(188) {
        let pid = ((u16::from(mutated[i + 1] & 0x1F)) << 8) | u16::from(mutated[i + 2]);
        if pid != TSDUCK_PMT_PID || mutated[i + 1] & 0x40 == 0 {
            continue;
        }
        // Fixed 4-byte TS header, then the adaptation field (when present),
        // then the `pointer_field` (present because PUSI is set).
        let afc = (mutated[i + 3] >> 4) & 0x03;
        let mut payload_start = i + 4;
        if afc & 0x02 != 0 {
            payload_start += 1 + usize::from(mutated[i + 4]);
        }
        payload_start += 1 + usize::from(mutated[payload_start]);
        let section_len = 3
            + (((usize::from(mutated[payload_start + 1]) & 0x0F) << 8)
                | usize::from(mutated[payload_start + 2]));
        let version_byte = payload_start + 5;
        // version_number is bits 5-1; reserved bits 7-6 and cni at bit 0.
        let old_version = (mutated[version_byte] >> 1) & 0x1F;
        let new_version = if old_version == 0 { 1 } else { 0 };
        mutated[version_byte] = (mutated[version_byte] & 0xE1) | (new_version << 1);
        let crc = broadcast_common::crc32_mpeg2::compute(
            &mutated[payload_start..payload_start + section_len - 4],
        );
        mutated[payload_start + section_len - 4..payload_start + section_len]
            .copy_from_slice(&crc.to_be_bytes());
        bumped += 1;
    }
    assert!(bumped > 0, "must find the PMT section TSDuck reports");

    let mut with_bump = original;
    with_bump.extend_from_slice(&mutated);
    let mut report2 = Report::new();
    PatPmtVersionCheck.run(&with_bump, &mut report2);
    let versions: Vec<_> = report2
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pmt-version")
        .collect();
    // Two programs share PID 0x0064 in this capture (table_id_ext 0x0401 and
    // 0x0601 — `tstables` reports both), and the bump above reaches both, so
    // exactly two findings are due. Asserting the exact set, with the
    // transition and severity, is what proves the PID was tracked *per
    // program*; a count of "at least one" would have hidden the fact that
    // the check distinguishes the two sub-tables.
    assert_eq!(
        versions.len(),
        2,
        "one PMT version change per program on PID {TSDUCK_PMT_PID:#06X}; got {:?}",
        report2.findings(),
    );
    for finding in &versions {
        assert_eq!(
            finding.severity,
            Severity::Info,
            "a version change is informational; got {finding:?}",
        );
        assert!(
            finding.message.contains("1 → 0"),
            "the message must name the transition; got {:?}",
            finding.message,
        );
    }
    let extensions: Vec<&str> = versions
        .iter()
        .map(|f| {
            f.message
                .rsplit_once("table_id_ext=")
                .expect("the finding names its table_id_ext")
                .1
                .trim_end_matches(')')
        })
        .collect();
    assert!(
        extensions.contains(&"0x0401") && extensions.contains(&"0x0601"),
        "both programs on the shared PID must be distinguished by          table_id_ext; got {extensions:?}",
    );

    // And the aliased PID a hand-rolled 4-byte-stride walk would invent from
    // the PAT's own CRC_32 (audit MD-W1(a)) must never be tracked.
    assert!(
        report2
            .findings()
            .iter()
            .all(|f| f.location.pid != u32::from(PAT_CRC_ALIASED_PID)),
        "PID {PAT_CRC_ALIASED_PID:#06X} is the PAT's CRC_32, never a PMT PID",
    );
}

/// The PID a hand-rolled 4-byte-stride walk over `m6-single.ts`'s PAT
/// section body derives from the trailing CRC_32 bytes (audit MD-W1(a)):
/// the PAT's CRC_32 is `F9 B4 63 EF`, so bytes 2..4 of it read as PID
/// `0x03EF`.
const PAT_CRC_ALIASED_PID: u16 = 0x03EF;

// ── PatPmtVersionCheck hostile-input tests ──────────────────────────────────

/// A PAT whose **payload** byte is corrupted (not just its CRC) must not
/// register any PMT PID: discovery runs the same CRC + `current_next` gate
/// as version tracking (audit MD-W1(a)/(c)).
///
/// The corrupt payload byte is chosen to flip a *program entry's* PID, so an
/// ungated walk would start watching the corrupted PID — visible because
/// that PID is fed PES bytes that reassemble into a `table_id 0x00` section,
/// producing garbage `pat-version` findings once repeated.
#[test]
fn corrupt_pat_payload_registers_no_pmt_pid() {
    use dvb_si::tables::pmt::StreamType;

    // A corrupt PAT must be dropped rather than parsed. The decisive check
    // that it registers no PMT PID lives in the module's own unit test
    // (`pat_pmt_version::tests::corrupt_pat_registers_no_pmt_pid`), where the
    // watched-PID set is observable; here the end-to-end observable is that
    // the components the corrupt PAT names are never reached.
    let one = pat_pmt_ts(&[(0x0101, StreamType::H264)], 0, true);
    let mut corrupt = one.clone();
    // Flip a byte in the PAT's program entry (payload, not CRC).
    let entry_pid_lo = 5 + 8 + 3;
    assert_eq!(corrupt[entry_pid_lo], (PAT_PMT_PID & 0xFF) as u8);
    corrupt[entry_pid_lo] ^= 0xFF;

    // A second, well-formed generation follows: the check must report nothing
    // about the corrupt one (its version never enters the table).
    let mut stream = Vec::new();
    for _ in 0..2 {
        stream.extend_from_slice(&corrupt);
        stream.extend_from_slice(&one);
    }

    let mut report = Report::new();
    PatPmtVersionCheck.run(&stream, &mut report);
    // The valid generation is constant, so no version finding is due; the
    // corrupt one must not have introduced a phantom change.
    assert!(
        report
            .findings()
            .iter()
            .all(|f| f.rule_id != "pat-version" && f.rule_id != "pmt-version"),
        "a corrupt PAT must not introduce a phantom version change; got {:?}",
        report.findings(),
    );
}

/// A PAT declaring `program_number 0` (the NIT) and the reserved NULL PID
/// must not put those PIDs under watch.
#[test]
fn nit_and_null_pids_are_not_tracked() {
    use broadcast_common::Serialize;
    use dvb_si::tables::pat::{PatEntry, PatSection};
    use mpeg_ts::mux::SectionPacketiser;

    let pat = PatSection {
        transport_stream_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: vec![
            PatEntry {
                program_number: 0, // NIT
                pid: 0x0010,
            },
            PatEntry {
                program_number: 1,
                pid: 0x1FFF, // reserved NULL PID
            },
        ],
    };
    let mut buf = vec![0u8; pat.serialized_len()];
    let n = pat.serialize_into(&mut buf).expect("serialize PAT");
    buf.truncate(n);

    let mut ts = Vec::new();
    for packet in SectionPacketiser::new(dvb_si::tables::pat::PID).packetise(&[&buf]) {
        ts.extend_from_slice(&packet);
    }
    // Feed the two candidate PIDs bytes that would reassemble as a PAT-shaped
    // section if they were ever watched.
    for pid in [0x0010u16, 0x1FFF] {
        let mut packet = vec![0x47u8; 188];
        packet[1] = 0x40 | ((pid >> 8) as u8 & 0x1F);
        packet[2] = (pid & 0xFF) as u8;
        packet[3] = 0x10;
        packet[4] = 0x00;
        packet[5] = 0x00; // table_id 0x00 — a "PAT" if watched
        packet[6] = 0xB0;
        packet[7] = 0x0D;
        ts.extend_from_slice(&packet);
    }

    let mut report = Report::new();
    PatPmtVersionCheck.run(&ts, &mut report);
    assert!(
        report.findings().is_empty(),
        "the NIT and NULL PIDs must never be put under watch; got {:?}",
        report.findings(),
    );
}

/// Hostile inputs must not panic: truncated sections, a section_length past
/// the packet, and a zero-length section.
#[test]
fn hostile_pat_pmt_inputs_do_not_panic() {
    // A PAT packet truncated to a few bytes.
    for len in [0usize, 1, 3, 4, 5, 7, 12, 100, 187] {
        let packet = vec![0x47u8; len];
        let mut report = Report::new();
        PatPmtVersionCheck.run(&packet, &mut report);
    }

    // A "PAT" section whose section_length claims more than the packet holds.
    let mut packet = vec![0x47u8; 188];
    packet[1] = 0x40;
    packet[2] = 0x00;
    packet[3] = 0x10;
    packet[4] = 0x00; // pointer_field
    packet[5] = 0x00; // table_id PAT
    packet[6] = 0xBF; // section_syntax=1, section_length high = 0xFFF
    packet[7] = 0xFF;
    let mut report = Report::new();
    PatPmtVersionCheck.run(&packet, &mut report);
    assert!(
        report.findings().is_empty(),
        "an over-long section_length must be rejected, not parsed; got {:?}",
        report.findings(),
    );

    // A zero-length section body.
    let mut packet = vec![0x47u8; 188];
    packet[1] = 0x40;
    packet[2] = 0x00;
    packet[3] = 0x10;
    packet[4] = 0x00;
    packet[5] = 0x00; // table_id PAT
    packet[6] = 0xB0; // section_length = 0
    packet[7] = 0x00;
    let mut report = Report::new();
    PatPmtVersionCheck.run(&packet, &mut report);
}

/// PES data arriving on a PID that carries a PMT must not be reassembled
/// into a section and misinterpreted (ISO/IEC 13818-1 §2.4.3.7 start code).
#[test]
fn pes_on_a_pmt_pid_is_not_read_as_a_section() {
    use dvb_si::tables::pmt::StreamType;

    let mut ts = pat_pmt_ts(&[(0x0101, StreamType::H264)], 0, true);
    // PES start code on the same PID as the PMT: `00 00 01 E0` declares a
    // section with table_id 0x00 and section_length 0 — not a PMT.
    for cc in 0..4u8 {
        let mut packet = vec![0x47u8; 188];
        packet[1] = 0x40 | ((PAT_PMT_PID >> 8) as u8 & 0x1F);
        packet[2] = (PAT_PMT_PID & 0xFF) as u8;
        packet[3] = 0x10 | (cc & 0x0F);
        packet[4] = 0x00;
        packet[5..9].copy_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
        ts.extend_from_slice(&packet);
    }
    let mut report = Report::new();
    PatPmtVersionCheck.run(&ts, &mut report);
    assert!(
        report.findings().is_empty(),
        "a PES start code on a PMT PID must not be read as a section; got {:?}",
        report.findings(),
    );
}

// ── PcrCheck tests ───────────────────────────────────────────────────────────

/// Path helper: fixture TS file from fixtures/ top level.
fn fixture_pcr(name: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
}

/// france-tnt-pcr.ts is a clean multi-PCR TS stream. PcrCheck must produce
/// zero PCR-error findings (Warning or Error severity on PCR rules).
#[test]
fn pcr_check_clean_fixture_no_errors() {
    let ts = fixture_pcr("france-tnt-pcr.ts");
    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);

    // We allow Info findings but no Warnings or Errors.
    let pcr_warn_or_err: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| {
            (f.rule_id == "pcr-repetition" || f.rule_id == "pcr-discontinuity")
                && matches!(f.severity, Severity::Warning | Severity::Error)
        })
        .collect();

    assert!(
        pcr_warn_or_err.is_empty(),
        "clean fixture should produce no PCR warnings/errors, got {}: {:#?}",
        pcr_warn_or_err.len(),
        pcr_warn_or_err,
    );
}

/// france-pcr-discontinuity.ts has a +10s PCR jump on PID 0x0208 with
/// discontinuity_indicator set. PcrCheck must NOT flag that jump.
#[test]
fn pcr_check_discontinuity_not_flagged() {
    let ts = fixture("france-pcr-discontinuity.ts");
    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);

    // No pcr-discontinuity findings on PID 0x0208 — the signalled jump
    // is legitimate.
    let disc_on_0208: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pcr-discontinuity" && f.location.pid == 0x0208)
        .collect();

    assert!(
        disc_on_0208.is_empty(),
        "signalled discontinuity on PID 0x0208 must not be flagged, got {disc_on_0208:?}",
    );

    // Also check that the discontinuity PID (0x0208) has no repetition errors.
    let rep_on_0208: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pcr-repetition" && f.location.pid == 0x0208)
        .collect();

    assert!(
        rep_on_0208.is_empty(),
        "PCR repetition on PID 0x0208 with signalled discontinuity should be clean: {rep_on_0208:?}",
    );
}

/// Positive case: take the clean fixture bytes, corrupt one PCR on PID 0x0208
/// by adding a large offset WITHOUT setting discontinuity_indicator, then
/// assert PcrCheck produces a PCR anomaly finding.
#[test]
fn pcr_check_corrupted_pcr_produces_finding() {
    let mut ts = fixture_pcr("france-tnt-pcr.ts");

    // Locate the first PCR-bearing packet on PID 0x0208.
    // PCR flag = 0x10 in adaptation field flags byte.
    let mut found = false;
    for i in (0..ts.len()).step_by(188) {
        let pid = (((ts[i + 1] & 0x1F) as u16) << 8) | ts[i + 2] as u16;
        if pid != 0x0208 {
            continue;
        }
        let afc = (ts[i + 3] >> 4) & 0x03;
        if afc < 2 {
            continue;
        }
        let af_len = ts[i + 4] as usize;
        if af_len == 0 {
            continue;
        }
        let flags = ts[i + 5];
        if flags & 0x10 == 0 {
            continue;
        }
        // Found a PCR packet on PID 0x0208. Corrupt the PCR base by adding
        // a large offset (~10s worth of 90 kHz base ticks) without setting
        // discontinuity_indicator.
        let pcr_start = i + 6;
        // Decode current base value.
        let base = ((ts[pcr_start] as u64) << 25)
            | ((ts[pcr_start + 1] as u64) << 17)
            | ((ts[pcr_start + 2] as u64) << 9)
            | ((ts[pcr_start + 3] as u64) << 1)
            | ((ts[pcr_start + 4] as u64) >> 7);
        // Add ~12 seconds to the base (12 × 90_000 = 1_080_000).
        let new_base = (base + 1_080_000) & 0x1_FFFF_FFFF;
        ts[pcr_start] = ((new_base >> 25) & 0xFF) as u8;
        ts[pcr_start + 1] = ((new_base >> 17) & 0xFF) as u8;
        ts[pcr_start + 2] = ((new_base >> 9) & 0xFF) as u8;
        ts[pcr_start + 3] = ((new_base >> 1) & 0xFF) as u8;
        ts[pcr_start + 4] = (ts[pcr_start + 4] & 0x7E)
            | (((new_base & 0x01) as u8) << 7)
            | (ts[pcr_start + 4] & 0x01);
        found = true;
        break;
    }

    assert!(
        found,
        "could not find a PCR packet on PID 0x0208 to corrupt"
    );

    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);

    // The corrupted PCR should produce findings on PID 0x0208.
    let pcr_findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| {
            f.location.pid == 0x0208
                && (f.rule_id == "pcr-repetition" || f.rule_id == "pcr-discontinuity")
        })
        .collect();

    assert!(
        !pcr_findings.is_empty(),
        "corrupted PCR on PID 0x0208 without discontinuity_indicator must produce a finding"
    );

    // At least one should be Error severity.
    let has_error = pcr_findings.iter().any(|f| f.severity == Severity::Error);
    assert!(
        has_error,
        "corrupted PCR should produce at least one Error: {pcr_findings:?}",
    );
}
