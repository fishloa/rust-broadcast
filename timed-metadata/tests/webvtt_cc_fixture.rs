//! Fixture-driven CEA-608 -> WebVTT test (issue #568).
//!
//! Fixture: `fixtures/cc/cea608_cc1_synthetic.txt` — a synthetic-but-spec-real
//! CEA-608 (line-21) CC1 `cc_data()` byte stream (CTA-608-E control/PAC/char
//! codes; Table B.9 wire framing), generated once and committed (see the
//! fixture file's own header comment for the exact caption plan and the
//! generation recipe). It is NOT hand-typed inline bytes in this test file:
//! it is an independent artifact this test reads from disk and decodes, per
//! the project's fixture discipline.
//!
//! A **real** capture with embedded EIA-608 (ffmpeg-bugs sample
//! `transformers_EIA608_H264.ts` from `samples.ffmpeg.org`, ticket #2885) was
//! located and used to informally validate this module's design during
//! development (`ffmpeg -f lavfi -i "movie=<file>[out+subcc]" -c:s srt`
//! extracted real pop-on cues with clean start/end timing that match the
//! shape this extractor produces). That capture is licensed third-party
//! movie footage (~130 MB) and is **not** committed here — per this
//! project's fixture-licensing convention it would belong in the
//! `.test-streams/`-gated pattern, not the public tree — so the committed,
//! CI-run fixture is the synthetic one below instead (STEP 1 fallback
//! option 3, explicitly sanctioned for exactly this situation).
#![cfg(feature = "cc-data")]

use broadcast_common::Parse;
use cc_data::CcData;
use cc_data::decode::Cea608Channel;
use std::fs;
use std::path::Path;
use timed_metadata::event::MediaTime;
use timed_metadata::webvtt::{Cea608CueExtractor, Cue, write_document, write_segment};

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("cc")
        .join("cea608_cc1_synthetic.txt")
}

/// Parse the fixture into `(pts_90k, cc_data_bytes)` frames, skipping
/// comment/blank lines.
fn load_frames() -> Vec<(u64, Vec<u8>)> {
    let text = fs::read_to_string(fixture_path()).expect("read cea608_cc1_synthetic.txt fixture");
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
    assert_eq!(frames.len(), 13, "fixture frame count changed unexpectedly");

    let mut ex = Cea608CueExtractor::new(Cea608Channel::Cc1);
    for (pts, bytes) in &frames {
        let cc = CcData::parse(bytes).expect("valid cc_data() Table B.9 bytes");
        ex.push_frame(*pts, &cc.triplets);
    }
    // Nothing is open past the final EDM (pts 42000), but finalize
    // defensively — it must be a no-op here.
    ex.finalize(45_000);
    ex.into_cues()
}

/// Decode -> cue: exact expected text + start/end PTS, derived from the
/// fixture's own documented caption plan (pop-on "HELLO", roll-up
/// "HI"->"HI\nBYE"->"BYE", paint-on "OK"). Timing matches the fixture's
/// commit/erase frame PTS exactly (this is a synthetic fixture with frame
/// granularity == PTS granularity, so "within one frame duration" is exact
/// equality here).
#[test]
fn decode_to_expected_cues() {
    let cues = extract_cues();
    let expected = [
        (6_000u64, 9_000u64, "HELLO"), // pop-on: commit@EOC, erase@EDM
        (15_000, 21_000, "HI"),        // roll-up row 1 alone
        (21_000, 24_000, "HI\nBYE"),   // roll-up rows 1+2 both visible
        (24_000, 33_000, "BYE"),       // roll-up row 1 scrolled off
        (39_000, 42_000, "OK"),        // paint-on: painted, then erased
    ];
    assert_eq!(cues.len(), expected.len(), "cues: {cues:?}");
    for (cue, (start, end, text)) in cues.iter().zip(expected.iter()) {
        assert_eq!(cue.start, MediaTime(*start), "cue {text:?} start");
        assert_eq!(cue.end, MediaTime(*end), "cue {text:?} end");
        assert_eq!(cue.text, *text);
    }
}

/// Standalone WebVTT well-formedness check (no external tool dependency):
/// signature, blank-line-separated cue blocks (structurally enforced by
/// splitting the whole document on `"\n\n"` — a stray blank line *inside* a
/// cue's payload would split it into a fragment that fails the "starts with
/// a timings line" check below), `(hh:)?mm:ss.ttt --> ...` timestamp
/// grammar, and non-decreasing start / `end > start` ordering (W3C WebVTT
/// §4).
fn assert_valid_webvtt(doc: &str) {
    let blocks: Vec<&str> = doc.split("\n\n").collect();
    let header_block = blocks.first().expect("at least one block");
    assert_eq!(
        header_block.lines().next(),
        Some("WEBVTT"),
        "must start with the signature"
    );
    assert_eq!(
        blocks.last(),
        Some(&""),
        "document must end with a blank-line-terminated block"
    );
    assert!(blocks.len() > 2, "expected at least one cue block: {doc:?}");

    let mut last_start_ms: i64 = -1;
    for block in &blocks[1..blocks.len() - 1] {
        let mut cue_lines = block.lines();
        let timings = cue_lines
            .next()
            .unwrap_or_else(|| panic!("empty cue block (stray blank line?): {doc:?}"));
        let (start, end) = timings
            .split_once(" --> ")
            .unwrap_or_else(|| panic!("cue block must open with a timings line: {timings:?}"));
        // A settings tail (e.g. "align:start") is space-separated after the
        // end timestamp; strip it for the grammar check.
        let end = end.split_whitespace().next().unwrap_or(end);
        let start_ms = assert_valid_timestamp(start);
        let end_ms = assert_valid_timestamp(end);
        assert!(end_ms > start_ms, "cue end must be > start: {timings}");
        assert!(
            start_ms >= last_start_ms,
            "cues must be in non-decreasing start order: {timings}"
        );
        last_start_ms = start_ms;
        assert!(
            cue_lines.next().is_some(),
            "cue block must have a non-empty payload: {block:?}"
        );
    }
}

