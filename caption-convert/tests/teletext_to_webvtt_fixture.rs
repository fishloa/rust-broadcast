//! Fixture-driven EBU Teletext -> WebVTT/SRT test, exercising
//! `caption_convert::TeletextToWebVtt` end to end against the same real
//! fixture `timed-metadata`'s own #666 test uses:
//! `fixtures/teletext/teletext_subtitle_synthetic.txt` (magazine 8, page
//! 0x88; see that fixture's header comment for its provenance and page
//! plan -- header/erase, "HELLO WORLD" row, "THIS IS A TEST" row, then a
//! second erase).
//!
//! This proves the wrapper (raw 44-byte Teletext data-field wire in,
//! WebVTT/SRT string out) correctly threads through to `timed-metadata`'s
//! extractor -- the extractor's own decode correctness (Hamming-8/4 +
//! odd-parity + page composition) is `timed-metadata`'s test, not
//! re-verified here.
//!
//! The mutation-bite test below mutates the loaded **byte frames in
//! memory** (never the shared fixture file on disk), the same byte-offset
//! recipe `timed-metadata`'s own #666 fixture test uses.
#![cfg(feature = "teletext")]

use caption_convert::TeletextToWebVtt;
use std::fs;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("teletext")
        .join("teletext_subtitle_synthetic.txt")
}

fn load_frames() -> Vec<(u64, Vec<u8>)> {
    let text =
        fs::read_to_string(fixture_path()).expect("read teletext_subtitle_synthetic.txt fixture");
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

fn convert_frames(frames: &[(u64, Vec<u8>)]) -> TeletextToWebVtt {
    let mut conv = TeletextToWebVtt::new(8, 0x88);
    for (pts, bytes) in frames {
        conv.push_wire_fields(*pts, &[bytes])
            .expect("valid teletext data field wire");
    }
    conv.finalize(31_000);
    conv
}

fn convert() -> TeletextToWebVtt {
    let frames = load_frames();
    assert_eq!(frames.len(), 4, "fixture frame count changed unexpectedly");
    convert_frames(&frames)
}

#[test]
fn webvtt_output_contains_expected_text() {
    let vtt = convert().into_webvtt();
    assert!(vtt.starts_with("WEBVTT\n\n"));
    assert!(vtt.contains("HELLO WORLD"));
    assert!(vtt.contains("THIS IS A TEST"));
    // Both rows visible together at some point (per the fixture's plan).
    assert!(vtt.contains("HELLO WORLD\nTHIS IS A TEST"));
}

#[test]
fn srt_output_contains_expected_text() {
    let srt = convert().into_srt();
    assert!(srt.starts_with("1\n"));
    assert!(srt.contains("HELLO WORLD"));
    assert!(srt.contains("THIS IS A TEST"));
}

/// Bite test (odd parity): corrupt one data bit of the fixture's row-20
/// "HELLO WORLD" packet's 'H' byte (wire offset 4 = header_byte(1) +
/// framing_code(1) + txt_data_block[2], the same offset
/// `timed-metadata`'s own #666 fixture test uses). Odd parity only
/// *detects* errors (ETSI EN 300 706 §8.1), so the corrupted character must
/// decode as the replacement character, not silently as 'H' or anything
/// else -- proving this wrapper's output really reflects a live decode, not
/// a fixed string.
#[test]
fn mutation_bite_parity_corruption_yields_replacement_char() {
    let mut frames = load_frames();
    frames[1].1[4] ^= 0x01;

    let mutated_vtt = convert_frames(&frames).into_webvtt();
    assert!(
        mutated_vtt.contains('\u{FFFD}'),
        "corrupting the parity-protected byte must yield the replacement character:\n{mutated_vtt}"
    );
    assert!(
        !mutated_vtt.contains("HELLO WORLD"),
        "corrupted 'H' must not silently decode as 'H': {mutated_vtt}"
    );

    let original_vtt = convert().into_webvtt();
    assert!(
        original_vtt.contains("HELLO WORLD"),
        "unmutated fixture must still decode to HELLO WORLD"
    );
}
