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
//!
//! - `fixtures/ts/scte35-other-pid.ts` (issue #1046 / audit MD-C1) — a real
//!   TSDuck-built stream (see `fixtures/ts/scte35-other-pid-PROVENANCE.md` for
//!   the exact `tsp`/`tstabcomp` commands) whose PMT declares the SCTE-35 PID
//!   as `0x0150` (`stream_type=0x86` + a `registration_descriptor` for
//!   `"CUEI"`) — deliberately **not** the conventional `0x01F0` — carrying a
//!   lone out `splice_insert` (`event_id=777`). Proves `Scte35Check` finds the
//!   cue via PMT-driven PID discovery rather than the old hard-coded PID.

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
/// (`0x4800008f`, event_id 0x4800008f, out_of_network=true) — a real SCTE-35
/// message from the spec/threefive corpus, packetized on PID 0x01F0.
///
/// Decoded with this workspace's own `scte35-splice` parser, the vector's
/// `break_duration()` is present with `auto_return = true` (duration 5426421
/// ticks), so it is a *self-closing* break: the splicer returns after
/// `duration` with no separate "in" cue (ANSI/SCTE 35 §9.8.2 / §9.9.2.2).
/// The check must therefore parse the real cue and report **no** unbalanced
/// finding — the vector is complete signalling, not a missing return.
///
/// This asserts the cue was *seen*, not merely that nothing was reported: the
/// fixture's `splice_info_section` is re-parsed here with the same
/// `scte35-splice` parser and must yield the vector's own event id and
/// auto-return break. A control case then shows what the check does report
/// for the same event id once `auto_return` is cleared.
#[test]
fn real_canonical_splice_insert_parsed_and_not_unbalanced() {
    use broadcast_common::Parse;
    use scte35_splice::commands::AnyCommand;

    /// The canonical vector's own event id.
    const REAL_EVENT_ID: u32 = 0x4800_008f;

    let ts = read("ts/scte35-real.ts");
    let section_start = ts
        .iter()
        .position(|&b| b == 0xfc)
        .expect("fixture must carry a splice_info_section (table_id 0xFC)");
    let section_len = 3
        + (((usize::from(ts[section_start + 1]) & 0x0F) << 8) | usize::from(ts[section_start + 2]));
    let section = &ts[section_start..section_start + section_len];

    // The vector's own identity, asserted against the fixture's bytes with
    // the workspace's parser — independent of `Scte35Check`.
    let sis = scte35_splice::SpliceInfoSection::parse(section).expect("parse the real cue");
    let AnyCommand::SpliceInsert(si) = &sis.clear.as_ref().expect("clear section").command else {
        panic!("the canonical vector is a splice_insert");
    };
    assert_eq!(
        si.splice_event_id, REAL_EVENT_ID,
        "the real vector's event id"
    );
    assert!(si.out_of_network_indicator, "the real vector is an out");
    assert!(
        si.break_duration.as_ref().is_some_and(|b| b.auto_return),
        "the real vector's break_duration auto-returns",
    );

    // The check itself: the cue is parsed and is NOT unbalanced.
    let mut report = Report::new();
    Scte35Check.run(&ts, &mut report);
    assert!(
        scte35_unbalanced(&report).is_empty(),
        "the real canonical splice_insert (0x4800008f) carries break_duration.auto_return = true, so it closes at its own duration and must not be flagged unbalanced; got {:?}",
        report.findings(),
    );

    // Control: the same lone "out" WITHOUT auto_return IS reported, which
    // proves the check reached the cue rather than silently skipping the
    // fixture. The `auto_return` bit is located by re-parsing candidates
    // rather than by hand-computing the splice_insert layout: clear each
    // high bit in turn, recompute the CRC, and take the candidate whose parse
    // yields the same event with `auto_return` cleared.
    let mut control: Option<Vec<u8>> = None;
    for offset in 0..section_len.saturating_sub(4) {
        let bit = section_start + offset;
        if ts[bit] & 0x80 == 0 {
            continue;
        }
        let mut candidate = ts.clone();
        candidate[bit] &= 0x7F;
        let crc = broadcast_common::crc32_mpeg2::compute(
            &candidate[section_start..section_start + section_len - 4],
        );
        candidate[section_start + section_len - 4..section_start + section_len]
            .copy_from_slice(&crc.to_be_bytes());
        let Ok(sis) = scte35_splice::SpliceInfoSection::parse(
            &candidate[section_start..section_start + section_len],
        ) else {
            continue;
        };
        let Some(clear) = sis.clear.as_ref() else {
            continue;
        };
        let AnyCommand::SpliceInsert(si) = &clear.command else {
            continue;
        };
        if si.splice_event_id == REAL_EVENT_ID
            && si.out_of_network_indicator
            && si.break_duration.as_ref().is_some_and(|b| !b.auto_return)
        {
            control = Some(candidate);
            break;
        }
    }
    let control = control.expect("locate the auto_return bit by re-parsing candidates");

    let mut control_report = Report::new();
    Scte35Check.run(&control, &mut control_report);
    let control_unbal = scte35_unbalanced(&control_report);
    assert!(
        !control_unbal.is_empty(),
        "the same lone out WITHOUT auto_return must be reported, proving the check saw the cue; got {:?}",
        control_report.findings(),
    );
    assert!(
        control_unbal
            .iter()
            .any(|f| f.message.contains("1207959695")),
        "the control finding names the real event id 1207959695; got {control_unbal:?}",
    );
}

/// `scte35-other-pid.ts` carries its cue on PMT-declared PID `0x0150`, not the
/// conventional `0x01F0` (issue #1046 / audit MD-C1). Before the fix,
/// `Scte35Check` only ever looked at `0x01F0` and would report zero findings
/// here — a "No issues found." false clean on a stream that plainly needs a
/// splice-balance check.
#[test]
fn real_scte35_on_non_default_pid_is_found() {
    let ts = read("ts/scte35-other-pid.ts");
    let mut report = Report::new();
    Scte35Check.run(&ts, &mut report);
    let unbal = scte35_unbalanced(&report);
    assert!(
        !unbal.is_empty(),
        "SCTE-35 cue on PMT-declared PID 0x0150 must be found and flagged unbalanced \
         (lone out, event_id 777), got {:?}",
        report.findings(),
    );
    assert!(
        unbal.iter().any(|f| f.message.contains("777")),
        "unbalanced finding should reference event_id 777: {unbal:?}",
    );
}