/// Validate `(hh:)?mm:ss.ttt` and return the value in milliseconds.
fn assert_valid_timestamp(ts: &str) -> i64 {
    let (rest, ms_str) = ts.split_once('.').expect("timestamp must have .ttt");
    assert_eq!(
        ms_str.len(),
        3,
        "milliseconds must be exactly 3 digits: {ts}"
    );
    let ms: i64 = ms_str.parse().expect("milliseconds must be numeric");
    let fields: Vec<&str> = rest.split(':').collect();
    let (h, m, s) = match fields.as_slice() {
        [m, s] => (0i64, *m, *s),
        [h, m, s] => (h.parse().expect("hours numeric"), *m, *s),
        _ => panic!("timestamp must be mm:ss or hh:mm:ss: {ts}"),
    };
    assert_eq!(m.len(), 2, "minutes must be exactly 2 digits: {ts}");
    assert_eq!(s.len(), 2, "seconds must be exactly 2 digits: {ts}");
    let m: i64 = m.parse().expect("minutes numeric");
    let s: i64 = s.parse().expect("seconds numeric");
    assert!(m < 60, "minutes must be 00-59: {ts}");
    assert!(s < 60, "seconds must be 00-59: {ts}");
    ((h * 60 + m) * 60 + s) * 1000 + ms
}

#[test]
fn webvtt_output_is_well_formed() {
    let cues = extract_cues();
    let doc = write_document(&cues);
    assert_valid_webvtt(&doc);
    // Spot-check the escaped/plain payload made it through.
    assert!(doc.contains("HELLO"));
    assert!(doc.contains("HI\nBYE"));
}

/// If `ffmpeg` is on PATH, additionally cross-validate with a real external
/// tool (bonus over the standalone check above; skips cleanly if absent so
/// this never gates CI on tool availability).
#[test]
fn webvtt_output_validates_with_ffmpeg_if_available() {
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("webvtt_output_validates_with_ffmpeg_if_available: SKIPPED — ffmpeg not on PATH");
        return;
    }
    let cues = extract_cues();
    let doc = write_document(&cues);
    let dir = std::env::temp_dir();
    let vtt_path = dir.join("timed_metadata_568_fixture.vtt");
    let srt_path = dir.join("timed_metadata_568_fixture.srt");
    fs::write(&vtt_path, &doc).expect("write temp vtt");
    let _ = fs::remove_file(&srt_path);
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(&vtt_path)
        .arg(&srt_path)
        .status()
        .expect("run ffmpeg");
    assert!(
        status.success(),
        "ffmpeg rejected the emitted WebVTT as invalid"
    );
    let srt = fs::read_to_string(&srt_path).expect("ffmpeg produced an srt");
    assert!(srt.contains("HELLO"));
    let _ = fs::remove_file(&vtt_path);
    let _ = fs::remove_file(&srt_path);
}

/// `X-TIMESTAMP-MAP`: the segment's MPEGTS value equals the segment's first
/// PES PTS, and cue times are correct relative to `LOCAL:00:00:00.000`.
#[test]
fn x_timestamp_map_on_segment_boundary() {
    let cues = extract_cues();
    // Split at the roll-up/paint-on boundary: segment 2 starts at the first
    // roll-up cue's PES PTS (15_000 ticks), matching a segmenter that cut a
    // media segment there.
    let segment_start = MediaTime(15_000);
    let seg2: Vec<Cue> = cues
        .iter()
        .filter(|c| c.start.0 >= segment_start.0)
        .cloned()
        .collect();
    assert_eq!(seg2.len(), 4);

    let doc = write_segment(&seg2, segment_start);
    let mut lines = doc.lines();
    assert_eq!(lines.next(), Some("WEBVTT"));
    let header = lines.next().expect("timestamp-map header");
    assert_eq!(header, "X-TIMESTAMP-MAP=MPEGTS:15000,LOCAL:00:00:00.000");
    assert_eq!(lines.next(), Some(""));

    // First cue in the segment ("HI") started exactly at the segment start,
    // so its LOCAL time is 00:00:00.000.
    let timings = lines.next().expect("first cue timings line");
    assert!(timings.starts_with("00:00:00.000 --> "));

    assert_valid_webvtt(&doc);
}

/// A cue that starts exactly at the 2^33 PTS wrap boundary maps to
/// `MPEGTS:0`, not a huge 34-bit value — confirms the modulo, not just a
/// pass-through, per RFC 8216 §3.5 (`MPEGTS:<n>` is a 33-bit value).
#[test]
fn x_timestamp_map_wraps_at_33_bits() {
    use timed_metadata::timeline::PTS_WRAP;
    let doc = write_segment(&[], MediaTime(PTS_WRAP));
    assert!(doc.contains("X-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000"));
}
