//! WebVTT parsing, plus re-exports of `timed-metadata`'s WebVTT writer.
//!
//! Cite: W3C WebVTT: The Web Video Text Tracks Format
//! (<https://www.w3.org/TR/webvtt1/>) SS4 (file structure) / SS4.3.1 (cue
//! timings) / SS6.4 (payload escaping).
//!
//! Writing (`WEBVTT` signature, cue blocks, `X-TIMESTAMP-MAP` HLS segments)
//! is `timed-metadata`'s [`timed_metadata::webvtt`] module (issue #568/#666)
//! -- re-exported here rather than duplicated. **Parsing** a WebVTT document
//! back into [`Cue`]s is new work this crate adds (needed for the
//! WebVTT -> SRT direction): [`parse_webvtt`].
//!
//! The parser is intentionally a **subset** reader: it extracts cue timing
//! and plain payload text, which is everything a lossless-for-SRT-purposes
//! reader needs. It recognises but drops (setting
//! [`ParsedWebVtt::lossy`]) constructs SRT cannot represent:
//!
//! - `NOTE` / `STYLE` / `REGION` blocks (SS4-SS6): dropped entirely.
//! - A cue identifier line (the optional line before the timing line):
//!   dropped -- SRT's leading integer is a strict *sequence* number, not
//!   carried from an arbitrary WebVTT identifier.
//! - Cue settings (`line:`/`position:`/`align:`/... after the end timestamp
//!   on the timing line): dropped.
//! - Inline markup (`<i>`/`<b>`/`<c...>`/timestamp tags/...) in the payload:
//!   **not** dropped -- passed through verbatim, since many real SRT
//!   consumers tolerate basic tags -- but flagged `lossy` because it is not
//!   part of the (nonexistent) SRT specification.

use crate::error::Error;
use crate::time::{normalize_line_endings, parse_timestamp};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use timed_metadata::webvtt::Cue;

pub use timed_metadata::webvtt::{
    cue_block, escape_payload, format_timestamp, write_document, write_segment,
};

/// The result of [`parse_webvtt`]: the extracted cues, plus whether the
/// source document used any construct this crate's model (and SRT) cannot
/// represent.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ParsedWebVtt {
    /// The extracted cues, in document order.
    pub cues: Vec<Cue>,
    /// `true` if the source document contained a `NOTE`/`STYLE`/`REGION`
    /// block, a cue identifier, cue settings, or inline markup -- anything
    /// this crate's plain `Cue` model (and SRT) cannot carry.
    pub lossy: bool,
}

/// Unescape the three entities [`escape_payload`] emits (`&amp;` `&lt;`
/// `&gt;`), left-to-right, single pass -- the exact inverse of
/// `escape_payload`. Other WebVTT-defined entities (`&lrm;` `&rlm;` `&nbsp;`)
/// are out of scope and pass through literally (a documented, not silent,
/// limitation -- they are rare in real captions and this crate emits none of
/// them).
fn unescape_payload(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let rest: String = chars.clone().take(4).collect();
        if rest.starts_with("amp;") {
            out.push('&');
            for _ in 0..4 {
                chars.next();
            }
        } else if rest.starts_with("lt;") {
            out.push('<');
            for _ in 0..3 {
                chars.next();
            }
        } else if rest.starts_with("gt;") {
            out.push('>');
            for _ in 0..3 {
                chars.next();
            }
        } else {
            out.push('&');
        }
    }
    out
}

