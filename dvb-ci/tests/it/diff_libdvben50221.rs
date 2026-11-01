//! Differential test: dvb-ci APDU tag constants vs the libdvben50221 canonical reference.
//!
//! The golden file (`en50221_tags_golden.txt`) was extracted from
//! `tbsdtv/dvb-apps lib/libdvben50221/en50221_app_tags.h` and committed next to this
//! test.  Each line has the form `TAG_NAME = 0xVALUE`.
//!
//! For every C `TAG_*` name that dvb-ci implements, we assert that the Rust constant's
//! `.as_u24()` equals the golden hex value byte-for-byte.  Tags present in the golden
//! file but not yet implemented in dvb-ci are collected into an informational
//! coverage-gap list (printed with `eprintln!`) and do NOT cause a test failure.

use dvb_ci::tag;
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Mapping: C golden name  →  dvb-ci Rust constant (ApduTag value).
//
// Names are taken verbatim from en50221_tags_golden.txt; values are the
// corresponding `dvb_ci::tag::*` constants.
// ---------------------------------------------------------------------------
fn build_mapping() -> HashMap<&'static str, u32> {
    let mut m: HashMap<&'static str, u32> = HashMap::new();

    // Resource Manager
    m.insert("TAG_PROFILE_ENQUIRY", tag::PROFILE_ENQ.as_u24());
    m.insert("TAG_PROFILE", tag::PROFILE.as_u24());
    m.insert("TAG_PROFILE_CHANGE", tag::PROFILE_CHANGE.as_u24());

    // Application Information
    m.insert("TAG_APP_INFO_ENQUIRY", tag::APPLICATION_INFO_ENQ.as_u24());
    m.insert("TAG_APP_INFO", tag::APPLICATION_INFO.as_u24());
    m.insert("TAG_ENTER_MENU", tag::ENTER_MENU.as_u24());

    // CA Support
    m.insert("TAG_CA_INFO_ENQUIRY", tag::CA_INFO_ENQ.as_u24());
    m.insert("TAG_CA_INFO", tag::CA_INFO.as_u24());
    m.insert("TAG_CA_PMT", tag::CA_PMT.as_u24());
    m.insert("TAG_CA_PMT_REPLY", tag::CA_PMT_REPLY.as_u24());

    // Host Control
    m.insert("TAG_TUNE", tag::TUNE.as_u24());
    m.insert("TAG_REPLACE", tag::REPLACE.as_u24());
    m.insert("TAG_CLEAR_REPLACE", tag::CLEAR_REPLACE.as_u24());
    m.insert("TAG_ASK_RELEASE", tag::ASK_RELEASE.as_u24());

    // Date-Time
    m.insert("TAG_DATE_TIME_ENQUIRY", tag::DATE_TIME_ENQ.as_u24());
    m.insert("TAG_DATE_TIME", tag::DATE_TIME.as_u24());

    // MMI
    m.insert("TAG_CLOSE_MMI", tag::CLOSE_MMI.as_u24());
    m.insert("TAG_DISPLAY_CONTROL", tag::DISPLAY_CONTROL.as_u24());
    m.insert("TAG_DISPLAY_REPLY", tag::DISPLAY_REPLY.as_u24());
    m.insert("TAG_TEXT_LAST", tag::TEXT_LAST.as_u24());
    m.insert("TAG_TEXT_MORE", tag::TEXT_MORE.as_u24());
    m.insert("TAG_KEYPAD_CONTROL", tag::KEYPAD_CONTROL.as_u24());
    m.insert("TAG_KEYPRESS", tag::KEYPRESS.as_u24());
    m.insert("TAG_ENQUIRY", tag::ENQ.as_u24());
    m.insert("TAG_ANSWER", tag::ANSW.as_u24());
    m.insert("TAG_MENU_LAST", tag::MENU_LAST.as_u24());
    m.insert("TAG_MENU_MORE", tag::MENU_MORE.as_u24());
    m.insert("TAG_MENU_ANSWER", tag::MENU_ANSW.as_u24());
    m.insert("TAG_LIST_LAST", tag::LIST_LAST.as_u24());
    m.insert("TAG_LIST_MORE", tag::LIST_MORE.as_u24());
    m.insert(
        "TAG_SUBTITLE_SEGMENT_LAST",
        tag::SUBTITLE_SEGMENT_LAST.as_u24(),
    );
    m.insert(
        "TAG_SUBTITLE_SEGMENT_MORE",
        tag::SUBTITLE_SEGMENT_MORE.as_u24(),
    );
    m.insert("TAG_DISPLAY_MESSAGE", tag::DISPLAY_MESSAGE.as_u24());
    m.insert("TAG_SCENE_END_MARK", tag::SCENE_END_MARK.as_u24());
    m.insert("TAG_SCENE_DONE", tag::SCENE_DONE.as_u24());
    m.insert("TAG_SCENE_CONTROL", tag::SCENE_CONTROL.as_u24());
    m.insert(
        "TAG_SUBTITLE_DOWNLOAD_LAST",
        tag::SUBTITLE_DOWNLOAD_LAST.as_u24(),
    );
    m.insert(
        "TAG_SUBTITLE_DOWNLOAD_MORE",
        tag::SUBTITLE_DOWNLOAD_MORE.as_u24(),
    );
    m.insert("TAG_FLUSH_DOWNLOAD", tag::FLUSH_DOWNLOAD.as_u24());
    m.insert("TAG_DOWNLOAD_REPLY", tag::DOWNLOAD_REPLY.as_u24());

    // Low-Speed Communications
    m.insert("TAG_COMMS_COMMAND", tag::COMMS_CMD.as_u24());
    m.insert(
        "TAG_CONNECTION_DESCRIPTOR",
        tag::CONNECTION_DESCRIPTOR.as_u24(),
    );
    m.insert("TAG_COMMS_REPLY", tag::COMMS_REPLY.as_u24());
    m.insert("TAG_COMMS_SEND_LAST", tag::COMMS_SEND_LAST.as_u24());
    m.insert("TAG_COMMS_SEND_MORE", tag::COMMS_SEND_MORE.as_u24());
    m.insert("TAG_COMMS_RECV_LAST", tag::COMMS_RCV_LAST.as_u24());
    m.insert("TAG_COMMS_RECV_MORE", tag::COMMS_RCV_MORE.as_u24());

    m
}

