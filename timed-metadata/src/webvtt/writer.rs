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
use alloc::format;
use alloc::string::String;

/// 90 kHz ticks per millisecond (`crate::PTS_HZ` / 1000): the ratio used to
/// render a [`MediaTime`] as a WebVTT timestamp.
const TICKS_PER_MS: u64 = crate::PTS_HZ / 1000;
/// Milliseconds per hour / minute / second, for timestamp field extraction.
const MS_PER_HOUR: u64 = 3_600_000;
const MS_PER_MIN: u64 = 60_000;
const MS_PER_SEC: u64 = 1_000;

/// Format a [`MediaTime`] as a WebVTT timestamp `hh:mm:ss.ttt` (W3C WebVTT
/// §4.3.1). Hours are always emitted: the grammar `(hh:)?mm:ss.ttt` makes
/// hours *optional*, not forbidden, and always including them keeps this
/// function total and monotonic without a >=1h special case.
#[must_use]
pub fn format_timestamp(t: MediaTime) -> String {
    let total_ms = t.0 / TICKS_PER_MS;
    let hours = total_ms / MS_PER_HOUR;
    let mins = (total_ms % MS_PER_HOUR) / MS_PER_MIN;
    let secs = (total_ms % MS_PER_MIN) / MS_PER_SEC;
    let ms = total_ms % MS_PER_SEC;
    format!("{hours:02}:{mins:02}:{secs:02}.{ms:03}")
}

/// Escape a cue payload line per W3C WebVTT §6.4: `&` -> `&amp;`,
/// `<` -> `&lt;`, `>` -> `&gt;` (order is immaterial here: this is a single
/// left-to-right character scan, not a sequence of whole-string
/// find-and-replace passes, so an inserted `&amp;` is never re-scanned and
/// cannot be corrupted by a later `<`/`>` substitution).
#[must_use]
pub fn escape_payload(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
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
    for (i, line) in cue.text.lines().enumerate() {
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
/// signature, an `X-TIMESTAMP-MAP=MPEGTS:<n>,LOCAL:00:00:00.000` header
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
    let mut out = format!("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:{mpegts},LOCAL:00:00:00.000\n\n");
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
            Some("X-TIMESTAMP-MAP=MPEGTS:9000000,LOCAL:00:00:00.000")
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
        assert!(seg.contains("X-TIMESTAMP-MAP=MPEGTS:5,LOCAL:00:00:00.000"));
    }
}