/// Parse a WebVTT document (W3C WebVTT SS4) into [`ParsedWebVtt`].
///
/// # Errors
///
/// [`Error::InvalidWebVtt`] if the document does not open with the `WEBVTT`
/// signature, or a cue block's timing line is malformed;
/// [`Error::InvalidTimestamp`] if a timestamp does not match
/// `(hh:)?mm:ss.ttt`.
pub fn parse_webvtt(input: &str) -> Result<ParsedWebVtt, Error> {
    let input = input.strip_prefix('\u{FEFF}').unwrap_or(input); // optional BOM
    // Normalise to LF-only *before* splitting into lines: W3C WebVTT SS4
    // defines a line terminator as LF, lone CR, or CRLF (not just the LF/
    // CRLF pair `str::lines()` recognises) -- see `normalize_line_endings`'s
    // docs for why skipping this step lets a stray `\r` end up embedded in a
    // cue's text and then silently vanish on the next write.
    let normalized = normalize_line_endings(input);
    let mut lines = normalized.lines();
    let header = lines.next().ok_or(Error::EmptyInput)?;
    if !(header == "WEBVTT" || header.starts_with("WEBVTT ") || header.starts_with("WEBVTT\t")) {
        return Err(Error::InvalidWebVtt(
            "missing WEBVTT signature on line 1".to_string(),
        ));
    }

    // Group the remaining lines into blank-line-delimited blocks. This
    // walks `lines()` directly (rather than rejoining collected lines and
    // re-splitting on `"\n\n"`) so a block's position -- first, last, or
    // preceded/followed by any number of blank lines -- never changes how
    // it is delimited.
    let mut cues = Vec::new();
    let mut lossy = false;
    let mut current: Vec<&str> = Vec::new();

    let mut flush = |current: &mut Vec<&str>| -> Result<(), Error> {
        if current.is_empty() {
            return Ok(());
        }
        if let Some((cue, block_lossy)) = parse_block(current)? {
            cues.push(cue);
            lossy |= block_lossy;
        } else {
            lossy = true; // a dropped NOTE/STYLE/REGION block
        }
        current.clear();
        Ok(())
    };

    for line in lines {
        // W3C WebVTT SS4's block-boundary rule is "the line is the empty
        // string" -- a zero-length line -- NOT "the line is blank/
        // whitespace-only". A payload can legitimately have a
        // whitespace-only *interior* line (e.g. a single-space spacer line);
        // treating it as a block delimiter (via `.trim().is_empty()`) used
        // to silently truncate the cue right there, losing every line after
        // it (found by fuzzing the webvtt<->srt round-trip).
        if line.is_empty() {
            flush(&mut current)?;
        } else {
            current.push(line);
        }
    }
    flush(&mut current)?;

    Ok(ParsedWebVtt { cues, lossy })
}

/// Parse one blank-line-delimited block's lines. Returns `Ok(None)` for a
/// block this crate deliberately drops (`NOTE`/`STYLE`/`REGION`); otherwise
/// `Ok(Some((cue, block_was_lossy)))`.
fn parse_block(lines: &[&str]) -> Result<Option<(Cue, bool)>, Error> {
    let mut lossy = false;
    let first = lines[0];

    if first.starts_with("NOTE")
        || first == "STYLE"
        || first.starts_with("STYLE ")
        || first == "REGION"
        || first.starts_with("REGION ")
    {
        return Ok(None);
    }

    let (idx, timing_line) = if first.contains("-->") {
        (1, first)
    } else {
        // An identifier line precedes the timing line.
        lossy = true;
        let t = *lines.get(1).ok_or_else(|| {
            Error::InvalidWebVtt("cue identifier with no timing line".to_string())
        })?;
        (2, t)
    };

    let (start_str, after_arrow) = timing_line
        .split_once("-->")
        .ok_or_else(|| Error::InvalidWebVtt(format_timing(timing_line)))?;
    let after_arrow = after_arrow.trim_start();
    let (end_str, settings) = match after_arrow.split_once(char::is_whitespace) {
        Some((e, s)) if !s.trim().is_empty() => (e, Some(s)),
        _ => (after_arrow.trim_end(), None),
    };
    if settings.is_some() {
        lossy = true;
    }

    let start = parse_timestamp(start_str.trim())?;
    let end = parse_timestamp(end_str.trim())?;

    let text_lines = &lines[idx..];
    if text_lines.iter().any(|l| l.contains('<')) {
        lossy = true;
    }
    let text = text_lines
        .iter()
        .map(|l| unescape_payload(l))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(Some((
        Cue {
            start: timed_metadata::MediaTime(start),
            end: timed_metadata::MediaTime(end),
            text,
        },
        lossy,
    )))
}

