//! Real-fixture tests for `PtsCheck` — PTS/DTS diagnostics on genuine broadcast
//! captures (backward, wrap, forbidden flags).

use std::fs;

use media_doctor::{Diagnostic, PtsCheck, Report, Severity};

fn read(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), rel);
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

fn pts_errors(report: &Report) -> Vec<&media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect()
}

/// `pts-backward.ts` carries 5 PES on PID 0x100 with PTS values 90000, 93000,
/// 96000, 40000, 99000. The 96000 → 40000 transition is a genuine backward jump.
#[test]
fn pts_backward_detected() {
    let ts = read("ts/pts-backward.ts");
    let mut report = Report::new();
    PtsCheck.run(&ts, &mut report);
    let errors = pts_errors(&report);
    let backward: Vec<_> = errors
        .iter()
        .filter(|f| f.rule_id == "pts-backward" && f.location.pid == 0x100)
        .collect();
    assert!(
        !backward.is_empty(),
        "expected at least one pts-backward error on PID 0x100 in pts-backward.ts, \
         got {} total errors: {:?}",
        errors.len(),
        errors,
    );
}

/// `pts-wrap.ts` carries PTS 2^33-5000, 2^33-2000, 1000, 4000, 7000 — a legal
/// 33-bit wrap. No backward jump should be flagged.
#[test]
fn pts_wrap_not_flagged() {
    let ts = read("ts/pts-wrap.ts");
    let mut report = Report::new();
    PtsCheck.run(&ts, &mut report);
    let backward: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pts-backward")
        .collect();
    assert!(
        backward.is_empty(),
        "expected zero pts-backward errors in pts-wrap.ts, got {}: {:?}",
        backward.len(),
        backward,
    );
}

/// An inline hand-built PES with `PTS_DTS_flags == 0b01` must be flagged as an
/// error (forbidden by ITU-T H.222.0 §2.4.3.7).
#[test]
fn forbidden_pts_dts_flags_detected() {
    // Build a TS packet containing a PES with forbidden flags.
    // PES: start_code(3) | stream_id(1) | length(2) | flags1(1) | flags2(1) | hdr_len(1) | payload
    // flags2 = 0x40 → PTS_DTS_flags = 0b01
    let payload: &[u8] = &[0xAA, 0xBB];
    let mut pes = vec![0x00, 0x00, 0x01, 0xE0];
    let length = 3u16 + payload.len() as u16; // header part + payload
    pes.extend_from_slice(&length.to_be_bytes());
    pes.push(0x80); // flags1: marker bit
    pes.push(0x40); // flags2: PTS_DTS_flags = 01 (forbidden)
    pes.push(0x00); // PES_header_data_length = 0
    pes.extend_from_slice(payload);
    // Pad to exactly 188 bytes.
    let mut ts = vec![0x47u8; 188];
    ts[1] = 0x41; // PUSI=1, PID low bits start; actual PID = 0x0100
    ts[1] |= 0x01; // PID[8] = 1
    ts[2] = 0x00; // PID[7:0] = 0x00
    ts[3] = 0x10; // AFC=01 (no adaptation), CC=0

    let copy_len = pes.len().min(184);
    ts[4..4 + copy_len].copy_from_slice(&pes[..copy_len]);

    let mut report = Report::new();
    PtsCheck.run(&ts, &mut report);
    let ff: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pts-forbidden-flags")
        .collect();
    assert!(
        !ff.is_empty(),
        "expected at least one pts-forbidden-flags error, got {} total findings: {:?}",
        report.len(),
        report.findings(),
    );
}

// ---------------------------------------------------------------------------
// Real-capture negatives (regression): the fault-detector must NOT false-positive
// on clean real broadcast/encoded streams. Before the DTS-based + PES-PID-gated
// fix, PtsCheck flagged legal B-frame PTS reordering as `pts-backward` (dozens of
// errors on h264_aac.ts) and misread EIT section bytes on PID 0x0012 as a PES
// header (`pts-forbidden-flags` on france-tnt-pcr.ts).
// ---------------------------------------------------------------------------

/// `h264_aac.ts` — a clean real H.264 (with B-frames) + AAC capture. Its video
/// PTS reorders vs decode order, but DTS is monotonic → zero PtsCheck errors.
#[test]
fn real_h264_aac_no_false_positives() {
    let ts = read("ts/h264_aac.ts");
    let mut report = Report::new();
    PtsCheck.run(&ts, &mut report);
    let errs = pts_errors(&report);
    assert!(
        errs.is_empty(),
        "clean real H.264+AAC stream must yield no PtsCheck errors (B-frame PTS \
         reorder is legal), got {}: {:?}",
        errs.len(),
        errs,
    );
}

/// `france-tnt-pcr.ts` — a real multi-programme DVB capture (video PIDs with
/// B-frame reorder + PSI/SI PIDs like EIT 0x0012). Must yield no PtsCheck errors:
/// DTS is monotonic and non-PES PIDs are skipped.
#[test]
fn real_france_capture_no_false_positives() {
    let ts = read("france-tnt-pcr.ts");
    let mut report = Report::new();
    PtsCheck.run(&ts, &mut report);
    let errs = pts_errors(&report);
    assert!(
        errs.is_empty(),
        "clean real DVB capture must yield no PtsCheck errors, got {}: {:?}",
        errs.len(),
        errs,
    );
}
