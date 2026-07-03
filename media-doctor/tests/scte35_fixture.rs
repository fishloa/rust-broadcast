//! Fixture tests for `Scte35Check` — splice consistency diagnostics on
//! spec-valid SCTE-35 cues (CRC-correct `splice_insert` sections packetized into
//! real TS framing).
//!
//! The fixtures are:
//!
//! - `fixtures/ts/scte35-balanced.ts` — one TS packet (PID 0x01F0) carrying two
//!   `splice_info_section`s: an out (`event_id=100`, `out_of_network=true`) followed
//!   by the matching in (`event_id=100`, `out_of_network=false`). Assert zero
//!   unbalanced/duplicate findings.
//!
//! - `fixtures/ts/scte35-unbalanced.ts` — one TS packet (PID 0x01F0) carrying a
//!   single `splice_info_section`: an out (`event_id=200`, `out_of_network=true`)
//!   with no matching in. Assert at least one unbalanced-splice finding referencing
//!   event_id 200.

use std::fs;

use media_doctor::{Diagnostic, Report, Scte35Check};

fn read(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), rel);
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

fn scte35_unbalanced(report: &Report) -> Vec<&media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "scte35-unbalanced")
        .collect()
}

fn scte35_dup_out(report: &Report) -> Vec<&media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "scte35-dup-out")
        .collect()
}

/// `scte35-balanced.ts` carries a well-formed out→in pair. The check must
/// parse BOTH cues and report ZERO unbalanced or duplicate findings.
#[test]
fn scte35_balanced_fixture() {
    let ts = read("ts/scte35-balanced.ts");
    let mut report = Report::new();
    Scte35Check.run(&ts, &mut report);

    let unbal = scte35_unbalanced(&report);
    let dup = scte35_dup_out(&report);

    assert!(
        unbal.is_empty(),
        "balanced fixture should have no unbalanced findings, got {}: {:?}",
        unbal.len(),
        unbal,
    );
    assert!(
        dup.is_empty(),
        "balanced fixture should have no duplicate-out findings, got {}: {:?}",
        dup.len(),
        dup,
    );
}

/// `scte35-unbalanced.ts` carries a single out (event_id=200) with no matching
/// in. Assert AT LEAST ONE unbalanced-splice finding referencing event_id 200.
#[test]
fn scte35_unbalanced_fixture() {
    let ts = read("ts/scte35-unbalanced.ts");
    let mut report = Report::new();
    Scte35Check.run(&ts, &mut report);

    let unbal = scte35_unbalanced(&report);

    assert!(
        !unbal.is_empty(),
        "unbalanced fixture should have at least 1 unbalanced finding, got {}; report: {:?}",
        unbal.len(),
        report.findings(),
    );

    // At least one finding must reference event_id 200.
    let has_200 = unbal.iter().any(|f| f.message.contains("200"));
    assert!(
        has_200,
        "at least one unbalanced finding must mention event_id 200, got: {unbal:?}",
    );
}

/// `scte35-real.ts` carries the **canonical industry** `splice_insert` vector
/// (`4800008f`, event_id 0x4800008f, out_of_network=true) — a real SCTE-35
/// message from the spec/threefive corpus, packetized on PID 0x01F0. As a lone
/// "out" with no matching "in", `Scte35Check` must parse it and flag it
/// unbalanced — proving a real industry cue round-trips through our stack.
#[test]
fn real_canonical_splice_insert_parsed_and_flagged() {
    let ts = read("ts/scte35-real.ts");
    let mut report = Report::new();
    Scte35Check.run(&ts, &mut report);
    let unbal = scte35_unbalanced(&report);
    assert!(
        !unbal.is_empty(),
        "real canonical splice_insert (0x4800008f, lone out) must parse + flag \
         unbalanced, got {:?}",
        report.findings(),
    );
    // event_id 0x4800008f = 1207959695
    assert!(
        unbal.iter().any(|f| f.message.contains("1207959695")),
        "unbalanced finding should reference the real event_id 1207959695: {unbal:?}",
    );
}
