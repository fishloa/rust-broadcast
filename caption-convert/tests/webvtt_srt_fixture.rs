//! Fixture-driven WebVTT <-> SRT round trip, using the real
//! `fixtures/sub/cap.vtt` fixture (a `transmux`-produced WebVTT sample, two
//! cues).
//!
//! WebVTT <-> SRT is documented (issue #931) as "near-trivial" -- both are
//! plain text-and-timing formats over the same [`caption_convert::Cue`]
//! shape -- but "near-trivial" is exactly the kind of claim a byte-level
//! bug hides in, so this test round-trips a real file and byte-mutates it
//! to prove the parsers actually read the fields they claim to (not raw
//! passthrough).

use std::fs;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("sub")
        .join("cap.vtt")
}

fn load_fixture() -> String {
    fs::read_to_string(fixture_path()).expect("read cap.vtt fixture")
}

#[test]
fn webvtt_to_srt_is_lossless_for_this_fixture() {
    let vtt = load_fixture();
    let (srt, lossy) = caption_convert::webvtt_to_srt(&vtt).expect("valid WebVTT");
    assert!(
        !lossy,
        "cap.vtt is a plain document; must round-trip losslessly"
    );
    assert!(srt.starts_with("1\n"));
    assert!(srt.contains("Hello CMAF"));
    assert!(srt.contains("second cue"));
    assert!(srt.contains("00:00:00,000 --> 00:00:02,000"));
    assert!(srt.contains("00:00:02,000 --> 00:00:04,000"));
}

#[test]
fn srt_back_to_webvtt_preserves_cue_count_and_text() {
    let vtt = load_fixture();
    let (srt, _) = caption_convert::webvtt_to_srt(&vtt).unwrap();
    let back = caption_convert::srt_to_webvtt(&srt).unwrap();
    let original_parsed = caption_convert::parse_webvtt(&vtt).unwrap();
    let round_tripped = caption_convert::parse_webvtt(&back).unwrap();
    assert_eq!(round_tripped.cues, original_parsed.cues);
}

/// Bite test: change the fixture's second cue's start timestamp (in
/// memory) from `00:00:02.000` to `00:00:02.500` and show that exact
/// half-second shift survives all the way through SRT and back -- proving
/// the timestamp fields are actually read and re-emitted, not a fixed
/// template.
#[test]
fn mutation_bite_timestamp_change_survives_the_round_trip() {
    let original = load_fixture();
    assert!(original.contains("00:00:02.000 --> 00:00:04.000"));

    let mutated = original.replacen(
        "00:00:02.000 --> 00:00:04.000",
        "00:00:02.500 --> 00:00:04.500",
        1,
    );
    assert_ne!(
        mutated, original,
        "mutation must actually change the fixture text"
    );

    let (srt, _) = caption_convert::webvtt_to_srt(&mutated).unwrap();
    assert!(
        srt.contains("00:00:02,500 --> 00:00:04,500"),
        "mutated timestamp must survive into SRT:\n{srt}"
    );
    assert!(
        !srt.contains("00:00:02,000 --> 00:00:04,000"),
        "the original timestamp must not still be present:\n{srt}"
    );

    let back = caption_convert::srt_to_webvtt(&srt).unwrap();
    assert!(back.contains("00:00:02.500 --> 00:00:04.500"));

    // The unmutated fixture still round-trips to the original timestamp.
    let (srt_original, _) = caption_convert::webvtt_to_srt(&original).unwrap();
    assert!(srt_original.contains("00:00:02,000 --> 00:00:04,000"));
}

/// Bite test: corrupt the fixture's `WEBVTT` signature and show parsing
/// rejects it with a typed error, not silently producing zero cues.
#[test]
fn mutation_bite_bad_signature_is_rejected_not_silently_empty() {
    let original = load_fixture();
    let mutated = original.replacen("WEBVTT", "WEBVTX", 1);
    assert_ne!(mutated, original);

    let err = caption_convert::parse_webvtt(&mutated).unwrap_err();
    assert!(matches!(err, caption_convert::Error::InvalidWebVtt(_)));

    // The unmutated fixture still parses fine.
    assert!(caption_convert::parse_webvtt(&original).is_ok());
}
