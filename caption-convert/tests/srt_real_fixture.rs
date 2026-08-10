//! Fixture-driven SRT test using a **real** subtitle file: the official
//! English dialogue subtitles for *Sintel* (Blender Foundation's 2010
//! "Durian" open movie), `fixtures/sub/sintel-en.srt` -- see
//! `fixtures/PROVENANCE.md` for the exact source, revision, and CC BY 3.0
//! licence text this was fetched under.
//!
//! Unlike `fixtures/sub/cap.vtt` (a `transmux`-produced synthetic two-cue
//! sample, or `fixtures/cc/cea608_cc1_synthetic.txt` / the teletext synthetic
//! fixture), this is genuine hand-authored, professionally-timed dialogue
//! text: real punctuation (apostrophes, ellipses, a question mark cue), a
//! real multi-line cue, and non-round timestamps (`00:01:47,250`, not a
//! clean `00:00:0X.000`) -- exactly the kind of real-world shape inline
//! happy-path bytes do not exercise.

use std::fs;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("sub")
        .join("sintel-en.srt")
}

fn load_fixture() -> String {
    fs::read_to_string(fixture_path()).expect("read sintel-en.srt fixture")
}

#[test]
fn parses_real_structure_not_just_without_error() {
    let srt = load_fixture();
    let cues = caption_convert::parse_srt(&srt).expect("real Sintel subtitles must parse");

    // 26 numbered cues in the committed file (`grep -c` on the sequence
    // lines) -- a wrong impl that under- or over-splits blocks would drift
    // from this.
    assert_eq!(cues.len(), 26);

    // First cue: real dialogue, real timing (not a clean 0.000 boundary).
    assert_eq!(cues[0].text, "This blade has a dark past.");
    assert_eq!(
        cues[0].start,
        timed_metadata::MediaTime(90_000 * 107 + 90 * 250)
    ); // 00:01:47.250
    assert_eq!(
        cues[0].end,
        timed_metadata::MediaTime(90_000 * 110 + 90 * 500)
    ); // 00:01:50.500

    // Cue 3 is a genuine multi-line payload (the exact case
    // `write_srt`/`write_document` must rejoin with a single `\n`, and that
    // a whitespace-only-line misparse -- issue found by fuzzing -- would
    // corrupt).
    assert_eq!(
        cues[2].text,
        "You're a fool for traveling alone,\nso completely unprepared."
    );

    // The last cue carries a real ellipsis and no trailing newline in the
    // source file at all (the committed fixture's very last byte is `.`).
    assert_eq!(cues[25].text, "Scales...");
}

#[test]
fn round_trips_byte_for_byte_through_cues() {
    let srt = load_fixture();
    let cues = caption_convert::parse_srt(&srt).unwrap();

    let rewritten = caption_convert::write_srt(&cues);
    let reparsed = caption_convert::parse_srt(&rewritten).expect("rewritten SRT must re-parse");
    assert_eq!(
        reparsed, cues,
        "real Sintel subtitles must round-trip through write_srt -> parse_srt unchanged"
    );
}

#[test]
fn converts_losslessly_to_webvtt_and_back() {
    let srt = load_fixture();
    let cues = caption_convert::parse_srt(&srt).unwrap();

    let vtt = caption_convert::srt_to_webvtt(&srt).expect("srt_to_webvtt must accept real SRT");
    assert!(vtt.starts_with("WEBVTT\n\n"));
    assert!(vtt.contains("00:01:47.250 --> 00:01:50.500"));
    assert!(vtt.contains("This blade has a dark past."));

    let (back, lossy) = caption_convert::webvtt_to_srt(&vtt)
        .expect("webvtt_to_srt must accept its own writer's output");
    assert!(
        !lossy,
        "a plain-cue WebVTT document produced from real SRT must convert back losslessly"
    );
    let back_cues = caption_convert::parse_srt(&back).unwrap();
    assert_eq!(
        back_cues, cues,
        "srt -> webvtt -> srt must preserve every cue"
    );
}

/// Bite test: mutate the real fixture's first timestamp in memory and show
/// the exact shift survives the SRT -> WebVTT conversion -- proving the
/// timing fields are actually read and re-emitted, not templated.
#[test]
fn mutation_bite_timestamp_change_survives_srt_to_webvtt() {
    let original = load_fixture();
    assert!(original.contains("00:01:47,250 --> 00:01:50,500"));

    let mutated = original.replacen(
        "00:01:47,250 --> 00:01:50,500",
        "00:01:48,250 --> 00:01:51,500",
        1,
    );
    assert_ne!(mutated, original);

    let vtt = caption_convert::srt_to_webvtt(&mutated).unwrap();
    assert!(vtt.contains("00:01:48.250 --> 00:01:51.500"));
    assert!(!vtt.contains("00:01:47.250 --> 00:01:50.500"));

    // The unmutated fixture still converts to the original timing.
    let vtt_original = caption_convert::srt_to_webvtt(&original).unwrap();
    assert!(vtt_original.contains("00:01:47.250 --> 00:01:50.500"));
}
