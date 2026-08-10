//! SubRip Text (SRT) parsing and writing.
//!
//! **SRT has no formal specification.** There is no standards body
//! document to cite. This module follows the de facto format produced and
//! consumed by ffmpeg, VLC, and every mainstream subtitle editor: a
//! sequential 1-based block index, a `hh:mm:ss,ttt --> hh:mm:ss,ttt` timing
//! line (comma milliseconds separator -- the one consistent difference from
//! WebVTT's `.`), one or more plain-text payload lines, and a blank line
//! between blocks.
//!
//! SRT <-> WebVTT is documented as **near-trivial** (issue #931): both are
//! plain text-and-timing formats over the same [`Cue`] shape. SRT -> WebVTT
//! is lossless (SRT has no construct WebVTT cannot represent). WebVTT -> SRT
//! is lossless *unless* the source used a construct SRT cannot represent
//! (cue identifiers, cue settings, `NOTE`/`STYLE`/`REGION` blocks) -- see
//! [`crate::webvtt::parse_webvtt`]'s `lossy` flag, which
//! [`crate::webvtt_to_srt`] forwards.

use crate::error::Error;
use crate::time::{normalize_comma_separator, normalize_line_endings, parse_timestamp, to_hms_ms};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;
use timed_metadata::MediaTime;
use timed_metadata::webvtt::Cue;

/// Render 90 kHz ticks as an SRT timestamp `hh:mm:ss,ttt`.
#[must_use]
pub fn format_srt_timestamp(t: MediaTime) -> String {
    let (h, m, s, ms) = to_hms_ms(t.0);
    let mut out = String::with_capacity(12);
    // `write!` to a `String` cannot fail (no I/O), matching the panic-free
    // guarantee `timed_metadata::webvtt::format_timestamp` gives via `format!`.
    let _ = write!(out, "{h:02}:{m:02}:{s:02},{ms:03}");
    out
}

