//! Hard gate for the issue #567 codec-level checks: every new check must
//! produce **zero** mismatch findings across every real capture committed to
//! the repository (the `PtsCheck` lesson — a check that fires on real clean
//! content is rejected, not tuned around).
//!
//! Runs [`CodecSignallingCheck`], [`FpsCadenceCheck`], [`ParamSetsCheck`],
//! [`InterlaceCheck`], and [`check_container_codec`] against every `.ts`
//! under `fixtures/ts/**` and every file under `fixtures/ts/`.
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
    test_fixtures::root()
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

#[test]
fn avcc_level_above_the_sps_level_is_conformant() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    let mut baseline = Report::new();
    media_doctor::check_container_codec(&bytes, &mut baseline);
    assert!(
        baseline
            .findings()
            .iter()
            .all(|f| f.rule_id != "avcc-sps-mismatch"),
        "precondition: the unmodified fixture is clean: {:?}",
        baseline.findings()
    );
    let at = bytes
        .windows(4)
        .position(|w| w == b"avcC")
        .expect("avcC box");
    let level = at + 4 + 3;
    bytes[level] = bytes[level].saturating_add(1);
    let mut report = Report::new();
    media_doctor::check_container_codec(&bytes, &mut report);
    let hits: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "avcc-sps-mismatch")
        .collect();
    assert!(
        hits.is_empty(),
        "14496-15:2017 5.3.3.1: AVCLevelIndication need only be >= the highest SPS level; got {hits:?}"
    );
}

#[test]
fn hvcc_level_above_the_sps_level_is_conformant_and_below_is_not() {
    let original = std::fs::read(test_fixtures::path("mp4/hevc-main.mp4")).unwrap();
    let at = original
        .windows(4)
        .position(|w| w == b"hvcC")
        .expect("hvcC box");
    let level = at + 4 + 12;
    let hvcc_hits = |bytes: &[u8]| {
        let mut report = Report::new();
        media_doctor::check_container_codec(bytes, &mut report);
        report
            .findings()
            .iter()
            .filter(|f| f.rule_id == "hvcc-sps-mismatch" && f.message.contains("general_level_idc"))
            .count()
    };
    assert_eq!(
        hvcc_hits(&original),
        0,
        "precondition: the unmodified fixture is clean"
    );
    let mut above = original.clone();
    above[level] = above[level].saturating_add(3);
    assert_eq!(
        hvcc_hits(&above),
        0,
        "14496-15:2017 8.3.3.1.1: record level >= SPS level is conformant"
    );
    let mut below = original;
    below[level] = below[level].saturating_sub(3);
    assert_eq!(
        hvcc_hits(&below),
        1,
        "a record level below the SPS level is still flagged"
    );
}

// -------------------------------------------------------------------------
// AVC avcC-vs-SPS cross-checks (audit 23, finding 162)
// -------------------------------------------------------------------------

/// Run `check_container_codec` and return the (rule_id, severity, message) of
/// every finding on the given rule.
fn rule_hits(bytes: &[u8], rule_id: &str) -> Vec<(String, media_doctor::Severity)> {
    let mut report = Report::new();
    check_container_codec(bytes, &mut report);
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == rule_id)
        .map(|f| (f.message.clone(), f.severity))
        .collect()
}

fn sample_entry_offset(bytes: &[u8], codec_type: &[u8; 4]) -> usize {
    // The `ftyp` compatible-brands list also carries `avc1`, so anchor the
    // search at the `stsd` box that owns the sample entry.
    let stsd = bytes.windows(4).position(|w| w == b"stsd").expect("stsd");
    let position = bytes[stsd..]
        .windows(4)
        .position(|w| w == codec_type)
        .unwrap_or_else(|| panic!("{codec_type:?} sample entry"))
        + stsd;
    position - 4
}

fn avcc_record_offset(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|w| w == b"avcC").expect("avcC") + 4
}

fn hvcc_record_offset(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|w| w == b"hvcC").expect("hvcC") + 4
}