fn format_timing(timing_line: &str) -> String {
    alloc::format!("cue block has no '-->' in its timing line: {timing_line:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_signature() {
        let err = parse_webvtt("NOT WEBVTT\n\n").unwrap_err();
        assert!(matches!(err, Error::InvalidWebVtt(_)));
    }

    #[test]
    fn parses_plain_document() {
        let doc = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nHello CMAF\n\n00:00:02.000 --> 00:00:04.000\nsecond cue\n";
        let parsed = parse_webvtt(doc).unwrap();
        assert!(!parsed.lossy);
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(parsed.cues[0].text, "Hello CMAF");
        assert_eq!(parsed.cues[1].text, "second cue");
    }

    #[test]
    fn identifier_line_is_lossy() {
        let doc = "WEBVTT\n\ncue-1\n00:00:00.000 --> 00:00:01.000\nhi\n";
        let parsed = parse_webvtt(doc).unwrap();
        assert!(parsed.lossy);
        assert_eq!(parsed.cues[0].text, "hi");
    }

    #[test]
    fn cue_settings_are_lossy() {
        let doc = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000 line:0 align:start\nhi\n";
        let parsed = parse_webvtt(doc).unwrap();
        assert!(parsed.lossy);
        assert_eq!(parsed.cues[0].text, "hi");
    }

    #[test]
    fn inline_markup_is_lossy_but_kept() {
        let doc = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\n<i>hi</i>\n";
        let parsed = parse_webvtt(doc).unwrap();
        assert!(parsed.lossy);
        assert_eq!(parsed.cues[0].text, "<i>hi</i>");
    }

    #[test]
    fn note_style_region_blocks_are_dropped() {
        let doc = "WEBVTT\n\nNOTE this is a comment\n\nSTYLE\n::cue { color: red; }\n\n00:00:00.000 --> 00:00:01.000\nhi\n";
        let parsed = parse_webvtt(doc).unwrap();
        assert!(parsed.lossy);
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].text, "hi");
    }

    #[test]
    fn unescape_is_inverse_of_escape() {
        let text = "a &lt; b &amp; c &gt; d";
        assert_eq!(unescape_payload(text), "a < b & c > d");
        assert_eq!(
            unescape_payload(&escape_payload("a < b & c > d")),
            "a < b & c > d"
        );
    }

    #[test]
    fn parse_tolerates_lone_cr_line_ending() {
        // W3C WebVTT SS4 defines a line terminator as LF, a lone CR (not
        // followed by LF), or CRLF -- `str::lines()` alone only recognises
        // the first and third forms. Found by fuzzing: without normalising
        // the second form too, a lone CR (or the CR of a malformed doubled
        // `\r\r\n`) survived as a literal control character embedded in a
        // cue's text, and then silently vanished on the next
        // `write_document` -> `parse_webvtt` round-trip.
        let doc = "WEBVTT\r\r00:00:00.000 --> 00:00:01.000\rhi\r";
        let parsed = parse_webvtt(doc).unwrap();
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].text, "hi");

        let rewritten = write_document(&parsed.cues);
        let reparsed = parse_webvtt(&rewritten).unwrap();
        assert_eq!(reparsed.cues, parsed.cues);
    }

    #[test]
    fn whitespace_only_interior_line_is_not_a_block_delimiter() {
        // W3C WebVTT SS4's block boundary is a truly *empty* line (zero
        // characters), not any whitespace-only line. Found by fuzzing: a
        // single-space interior payload line used to be misread as the
        // blank line ending the cue, silently dropping every line after it.
        let doc = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nhi\n \n\n";
        let parsed = parse_webvtt(doc).unwrap();
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].text, "hi\n ");
    }

    #[test]
    fn multiline_payload_round_trips_through_write_and_parse() {
        let cues = alloc::vec![Cue {
            start: timed_metadata::MediaTime(90_000),
            end: timed_metadata::MediaTime(180_000),
            text: "line one\nline two".to_string(),
        }];
        let doc = write_document(&cues);
        let parsed = parse_webvtt(&doc).unwrap();
        assert!(!parsed.lossy);
        assert_eq!(parsed.cues, cues);
    }
}
