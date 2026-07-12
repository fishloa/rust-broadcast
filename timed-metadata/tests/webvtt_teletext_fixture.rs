//! Fixture-driven EBU Teletext -> WebVTT test (issue #666).
//!
//! Fixture: `fixtures/teletext/teletext_subtitle_synthetic.txt` — a
//! synthetic-but-spec-real EN 300 706 Teletext subtitle page (magazine 8,
//! page 0x88, header + two display rows), generated once and committed (see
//! the fixture file's own header comment for the exact page plan, the
//! generation recipe, and why no real capture or spec worked example was
//! available — this project's established fixture-honesty fallback). It is
//! NOT hand-typed inline bytes in this test file: it is an independent
//! artifact this test reads from disk, parses through `dvb-vbi`'s real
//! `TeletextDataField::parse`, and decodes through
//! `timed_metadata::webvtt::TeletextCueExtractor` — proving the full
//! Hamming-8/4 + odd-parity + page-composition pipeline end to end.
#![cfg(feature = "teletext")]

use dvb_vbi::TeletextDataField;
use std::fs;
use std::path::Path;
use timed_metadata::event::MediaTime;
use timed_metadata::webvtt::{Cue, TeletextCueExtractor, write_document};

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("teletext")
        .join("teletext_subtitle_synthetic.txt")
}

/// Parse the fixture into `(pts_90k, 44-byte TeletextDataField wire)` frames,
/// skipping comment/blank lines.
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

fn extract_cues() -> Vec<Cue> {
    let frames = load_frames();
    assert_eq!(frames.len(), 4, "fixture frame count changed unexpectedly");

    let mut ex = TeletextCueExtractor::new(8, 0x88);
    for (pts, bytes) in &frames {
        let field = TeletextDataField::parse(bytes).expect("valid TeletextDataField wire bytes");
        ex.push_frame(*pts, std::slice::from_ref(&field));
    }
    ex.finalize(33_000);
    ex.into_cues()
}

/// Decode -> cues: exact expected text + start/end PTS, derived from the
/// fixture's own documented page plan (header@0 establishes an empty erased
/// page, row 20 "HELLO WORLD"@3000, row 22 "THIS IS A TEST"@6000 completes
/// the 2-row subtitle, a second erase header@30000 clears it).
///
/// Two cues, not one: this extractor's diff-based boundary detection (see
/// `crate::webvtt` module docs) fires on *every* displayed-text change, so
/// row 20 landing alone at pts=3000 is itself a boundary (mirrors the CEA
/// roll-up extractors' documented "progressive reveal" granularity — a
/// deliberate, documented simplification, not a bug).
#[test]
fn decode_to_expected_cues() {
    let cues = extract_cues();
    let expected = [
        (3_000u64, 6_000u64, "HELLO WORLD"),
        (6_000, 30_000, "HELLO WORLD\nTHIS IS A TEST"),
    ];
    assert_eq!(cues.len(), expected.len(), "cues: {cues:?}");
    for (cue, (start, end, text)) in cues.iter().zip(expected.iter()) {
        assert_eq!(cue.start, MediaTime(*start), "cue {text:?} start");
        assert_eq!(cue.end, MediaTime(*end), "cue {text:?} end");
        assert_eq!(cue.text, *text);
    }
}

/// Standalone WebVTT well-formedness spot-check (full grammar validation
/// lives in `webvtt_cc_fixture.rs`/`writer.rs`'s own tests; this just proves
/// the Teletext-derived cue also serializes through the same writer).
#[test]
fn webvtt_output_contains_expected_text() {
    let cues = extract_cues();
    let doc = write_document(&cues);
    assert!(doc.starts_with("WEBVTT\n\n"));
    assert!(doc.contains("HELLO WORLD"));
    assert!(doc.contains("THIS IS A TEST"));
    assert!(doc.contains("00:00:00.066 --> 00:00:00.333"));
}

/// Mutation bite (odd parity): corrupt one data bit of the fixture's row-20
/// "HELLO WORLD" packet (byte offset 4 of the wire = the 3rd
/// `txt_data_block` byte = the 'H' character's parity-protected byte).
/// Odd parity only *detects* errors (ETSI EN 300 706 §8.1), so the corrupted
/// character must decode as the replacement character, not silently as some
/// other letter or the original 'H' — proving the FEC decode is real, not a
/// passthrough.
#[test]
fn mutation_bite_parity_corruption_yields_replacement_char() {
    let mut frames = load_frames();
    // frames[1] is the row-20 "HELLO WORLD" packet; wire offset 4 = the
    // header_byte(1) + framing_code(1) + txt_data_block[2] (first text byte).
    frames[1].1[4] ^= 0x01;

    let mut ex = TeletextCueExtractor::new(8, 0x88);
    for (pts, bytes) in &frames {
        let field = TeletextDataField::parse(bytes).expect("valid TeletextDataField wire bytes");
        ex.push_frame(*pts, std::slice::from_ref(&field));
    }
    ex.finalize(33_000);
    let cues = ex.into_cues();
    assert_eq!(cues.len(), 2, "cues: {cues:?}");
    assert!(
        cues[0].text.starts_with("\u{FFFD}ELLO WORLD"),
        "corrupted parity byte must decode as the replacement character, not 'H' or anything else: {:?}",
        cues[0].text
    );
    assert!(
        cues[1]
            .text
            .starts_with("\u{FFFD}ELLO WORLD\nTHIS IS A TEST")
    );
}

/// Mutation bite (Hamming-8/4): corrupt a single bit of the fixture's first
/// header packet's page-units byte (wire offset 4 = `txt_data_block[2]`).
/// Hamming-8/4 *corrects* single-bit errors (§8.2), so the page number must
/// still be recovered correctly and the cue must be identical to the
/// uncorrupted decode — proving the correction actually runs (not merely
/// "happens not to fail" — a byte that were passed through raw would decode
/// the wrong page number and the assembler would never activate).
#[test]
fn mutation_bite_hamming_single_bit_error_is_corrected() {
    let mut frames = load_frames();
    frames[0].1[4] ^= 0x02; // flip one bit of the page-units Hamming byte

    let mut ex = TeletextCueExtractor::new(8, 0x88);
    for (pts, bytes) in &frames {
        let field = TeletextDataField::parse(bytes).expect("valid TeletextDataField wire bytes");
        ex.push_frame(*pts, std::slice::from_ref(&field));
    }
    ex.finalize(33_000);
    let cues = ex.into_cues();
    assert_eq!(
        cues.len(),
        2,
        "a single-bit Hamming error in the page header must still be corrected \
         and the page recognised, producing the same cues as the uncorrupted decode: {cues:?}"
    );
    assert_eq!(cues[0].text, "HELLO WORLD");
    assert_eq!(cues[1].text, "HELLO WORLD\nTHIS IS A TEST");
}
