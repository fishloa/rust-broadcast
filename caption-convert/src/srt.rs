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
use crate::time::{normalize_comma_separator, parse_timestamp, to_hms_ms};
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
/// and both `\n` and `\r\n` line endings.
///
/// # Errors
///
/// [`Error::InvalidSrt`] if a block has no timing line, or
/// [`Error::InvalidTimestamp`] if a timestamp does not match
/// `(hh:)?mm:ss,ttt`.
pub fn parse_srt(input: &str) -> Result<Vec<Cue>, Error> {
    let normalized = input.replace("\r\n", "\n");
    let mut cues = Vec::new();

    for block in normalized.split("\n\n") {
        let block = block.trim();
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
