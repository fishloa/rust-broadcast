//! End-to-end tests for the `check-hls` and `check-dash` subcommands (audit
//! 23, finding 159).
//!
//! `cli_routing.rs` drives only `check` and `watch` through the binary, so the
//! two playlist/manifest subcommands (and their `--json` flag) had no test at
//! all: they could print nothing, return success on a broken playlist, or lose
//! the JSON rendering unnoticed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch input file under the workspace `target/` (already gitignored),
/// namespaced by test thread so parallel runs never read a half-written file.
fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/media-doctor-cli");
    fs::create_dir_all(&dir).expect("create scratch dir");
    let owner = std::thread::current()
        .name()
        .unwrap_or("main")
        .replace("::", "-");
    let path = dir.join(format!("{owner}-{name}"));
    fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    path
}

fn run(args: &[&str]) -> Output {
    let exe = env!("CARGO_BIN_EXE_media-doctor");
    assert!(Path::new(exe).exists(), "cargo must build the binary");
    Command::new(exe)
        .args(args)
        .output()
        .expect("media-doctor binary must run")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn json_rule_ids(text: &str) -> Vec<String> {
    let value: serde_json::Value = serde_json::from_str(text).expect("stdout must be JSON");
    value["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter_map(|finding| finding["rule_id"].as_str().map(str::to_owned))
        .collect()
}

/// `bad-extinf.m3u8` (EXTINF 15.0 against TARGETDURATION 10) must be reported
/// in the text rendering and in `--json`, at exit 0.
#[test]
fn check_hls_reports_a_bad_extinf_in_text_and_json() {
    let path = test_fixtures::path("hls/bad-extinf.m3u8");
    let input = path.display().to_string();

    let out = run(&["check-hls", "--input", &input]);
    assert!(out.status.success(), "{:?}", out);
    let text = stdout(&out);
    assert!(text.contains("Findings: 1 error(s)"), "{text}");
    assert!(text.contains("hls-extinf-exceeds-target"), "{text}");

    let out = run(&["check-hls", "--input", &input, "--json"]);
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        json_rule_ids(&stdout(&out)),
        ["hls-extinf-exceeds-target"],
        "the JSON findings array must carry the rule id",
    );
}

/// A duplicate Representation `@id` must be reported by `check-dash`, in text
/// and in `--json`.
#[test]
fn check_dash_reports_a_duplicate_representation_id_in_text_and_json() {
    let mpd = "<?xml version=\"1.0\"?>\n\
        <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
        mediaPresentationDuration=\"PT10S\">\n\
        <Period id=\"p0\"><AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\n\
        <Representation id=\"dup\" bandwidth=\"1000000\" width=\"640\" height=\"360\">\
        <BaseURL>v1.mp4</BaseURL></Representation>\n\
        <Representation id=\"dup\" bandwidth=\"2000000\" width=\"1280\" height=\"720\">\
        <BaseURL>v2.mp4</BaseURL></Representation>\n\
        </AdaptationSet></Period></MPD>\n";
    let path = scratch("duplicate-rep-id.mpd", mpd.as_bytes());
    let input = path.display().to_string();

    let out = run(&["check-dash", "--input", &input]);
    assert!(out.status.success(), "{:?}", out);
    assert!(
        stdout(&out).contains("dash-representation-id-duplicate"),
        "{}",
        stdout(&out)
    );

    let out = run(&["check-dash", "--input", &input, "--json"]);
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        json_rule_ids(&stdout(&out)),
        ["dash-representation-id-duplicate"],
    );
}

/// A nonexistent input is a tool error for both subcommands: exit 1 and an
/// `error:` line on stderr, never a silent success.
#[test]
fn a_missing_input_file_is_an_error_for_both_subcommands() {
    for subcommand in ["check-hls", "check-dash"] {
        let out = run(&[subcommand, "--input", "target/does-not-exist.m3u8"]);
        assert!(!out.status.success(), "{subcommand}: {:?}", out);
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("error:"),
            "{subcommand}: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