/// The avcC `chroma_format` field sits after the SPS/PPS arrays, whose lengths
/// are read from the record (ISO/IEC 14496-15:2017 §5.3.3.1.1).
fn avcc_chroma_offset(bytes: &[u8]) -> usize {
    let record = avcc_record_offset(bytes);
    let sps_len = u16::from_be_bytes([bytes[record + 6], bytes[record + 7]]) as usize;
    let pps_count = record + 8 + sps_len;
    let pps_len = u16::from_be_bytes([bytes[pps_count + 1], bytes[pps_count + 2]]) as usize;
    pps_count + 3 + pps_len
}

/// A differing `AVCProfileIndication` in the avcC record is one
/// `avcc-sps-mismatch` naming `profile_idc`.
#[test]
fn avcc_profile_disagreeing_with_the_sps_is_flagged() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    let record = avcc_record_offset(&bytes);
    // This fixture's avcC advertises High (0x64); Main (0x4D) disagrees with
    // the embedded SPS.
    bytes[record + 1] = 0x4D;
    let hits = rule_hits(&bytes, "avcc-sps-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);
    assert!(
        hits[0].0.contains("profile_idc"),
        "the finding names the SPS field: {:?}",
        hits[0].0
    );
}

/// An `AVCLevelIndication` below the embedded SPS level is one
/// `avcc-sps-mismatch` naming `AVCLevelIndication`.
#[test]
fn avcc_level_below_the_sps_level_is_flagged() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    let record = avcc_record_offset(&bytes);
    // The SPS level_idc is 13 (0x0D, Level 1.3); one below it is not.
    assert_eq!(bytes[record + 3], 0x0D, "fixture SPS level");
    bytes[record + 3] = 0x0C;
    let hits = rule_hits(&bytes, "avcc-sps-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);
    assert!(
        hits[0].0.contains("AVCLevelIndication"),
        "the finding names the record field: {:?}",
        hits[0].0
    );
}

/// A differing avcC `chroma_format` is one `avcc-sps-mismatch` naming
/// `chroma_format_idc`.
#[test]
fn avcc_chroma_disagreeing_with_the_sps_is_flagged() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    let chroma = avcc_chroma_offset(&bytes);
    // The reserved high bits stay, the 2-bit chroma_format_idc becomes 2
    // (4:2:2) against an SPS that says 1 (4:2:0).
    bytes[chroma] = (bytes[chroma] & !0b11) | 0b10;
    let hits = rule_hits(&bytes, "avcc-sps-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);
    assert!(
        hits[0].0.contains("chroma_format_idc"),
        "the finding names the SPS field: {:?}",
        hits[0].0
    );
}

/// The sample entry's width/height must match the SPS-decoded geometry; a zero
/// dimension means "undeclared" and is not compared.
#[test]
fn avc_sample_entry_dimensions_are_compared_to_the_sps() {
    let original = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    assert!(
        rule_hits(&original, "container-sps-dimension-mismatch").is_empty(),
        "precondition: matching dimensions are clean"
    );
    let entry = sample_entry_offset(&original, b"avc1");
    let width = entry + 32;
    let height = entry + 34;
    assert_eq!(
        (
            u16::from_be_bytes([original[width], original[width + 1]]),
            u16::from_be_bytes([original[height], original[height + 1]])
        ),
        (320, 240),
        "fixture geometry",
    );

    let mut mismatched = original.clone();
    mismatched[width..width + 2].copy_from_slice(&321u16.to_be_bytes());
    let hits = rule_hits(&mismatched, "container-sps-dimension-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);

    for (label, patch) in [("width", (width, 0u16)), ("height", (height, 0u16))] {
        let mut undeclared = original.clone();
        undeclared[patch.0..patch.0 + 2].copy_from_slice(&patch.1.to_be_bytes());
        assert!(
            rule_hits(&undeclared, "container-sps-dimension-mismatch").is_empty(),
            "a zero {label} means undeclared, not a mismatch: {:?}",
            rule_hits(&undeclared, "container-sps-dimension-mismatch"),
        );
    }
}

/// `frame_mbs_only_flag = 0` in the SPS is an Info (`avc-interlaced-content`);
/// a progressive SPS is not.
///
/// This fixture's SPS bit is flipped in place. Field coding also changes the
/// coded geometry, so the sample entry is updated to the geometry the patched
/// SPS decodes to (16x320) to isolate the interlace finding from the
/// dimension cross-check.
#[test]
fn an_interlaced_avc_sps_is_reported_and_a_progressive_one_is_not() {
    let original = std::fs::read(test_fixtures::path("mp4/h264-aac-prog.mp4")).unwrap();
    assert!(
        rule_hits(&original, "avc-interlaced-content").is_empty(),
        "precondition: the unmodified fixture is progressive"
    );

    let mut interlaced = original.clone();
    let record = avcc_record_offset(&interlaced);
    interlaced[record + 14] ^= 0b0001_0000;
    let entry = sample_entry_offset(&interlaced, b"avc1");
    interlaced[entry + 32..entry + 34].copy_from_slice(&16u16.to_be_bytes());
    interlaced[entry + 34..entry + 36].copy_from_slice(&320u16.to_be_bytes());

    let hits = rule_hits(&interlaced, "avc-interlaced-content");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].1,
        media_doctor::Severity::Info,
        "interlaced coding is a bitstream fact, not a mismatch"
    );
    assert!(
        rule_hits(&interlaced, "container-sps-dimension-mismatch").is_empty(),
        "the sample entry was updated to the patched SPS geometry"
    );
}

