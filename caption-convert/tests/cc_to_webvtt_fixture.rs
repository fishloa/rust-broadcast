//! Fixture-driven CEA-608 -> WebVTT/SRT test, exercising
//! `caption_convert::Cea608ToWebVtt` end to end against the same real
//! fixture `timed-metadata`'s own #568 test uses:
//! `fixtures/cc/cea608_cc1_synthetic.txt` (see that fixture's header comment
//! for its provenance and caption plan -- pop-on "HELLO", roll-up
//! "HI"->"HI\nBYE"->"BYE", paint-on "OK").
//!
//! This proves the wrapper (raw `cc_data()` bytes in, WebVTT/SRT string out)
//! correctly threads through to `timed-metadata`'s extractor -- the
//! extractor's own correctness (the roll-up/pop-on/paint-on state machine)
//! is `timed-metadata`'s test, not re-verified here.
//!
//! The mutation-bite test below never writes to the shared fixture file on
//! disk (mutating a workspace-shared fixture, even temporarily, risks
//! corrupting it for concurrently-running tests in other crates) -- it
//! mutates the loaded text **in memory** only.
#![cfg(feature = "cc-data")]

use caption_convert::Cea608ToWebVtt;
use cc_data::decode::Cea608Channel;
use std::fs;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("cc")
        .join("cea608_cc1_synthetic.txt")
}

fn load_fixture_text() -> String {
    fs::read_to_string(fixture_path()).expect("read cea608_cc1_synthetic.txt fixture")
}

fn load_frames(text: &str) -> Vec<(u64, Vec<u8>)> {
    let mut frames = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let pts: u64 = parts
            .next()
            .expect("pts field")
            .parse()
            .expect("pts is a u64");
        let hex = parts.next().expect("hex field");
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex byte"))
            .collect();
        frames.push((pts, bytes));
    }
    frames
}

fn convert_text(text: &str) -> Cea608ToWebVtt {
    let frames = load_frames(text);
    let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
    for (pts, bytes) in &frames {
        conv.push_cc_data(*pts, bytes)
            .expect("valid cc_data() Table B.9 bytes");
    }
    conv.finalize(45_000);
    conv
}

fn convert() -> Cea608ToWebVtt {
    let text = load_fixture_text();
    let frames = load_frames(&text);
    assert_eq!(frames.len(), 13, "fixture frame count changed unexpectedly");
    convert_text(&text)
}

#[test]
fn webvtt_output_contains_the_full_caption_plan() {
    let vtt = convert().into_webvtt();
    assert!(vtt.starts_with("WEBVTT\n\n"));
    for expected in ["HELLO", "HI", "BYE", "OK"] {
        assert!(vtt.contains(expected), "missing {expected:?} in:\n{vtt}");
    }
    // Roll-up two-row cue: both rows visible together at some point.
    assert!(vtt.contains("HI\nBYE"));
}

#[test]
fn srt_output_contains_the_full_caption_plan_and_uses_comma_timestamps() {
    let srt = convert().into_srt();
    assert!(srt.starts_with("1\n"));
    for expected in ["HELLO", "HI", "BYE", "OK"] {
        assert!(srt.contains(expected), "missing {expected:?} in:\n{srt}");
    }
    assert!(
        srt.contains(","),
        "SRT must use comma millisecond separators"
    );
    assert!(
        !srt.contains('.'),
        "SRT must not use WebVTT's dot separator"
    );
}

/// Bite test: corrupt (in memory only -- never on disk) the fixture's
/// pop-on "EOC" control-code byte (the commit event for "HELLO") so it no
/// longer decodes as EOC, show the expected cue text disappears, then use
/// the unmutated text and show it is present -- proving this test would
/// actually catch a regression, not just always pass.
#[test]
fn mutation_bite_corrupting_the_eoc_byte_drops_the_hello_cue() {
    let original = load_fixture_text();

    // Locate the EOC frame line (per the fixture's own documented plan:
    // pts 6000 is "HELLO"'s commit event, cc bytes 0x14 0x2F -- carried on
    // the wire with CEA-608's odd-parity bit already applied to 0x14
    // (even bit count -> parity set -> 0x94), so the wire hex is "942f").
    // Flip the control byte's low bit (0x2F -> 0x2E) so the line still
    // parses as *a* cc_data() structure, but no longer carries the EOC
    // control code that commits the pop-on buffer.
    let target_line = original
        .lines()
        .find(|l| l.trim_start().starts_with("6000 "))
        .expect("fixture must have a pts=6000 frame line");
    assert!(
        target_line.contains("942f"),
        "expected the EOC wire bytes '942f' on the pts=6000 line: {target_line:?}"
    );
    let mutated: String = original
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("6000 ") {
                line.replace("942f", "942e")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(
        mutated, original,
        "mutation must actually change the fixture text"
    );

    let mutated_vtt = convert_text(&mutated).into_webvtt();
    assert!(
        !mutated_vtt.contains("HELLO"),
        "mutating the EOC byte must drop the HELLO cue, but it is still present:\n{mutated_vtt}"
    );

    let original_vtt = convert_text(&original).into_webvtt();
    assert!(
        original_vtt.contains("HELLO"),
        "unmutated fixture must still produce HELLO"
    );
}
