//! Real-fixture tests for `PcrCheck` — TR 101 290 PCR diagnostics on genuine
//! broadcast captures (false-positive check + discontinuity honouring).

use std::fs;

use media_doctor::{Diagnostic, PcrCheck, Report, Severity};

fn read(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), rel);
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

fn pcr_errors(report: &Report) -> Vec<&media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect()
}

/// A clean real multi-PCR-PID broadcast capture must produce no PCR *errors*
/// (the false-positive check synthetic packets can't give).
#[test]
fn pcr_clean_real_stream_no_errors() {
    let ts = read("france-tnt-pcr.ts");
    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);
    let errs = pcr_errors(&report);
    assert!(
        errs.is_empty(),
        "clean france-tnt-pcr.ts should yield no PCR errors, got {}: {:?}",
        errs.len(),
        errs
    );
}

/// `france-pcr-discontinuity.ts` carries a *signalled* system-time-base
/// discontinuity (discontinuity_indicator=1) + a +10s PCR jump on PID 0x208.
/// A correct PcrCheck honours the flag and does NOT raise an error — if it
/// flagged the jump this would fail (the bite).
#[test]
fn pcr_signalled_discontinuity_not_flagged() {
    let ts = read("ts/france-pcr-discontinuity.ts");
    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);
    let errs = pcr_errors(&report);
    assert!(
        errs.is_empty(),
        "signalled discontinuity must not be flagged as a PCR error, got {}: {:?}",
        errs.len(),
        errs
    );
}
