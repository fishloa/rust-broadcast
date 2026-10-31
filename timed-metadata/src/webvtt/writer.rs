//! WebVTT cue serialization: cue-block formatting, the `WEBVTT` document
//! header, and HLS segmented output with `X-TIMESTAMP-MAP` (RFC 8216 §3.5).
//!
//! Cites `specs/rules/webvtt-rules.md` (curated W3C WebVTT §4 + RFC 8216
//! §3.5). First-pass writer: plain-cue-only (no `STYLE`/`REGION`/`NOTE`
//! header blocks, no cue-identifier lines, no cue settings) — see
//! `crate::webvtt` module docs for the full list of documented losses.
use crate::event::MediaTime;
use crate::timeline::PTS_WRAP;
use crate::webvtt::Cue;
use crate::webvtt::text::{
    escape_payload, format_timestamp_with, normalize_line_endings, payload_lines,
};
use alloc::format;
use alloc::string::String;

/// Format a [`MediaTime`] as a WebVTT timestamp `hh:mm:ss.ttt` (W3C WebVTT
/// §4.3.1). Hours are always emitted: the grammar `(hh:)?mm:ss.ttt` makes
/// hours *optional*, not forbidden, and always including them keeps this
/// function total and monotonic without a >=1h special case. The digit math
/// is [`format_timestamp_with`], shared with the SRT writer.
#[must_use]
pub fn format_timestamp(t: MediaTime) -> String {
    format_timestamp_with(t, '.')
}

/// Render one cue block: the timings line (`start --> end`) followed by the
/// escaped payload lines, terminated with a single trailing newline. No cue
/// identifier and no cue settings are emitted (first-pass simplification).
#[must_use]
pub fn cue_block(cue: &Cue) -> String {
    let mut out = format!(
        "{} --> {}\n",
        format_timestamp(cue.start),
        format_timestamp(cue.end)
    );
    // A blank line terminates a cue block (W3C WebVTT §4.1), so an empty line
    // inside the payload would truncate the cue: `payload_lines` drops them,
    // after `normalize_line_endings` folded CRLF / lone-CR / LF to LF.
    let normalized = normalize_line_endings(&cue.text);
    for (i, line) in payload_lines(&normalized).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&escape_payload(line));
    }
    out.push('\n');
    out
}

/// Render a standalone WebVTT document: the `WEBVTT` signature, a blank
/// line, then each cue block separated by a blank line (W3C WebVTT §4).
#[must_use]
pub fn write_document(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out.push_str(&cue_block(cue));
        out.push('\n');
    }
    out
}

/// Render one HLS **segment** of WebVTT (RFC 8216 §3.5): the `WEBVTT`
/// signature, an `X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:<n>` header (field order per RFC 8216 §3.5)
/// mapping this segment's local WebVTT clock to the shared MPEG-2 TS (PES)
/// timeline, then each cue rendered with times **relative to
/// `segment_start`** (so cue timestamps stay small and segment-local, per
/// the RFC 8216 convention of `LOCAL:00:00:00.000`).
///
/// `segment_start` is the carrying media segment's first PES PTS (already
/// wrap-unrolled, e.g. via [`crate::timeline::Timeline`]); it is reduced
/// modulo 2^33 for the `MPEGTS:` field, matching the 33-bit PTS the value
/// represents on the wire. Cues with `start`/`end` before `segment_start`
/// saturate to zero (should not occur for cues that genuinely belong to this
/// segment).
#[must_use]
pub fn write_segment(cues: &[Cue], segment_start: MediaTime) -> String {
    let mpegts = segment_start.0 % PTS_WRAP;
    let mut out = format!("WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:{mpegts}\n\n");
    for cue in cues {
        let local = Cue {
            start: MediaTime(cue.start.0.saturating_sub(segment_start.0)),
            end: MediaTime(cue.end.0.saturating_sub(segment_start.0)),
            text: cue.text.clone(),
        };
        out.push_str(&cue_block(&local));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn cue(start: u64, end: u64, text: &str) -> Cue {
        Cue {
            start: MediaTime(start),
            end: MediaTime(end),
            text: text.to_string(),
        }
    }

    #[test]
    fn blank_line_in_payload_does_not_truncate_the_cue() {
        // W3C WebVTT §4.1: a blank line ends the cue block. A `\n\n` inside
        // the text must not split the cue in two.
        let block = cue_block(&cue(0, 90_000, "one\n\ntwo\n\n\nthree"));
        assert_eq!(block, "00:00:00.000 --> 00:00:01.000\none\ntwo\nthree\n");
        assert!(!block.contains("\n\n"));
        // A lone CR and CRLF are line terminators too (`str::lines` kept
        // "a\r\rb" as one line).
        let cr = cue_block(&cue(0, 90_000, "a\r\rb\r\n\r\nc"));
        assert_eq!(cr, "00:00:00.000 --> 00:00:01.000\na\nb\nc\n");
    }

    #[test]
    fn timestamp_formatting() {
        assert_eq!(format_timestamp(MediaTime(0)), "00:00:00.000");
        // 90_000 ticks = 1.000s
        assert_eq!(format_timestamp(MediaTime(90_000)), "00:00:01.000");
        // 1 minute = 60 * 90_000 ticks
        assert_eq!(format_timestamp(MediaTime(60 * 90_000)), "00:01:00.000");
        // 1 hour
        assert_eq!(format_timestamp(MediaTime(3_600 * 90_000)), "01:00:00.000");
        // sub-second: 90 ticks = 1 ms at the 90 kHz clock.
        assert_eq!(format_timestamp(MediaTime(90)), "00:00:00.001");
    }

    #[test]
    fn escape_order_is_safe() {
        // A literal "&lt;" in the source text must not become "&amp;lt;" —
        // single-pass char scanning only ever escapes the original '&'.
        assert_eq!(escape_payload("&lt;"), "&amp;lt;");
        assert_eq!(escape_payload("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn cue_block_multiline() {
        let c = cue(90_000, 180_000, "line one\nline two");
        let block = cue_block(&c);
        assert_eq!(block, "00:00:01.000 --> 00:00:02.000\nline one\nline two\n");
    }

    #[test]
    fn write_document_signature_and_blank_separation() {
        let cues = [cue(0, 1_000, "a"), cue(2_000, 3_000, "b")];
        let doc = write_document(&cues);
        assert!(doc.starts_with("WEBVTT\n\n"));
        // Exactly one blank line between the two cue blocks.
        assert!(doc.contains("a\n\n00:00:00"));
    }

    #[test]
    fn write_segment_timestamp_map_and_local_times() {
        let cues = [cue(9_090_000, 9_180_000, "hi")]; // 101.0s .. 102.0s absolute
        let seg = write_segment(&cues, MediaTime(9_000_000)); // segment starts at 100.0s
        let mut lines = seg.lines();
        assert_eq!(lines.next(), Some("WEBVTT"));
        assert_eq!(
            lines.next(),
            Some("X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:9000000")
        );
        assert_eq!(lines.next(), Some(""));
        // cue-local time = 9_090_000 - 9_000_000 = 90_000 ticks = 1.000s
        assert_eq!(lines.next(), Some("00:00:01.000 --> 00:00:02.000"));
        assert_eq!(lines.next(), Some("hi"));
    }

    #[test]
    fn write_segment_mpegts_wraps_at_33_bits() {
        let wrap = PTS_WRAP; // 2^33
        let seg = write_segment(&[], MediaTime(wrap + 5));
        assert!(seg.contains("X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:5"));
    }
}