/// Render cues as an SRT document: `<index>\n<timings>\n<payload>\n\n` per
/// cue, 1-based sequential index.
#[must_use]
pub fn write_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        let _ = write!(
            out,
            "{}\n{} --> {}\n",
            i + 1,
            format_srt_timestamp(cue.start),
            format_srt_timestamp(cue.end)
        );
        for line in cue.text.lines() {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Parse an SRT document into [`Cue`]s.
///
/// Tolerates a missing leading sequence-number line (some encoders omit it)
/// and any of `\n`, `\r\n`, or a lone `\r` as a line ending -- SRT has no
/// formal spec, but real encoders/players are at least as permissive about
/// line endings as W3C WebVTT's own three-terminator-form definition (the
/// same private `normalize_line_endings` helper is shared with
/// [`crate::webvtt::parse_webvtt`]).
///
/// # Errors
///
/// [`Error::InvalidSrt`] if a block has no timing line, or
/// [`Error::InvalidTimestamp`] if a timestamp does not match
/// `(hh:)?mm:ss,ttt`.
pub fn parse_srt(input: &str) -> Result<Vec<Cue>, Error> {
    let normalized = normalize_line_endings(input);
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        // Trim only the *blank-line* padding a run of more than two
        // consecutive newlines leaves at a block's edges -- NOT
        // `str::trim()`'s generic whitespace trim, which also eats a
        // meaningful leading/trailing space on the block's first/last
        // *content* line (found by fuzzing: a payload's final line ending
        // in a real trailing space silently lost that space on parse,
        // since it sits right at the boundary `"\n\n"` split on).
        let block = block.trim_matches('\n');
        if block.is_empty() {
            continue;
        }
        let mut lines = block.lines();
        let first = lines
            .next()
            .ok_or_else(|| Error::InvalidSrt("empty block".to_string()))?;

        let timing_line = if first.contains("-->") {
            first
        } else {
            // A sequence-number line precedes the timing line; validate but
            // discard the number (SRT requires it be sequential, but this
            // parser does not require re-numbering to be gapless on input).
            if first.trim().parse::<u64>().is_err() {
                return Err(Error::InvalidSrt(format!(
                    "expected a sequence number, got {first:?}"
                )));
            }
            lines.next().ok_or_else(|| {
                Error::InvalidSrt("sequence number with no timing line".to_string())
            })?
        };

        let (start_str, after_arrow) = timing_line.split_once("-->").ok_or_else(|| {
            Error::InvalidSrt(format!("no '-->' in timing line: {timing_line:?}"))
        })?;
        // A styling tail (rare, some encoders emit "X1:.. Y1:..") is
        // discarded the same way the WebVTT parser discards cue settings.
        let end_str = after_arrow.split_whitespace().next().unwrap_or("");

        let start = parse_timestamp(&normalize_comma_separator(start_str.trim()))?;
        let end = parse_timestamp(&normalize_comma_separator(end_str.trim()))?;

        let text = lines.collect::<Vec<_>>().join("\n");

        cues.push(Cue {
            start: MediaTime(start),
            end: MediaTime(end),
            text,
        });
    }

    Ok(cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(start: u64, end: u64, text: &str) -> Cue {
        Cue {
            start: MediaTime(start),
            end: MediaTime(end),
            text: text.to_string(),
        }
    }

    #[test]
    fn timestamp_uses_comma() {
        assert_eq!(format_srt_timestamp(MediaTime(90_000)), "00:00:01,000");
    }

    #[test]
    fn write_then_parse_round_trips() {
        let cues = alloc::vec![cue(0, 90_000, "a"), cue(180_000, 270_000, "b\nc")];
        let doc = write_srt(&cues);
        let parsed = parse_srt(&doc).unwrap();
        assert_eq!(parsed, cues);
    }

    #[test]
    fn write_srt_sequence_numbers_are_1_based() {
        let cues = alloc::vec![cue(0, 1000, "a"), cue(1000, 2000, "b")];
        let doc = write_srt(&cues);
        assert!(doc.starts_with("1\n"));
        assert!(doc.contains("\n2\n"));
    }

    #[test]
    fn parse_tolerates_missing_sequence_number() {
        let doc = "00:00:00,000 --> 00:00:01,000\nhi\n";
        let cues = parse_srt(doc).unwrap();
        assert_eq!(cues, alloc::vec![cue(0, 90_000, "hi")]);
    }

    #[test]
    fn parse_tolerates_crlf() {
        let doc = "1\r\n00:00:00,000 --> 00:00:01,000\r\nhi\r\n";
        let cues = parse_srt(doc).unwrap();
        assert_eq!(cues, alloc::vec![cue(0, 90_000, "hi")]);
    }

    #[test]
    fn parse_tolerates_lone_cr_line_ending() {
        // W3C WebVTT SS4's third line-terminator form (a bare CR, not
        // followed by LF) -- SRT has no formal spec, but this parser is at
        // least as permissive as WebVTT's own definition (issue found by
        // fuzzing `caption-convert`'s webvtt/srt round-trip).
        let doc = "1\r00:00:00,000 --> 00:00:01,000\rhi\r";
        let cues = parse_srt(doc).unwrap();
        assert_eq!(cues, alloc::vec![cue(0, 90_000, "hi")]);
    }

    #[test]
    fn parse_preserves_trailing_space_on_final_block_line() {
        // Found by fuzzing: `block.trim()` (a generic whitespace trim) used
        // to eat a meaningful trailing space on a block's last content line
        // whenever that line sat right at the `"\n\n"` block-delimiter
        // boundary, not just the intentional blank-line padding around it.
        let doc = "1\n00:00:00,000 --> 00:00:01,000\nhi there \n";
        let cues = parse_srt(doc).unwrap();
        assert_eq!(cues, alloc::vec![cue(0, 90_000, "hi there ")]);
    }

    #[test]
    fn parse_rejects_missing_arrow() {
        let doc = "1\n00:00:00,000 00:00:01,000\nhi\n";
        assert!(parse_srt(doc).is_err());
    }

    #[test]
    fn parse_rejects_bad_sequence_number() {
        let doc = "not-a-number\n00:00:00,000 --> 00:00:01,000\nhi\n";
        assert!(parse_srt(doc).is_err());
    }
}
