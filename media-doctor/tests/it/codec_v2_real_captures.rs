//! Hard gate for the issue #567 codec-level checks: every new check must
//! produce **zero** mismatch findings across every real capture committed to
//! the repository (the `PtsCheck` lesson — a check that fires on real clean
//! content is rejected, not tuned around).
//!
//! Runs [`CodecSignallingCheck`], [`FpsCadenceCheck`], [`ParamSetsCheck`],
//! [`InterlaceCheck`], and [`check_container_codec`] against every `.ts`
//! under `fixtures/ts/**` and every file under `fixtures/transmux/`.
//!
//! `avc-interlaced-content` / `interlaced-content` are excluded from the
//! "mismatch" set on purpose: `fixtures/ts/h264/interlaced.ts` is genuinely
//! interlaced content, and correctly firing that Info/Warning there is the
//! check working as designed, not a false positive.

use std::fs;
use std::path::{Path, PathBuf};

use media_doctor::{
    CodecSignallingCheck, Diagnostic, FpsCadenceCheck, InterlaceCheck, ParamSetsCheck, Report,
    check_container_codec,
};

/// Rule IDs that report a plain bitstream fact rather than a claimed
/// signalling/bitstream mismatch — expected to fire on real (correctly
/// flagged) interlaced content, so excluded from the "false positive" set.
const NON_MISMATCH_RULE_IDS: &[&str] = &["avc-interlaced-content", "interlaced-content"];

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

/// Recursively collect every regular file under `dir`.
fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Run every issue #567 check against `bytes`, returning only "mismatch"
/// findings (see [`NON_MISMATCH_RULE_IDS`]).
fn mismatch_findings(bytes: &[u8]) -> Vec<media_doctor::Finding> {
    let mut report = Report::new();
    let diagnostics: &[&dyn Diagnostic] = &[
        &CodecSignallingCheck,
        &FpsCadenceCheck,
        &ParamSetsCheck,
        &InterlaceCheck,
    ];
    media_doctor::run_all(bytes, diagnostics, &mut report);
    check_container_codec(bytes, &mut report);

    report
        .findings()
        .iter()
        .filter(|f| !NON_MISMATCH_RULE_IDS.contains(&f.rule_id.as_str()))
        .cloned()
        .collect()
}

#[test]
fn zero_false_positives_across_every_real_capture() {
    let root = fixtures_root();
    let mut candidates = Vec::new();
    walk_files(&root.join("ts"), &mut candidates);
    walk_files(&root.join("transmux"), &mut candidates);

    assert!(
        candidates.len() > 10,
        "expected to find real fixtures under {}/{{ts,transmux}}, found {} — fixture layout \
         may have moved",
        root.display(),
        candidates.len()
    );

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for path in &candidates {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !matches!(ext, "ts" | "mp4" | "m4s" | "m4a" | "m4v" | "cmfv" | "cmfa") {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        checked += 1;

        let findings = mismatch_findings(&bytes);
        if !findings.is_empty() {
            failures.push(format!("{}: {:?}", path.display(), findings));
        }
    }

    assert!(
        checked > 10,
        "expected to check more than 10 real fixture files, checked {checked}"
    );
    assert!(
        failures.is_empty(),
        "false positive(s) on real committed captures ({checked} files checked):\n{}",
        failures.join("\n")
    );
}
