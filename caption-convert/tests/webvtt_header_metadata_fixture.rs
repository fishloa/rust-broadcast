//! Independent-oracle test for `parse_webvtt`'s header handling (issue
//! #1109, reopens #974 — audit finding CV-W1).
//!
//! `fixtures/sub/header_meta.vtt` (see that directory's README for
//! provenance) carries a `WEBVTT` header followed by `Kind:`/`Language:`
//! metadata text lines before the first blank line — W3C WebVTT SS4.1's own
//! header-text convention, distinct from the `X-TIMESTAMP-MAP` header this
//! crate already handled. Before the fix, any header line other than
//! `X-TIMESTAMP-MAP` fell through into the cue-block grouper, was misread
//! as a cue identifier with no timing line after it, and failed the whole
//! document.
//!
//! Oracle: ffmpeg 8.1.2 (independent of our parser) accepts the same file
//! as valid WebVTT and converts it to the same two cues via `-f srt`.

use std::fs;
use std::path::Path;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("sub")
        .join("header_meta.vtt")
}

#[test]
fn header_metadata_lines_do_not_fail_the_document() {
    let vtt = fs::read_to_string(fixture_path()).expect("read header_meta.vtt fixture");

    // ffmpeg 8.1.2 oracle (`ffmpeg -v error -i header_meta.vtt -f srt -`):
    //   1
    //   00:00:01,000 --> 00:00:03,000
    //   Hello header test
    //
    //   2
    //   00:00:04,000 --> 00:00:06,000
    //   Second cue
    let parsed = caption_convert::parse_webvtt(&vtt)
        .expect("header text before the first blank line must not fail the document");

    assert_eq!(
        parsed.cues.len(),
        2,
        "both cues after the header must survive"
    );
    assert_eq!(parsed.cues[0].text, "Hello header test");
    assert_eq!(parsed.cues[1].text, "Second cue");
    assert_eq!(
        parsed.cues[0].start,
        timed_metadata::MediaTime(90_000),
        "00:00:01.000 at the crate's 90kHz MediaTime tick rate"
    );
    assert_eq!(parsed.cues[0].end, timed_metadata::MediaTime(270_000));
    assert_eq!(parsed.cues[1].start, timed_metadata::MediaTime(360_000));
    assert_eq!(parsed.cues[1].end, timed_metadata::MediaTime(540_000));

    // The header text itself is unrepresentable in this crate's Cue model
    // (and in SRT), so the document is correctly flagged lossy rather than
    // silently dropping it.
    assert!(
        parsed.lossy,
        "dropping the Kind:/Language: header text must be flagged lossy"
    );
}

#[test]
fn header_metadata_document_converts_to_srt_matching_ffmpeg_oracle() {
    let vtt = fs::read_to_string(fixture_path()).expect("read header_meta.vtt fixture");
    let (srt, lossy) =
        caption_convert::webvtt_to_srt(&vtt).expect("header text must not fail conversion");
    assert!(lossy, "header text loss must be reported");

    let expected = "1\n00:00:01,000 --> 00:00:03,000\nHello header test\n\n\
                    2\n00:00:04,000 --> 00:00:06,000\nSecond cue\n\n";
    assert_eq!(
        srt, expected,
        "our SRT output must match ffmpeg's own `-f srt` conversion of the same file"
    );
}