// -------------------------------------------------------------------------
// HEVC hvcC-vs-SPS cross-checks (audit 23, finding 163)
// -------------------------------------------------------------------------

/// A differing hvcC `general_profile_idc` is one `hvcc-sps-mismatch` naming
/// `general_profile_idc`.
#[test]
fn hvcc_profile_disagreeing_with_the_sps_is_flagged() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/hevc-main.mp4")).unwrap();
    let record = hvcc_record_offset(&bytes);
    // profile_space = 0, tier = 0, profile_idc = 2 (Main 10) against an SPS
    // that says Main (1).
    bytes[record + 1] = 0x02;
    let hits = rule_hits(&bytes, "hvcc-sps-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);
    assert!(
        hits[0].0.contains("general_profile_idc"),
        "the finding names the record field: {:?}",
        hits[0].0
    );
}

/// A differing hvcC `chroma_format_idc` is one `hvcc-sps-mismatch` naming
/// `chroma_format_idc`.
#[test]
fn hvcc_chroma_disagreeing_with_the_sps_is_flagged() {
    let mut bytes = std::fs::read(test_fixtures::path("mp4/hevc-main.mp4")).unwrap();
    let record = hvcc_record_offset(&bytes);
    // The reserved high bits stay, the 2-bit chroma_format_idc becomes 2
    // (4:2:2) against an SPS that says 1 (4:2:0).
    bytes[record + 16] = (bytes[record + 16] & !0b11) | 0b10;
    let hits = rule_hits(&bytes, "hvcc-sps-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);
    assert!(
        hits[0].0.contains("chroma_format_idc"),
        "the finding names the record field: {:?}",
        hits[0].0
    );
}

/// The sample entry's width/height must match the HEVC SPS-decoded geometry; a
/// zero dimension means "undeclared" and is not compared.
#[test]
fn hevc_sample_entry_dimensions_are_compared_to_the_sps() {
    let original = std::fs::read(test_fixtures::path("mp4/hevc-main.mp4")).unwrap();
    assert!(
        rule_hits(&original, "container-sps-dimension-mismatch").is_empty(),
        "precondition: matching dimensions are clean"
    );
    let entry = sample_entry_offset(&original, b"hev1");
    let width = entry + 32;
    let height = entry + 34;
    assert_eq!(
        (
            u16::from_be_bytes([original[width], original[width + 1]]),
            u16::from_be_bytes([original[height], original[height + 1]])
        ),
        (320, 240),
        "fixture geometry",
    );

    let mut mismatched = original.clone();
    mismatched[width..width + 2].copy_from_slice(&321u16.to_be_bytes());
    let hits = rule_hits(&mismatched, "container-sps-dimension-mismatch");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, media_doctor::Severity::Error);

    for (label, patch) in [("width", (width, 0u16)), ("height", (height, 0u16))] {
        let mut undeclared = original.clone();
        undeclared[patch.0..patch.0 + 2].copy_from_slice(&patch.1.to_be_bytes());
        assert!(
            rule_hits(&undeclared, "container-sps-dimension-mismatch").is_empty(),
            "a zero {label} means undeclared, not a mismatch: {:?}",
            rule_hits(&undeclared, "container-sps-dimension-mismatch"),
        );
    }
}