// ---------------------------------------------------------------------------
// Parse the golden file.
// ---------------------------------------------------------------------------
fn parse_golden(path: &std::path::Path) -> HashMap<String, u32> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read golden file {}: {e}", path.display()));

    let mut golden: HashMap<String, u32> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        // Skip blank lines and comments.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Expected form: `TAG_NAME = 0xVALUE` (possibly with trailing whitespace).
        let Some((name_part, val_part)) = line.split_once('=') else {
            continue;
        };
        let name = name_part.trim().to_string();
        let val_str = val_part.trim();
        let hex = val_str
            .strip_prefix("0x")
            .or_else(|| val_str.strip_prefix("0X"));
        let Some(hex) = hex else {
            continue;
        };
        let Ok(value) = u32::from_str_radix(hex, 16) else {
            continue;
        };
        golden.insert(name, value);
    }
    golden
}

// ---------------------------------------------------------------------------
// The test.
// ---------------------------------------------------------------------------
#[test]
fn diff_vs_libdvben50221() {
    // Locate the golden file relative to this crate's manifest directory.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest_dir.join("tests").join("en50221_tags_golden.txt");

    let golden = parse_golden(&golden_path);
    let mapping = build_mapping();

    assert!(
        mapping.len() >= 40,
        "expected at least 40 tag mappings, got {}",
        mapping.len()
    );

    let mut mismatches: Vec<String> = Vec::new();
    let mut coverage_gaps: Vec<String> = Vec::new();
    let mut matched = 0usize;

    // Walk every entry in the golden file.
    for (golden_name, golden_value) in &golden {
        match mapping.get(golden_name.as_str()) {
            Some(&dvb_ci_value) => {
                if dvb_ci_value != *golden_value {
                    mismatches.push(format!(
                        "  {golden_name}: dvb-ci=0x{dvb_ci_value:06X}, golden=0x{golden_value:06X}"
                    ));
                } else {
                    matched += 1;
                }
            }
            None => {
                coverage_gaps.push(format!("  {golden_name} = 0x{golden_value:06X}"));
            }
        }
    }

    // Report coverage gaps (informational only).
    if !coverage_gaps.is_empty() {
        let mut sorted = coverage_gaps.clone();
        sorted.sort();
        eprintln!(
            "\n[diff_libdvben50221] coverage gaps ({} tags in golden not mapped by dvb-ci):",
            coverage_gaps.len()
        );
        for g in &sorted {
            eprintln!("{g}");
        }
    }

    eprintln!(
        "\n[diff_libdvben50221] matched {matched} tags against libdvben50221 golden reference"
    );

    // Fail on any value mismatch.
    assert!(
        mismatches.is_empty(),
        "dvb-ci tag value(s) differ from libdvben50221 golden reference:\n{}",
        mismatches.join("\n")
    );

    // Sanity: at least 40 of our mapped consts appeared in the golden file.
    assert!(
        matched >= 40,
        "expected at least 40 matched tags, got {matched} — check mapping keys match the golden file"
    );
}
