//! Shared WebVTT / SRT text helpers: the cue-timestamp grammar (both
//! directions), payload escaping (both directions) and line-terminator
//! normalisation.
//!
//! WebVTT (W3C WebVTT §4.3.1) and SRT (de facto) use the same
//! `(hh:)?mm:ss<sep>ttt` timestamp grammar, `.` for WebVTT and `,` for SRT;
//! the caller picks the separator. Before this module the *writer* half lived
//! here and the *parser* half, the payload *unescape*, and a second copy of the
//! hour/minute/second digit math lived in `caption-convert` ("kept in
//! lock-step" by a comment), so a rounding or overflow fix in one never
//! reached the other. This is now the single owner; `caption-convert` is a
//! thin client.

use crate::event::MediaTime;
use alloc::borrow::Cow;
use alloc::format;
use alloc::string::String;

/// 90 kHz ticks per millisecond (`crate::PTS_HZ` / 1000): the ratio used to
/// render and parse a [`MediaTime`] as a cue timestamp.
const TICKS_PER_MS: u64 = crate::PTS_HZ / 1000;
/// Milliseconds per hour / minute / second, for timestamp field extraction.
const MS_PER_HOUR: u64 = 3_600_000;
const MS_PER_MIN: u64 = 60_000;
const MS_PER_SEC: u64 = 1_000;

/// A cue timestamp did not match `(hh:)?mm:ss<sep>ttt` (or overflowed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampError;

impl core::fmt::Display for TimestampError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("invalid cue timestamp (expected (hh:)?mm:ss<sep>ttt)")
    }
}

impl core::error::Error for TimestampError {}

/// Split a [`MediaTime`] into `(hours, minutes, seconds, milliseconds)`.
#[must_use]
pub fn hms_ms(t: MediaTime) -> (u64, u64, u64, u64) {
    let total_ms = t.0 / TICKS_PER_MS;
    let hours = total_ms / MS_PER_HOUR;
    let mins = (total_ms % MS_PER_HOUR) / MS_PER_MIN;
    let secs = (total_ms % MS_PER_MIN) / MS_PER_SEC;
    let ms = total_ms % MS_PER_SEC;
    (hours, mins, secs, ms)
}

/// Format a [`MediaTime`] as `hh:mm:ss<sep>ttt`: `.` for WebVTT (W3C WebVTT
/// §4.3.1), `,` for SRT. Hours are always emitted: the grammar makes them
/// *optional*, not forbidden, and always including them keeps this function
/// total and monotonic without a >=1h special case.
#[must_use]
pub fn format_timestamp_with(t: MediaTime, ms_sep: char) -> String {
    let (hours, mins, secs, ms) = hms_ms(t);
    format!("{hours:02}:{mins:02}:{secs:02}{ms_sep}{ms:03}")
}

/// ASCII digits only (and non-empty). `str::parse::<u64>` also accepts a
/// leading `+`, which no timestamp grammar here allows (#1109 CV-O1).
fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parse a `(hh:)?mm:ss<sep>ttt` timestamp into a [`MediaTime`], where
/// `ms_sep` is the millisecond separator: `.` for WebVTT, `,` for SRT.
/// Splitting on `ms_sep` in place avoids a per-timestamp `String` (#1109
/// CV-O3).
///
/// # Errors
/// [`TimestampError`] for any deviation from the grammar (wrong separator,
/// digit counts, minutes/seconds >= 60, a `+` sign) and for an hour field so
/// large that the to-milliseconds / to-ticks arithmetic would overflow.
pub fn parse_timestamp(s: &str, ms_sep: char) -> Result<MediaTime, TimestampError> {
    let (rest, ms_str) = s.split_once(ms_sep).ok_or(TimestampError)?;
    if ms_str.len() != 3 || !all_digits(ms_str) {
        return Err(TimestampError);
    }
    let ms: u64 = ms_str.parse().map_err(|_| TimestampError)?;

    let mut fields = rest.split(':');
    let (first, second, third) = (fields.next(), fields.next(), fields.next());
    if fields.next().is_some() {
        return Err(TimestampError);
    }
    let (h_str, m, sec) = match (first, second, third) {
        (Some(m), Some(s), None) => (None, m, s),
        (Some(h), Some(m), Some(s)) => (Some(h), m, s),
        _ => return Err(TimestampError),
    };
    let h: u64 = match h_str {
        Some(h) if all_digits(h) => h.parse().map_err(|_| TimestampError)?,
        Some(_) => return Err(TimestampError),
        None => 0,
    };
    if m.len() != 2 || sec.len() != 2 || !all_digits(m) || !all_digits(sec) {
        return Err(TimestampError);
    }
    let m: u64 = m.parse().map_err(|_| TimestampError)?;
    let s: u64 = sec.parse().map_err(|_| TimestampError)?;
    if m >= 60 || s >= 60 {
        return Err(TimestampError);
    }
    // WebVTT's hour field has no digit-count cap (W3C WebVTT §4.3.1: "one or
    // more" digits), so an attacker-controlled `h` up to `u64::MAX` is a
    // *validly parsed* value that still overflows the `h*3600 + m*60 + s`
    // to-milliseconds, and then the `* TICKS_PER_MS` to-90kHz-ticks, widening
    // multiplications. Debug-assertions builds (fuzz targets, downstream debug
    // builds) would panic on that overflow; release builds would silently
    // wrap to a bogus timestamp - checked arithmetic turns both into the same
    // explicit `TimestampError`.
    let total_ms = h
        .checked_mul(MS_PER_HOUR)
        .and_then(|v| v.checked_add(m * MS_PER_MIN))
        .and_then(|v| v.checked_add(s * MS_PER_SEC))
        .and_then(|v| v.checked_add(ms))
        .ok_or(TimestampError)?;
    total_ms
        .checked_mul(TICKS_PER_MS)
        .map(MediaTime)
        .ok_or(TimestampError)
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

/// Unescape the three entities [`escape_payload`] emits (`&amp;` `&lt;`
/// `&gt;`), left-to-right, single pass - the exact inverse of
/// [`escape_payload`]. Other WebVTT-defined entities (`&lrm;` `&rlm;`
/// `&nbsp;`) are out of scope and pass through literally (a documented, not
/// silent, limitation - they are rare in real captions and this crate emits
/// none of them).
#[must_use]
pub fn unescape_payload(text: &str) -> String {
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

/// Normalise every line-terminator form to a single `\n`.
///
/// W3C WebVTT §4 defines a "line terminator" as one of three forms: a lone LF,
/// a lone CR *not* followed by LF, or a CRLF pair - i.e. a bare `\r` is itself
/// a valid line break, not just the `\r` half of `\r\n`. Rust's `str::lines()`
/// (and a plain `"\r\n" -> "\n"` replace, SRT's own de facto convention) only
/// recognise LF and CRLF: a lone CR, or a malformed doubled `\r\r\n`, survives
/// as a literal control character embedded in a parsed cue's payload - and
/// since `str::lines()` also swallows a `\r` immediately preceding a `\n` even
/// when that `\r` is genuine payload text, the embedded CR is then silently
/// dropped the next time that cue is written back out. Normalising every
/// terminator form up front, before any parser's own `.lines()` /
/// `"\n\n"`-splitting logic runs, closes both holes. Borrowed (no allocation)
/// when the input contains no `\r` at all.
#[must_use]
pub fn normalize_line_endings(input: &str) -> Cow<'_, str> {
    if !input.contains('\r') {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next(); // the paired LF of a CRLF terminator
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// The non-empty lines of an already-[`normalize_line_endings`]-normalised cue
/// payload. A blank line terminates a cue block (W3C WebVTT §4.1) and an
/// SRT reader reads one as the end of the block, so an empty line inside a
/// payload would truncate the cue and turn the rest into a bogus block: both
/// writers drop them.
pub fn payload_lines(normalized: &str) -> impl Iterator<Item = &str> {
    normalized.split('\n').filter(|l| !l.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hms_ms_splits_every_field() {
        // 1h 2m 3s 456ms
        let ticks = (3_600_000 + 2 * 60_000 + 3_000 + 456) * TICKS_PER_MS;
        assert_eq!(hms_ms(MediaTime(ticks)), (1, 2, 3, 456));
    }

    #[test]
    fn format_uses_the_requested_separator() {
        let t = MediaTime(90_000 + 45_000);
        assert_eq!(format_timestamp_with(t, '.'), "00:00:01.500");
        assert_eq!(format_timestamp_with(t, ','), "00:00:01,500");
    }

    #[test]
    fn format_then_parse_round_trips() {
        for ticks in [
            0,
            90,
            90_000,
            3_600 * 90_000 + 123 * 90,
            99 * 3_600 * 90_000,
        ] {
            for sep in ['.', ','] {
                let s = format_timestamp_with(MediaTime(ticks), sep);
                assert_eq!(parse_timestamp(&s, sep), Ok(MediaTime(ticks)), "{s}");
            }
        }
    }

    #[test]
    fn parse_basic() {
        assert_eq!(parse_timestamp("00:00:01.000", '.'), Ok(MediaTime(90_000)));
        assert_eq!(
            parse_timestamp("01:00:00.000", '.'),
            Ok(MediaTime(3_600 * 90_000))
        );
        assert_eq!(parse_timestamp("00:00:00.001", '.'), Ok(MediaTime(90)));
        // hour-less form
        assert_eq!(
            parse_timestamp("01:02.500", '.'),
            Ok(MediaTime(62_500 * 90))
        );
    }

    #[test]
    fn parse_rejects_bad_ms_length() {
        assert!(parse_timestamp("00:00:01.0", '.').is_err());
        assert!(parse_timestamp("00:00:01.00000", '.').is_err());
    }

    #[test]
    fn parse_rejects_out_of_range_fields() {
        assert!(parse_timestamp("00:60:00.000", '.').is_err());
        assert!(parse_timestamp("00:00:60.000", '.').is_err());
    }

    #[test]
    fn parse_rejects_missing_separator() {
        assert!(parse_timestamp("00:00:01", '.').is_err());
    }

    #[test]
    fn parse_rejects_hour_overflow_instead_of_panicking() {
        // W3C WebVTT §4.3.1 puts no digit-count cap on the hour field, so this
        // is a validly-parsed `u64` that still overflows the to-milliseconds /
        // to-ticks widening multiplications (found by fuzzing - a
        // debug-assertions build used to panic on this input).
        assert!(parse_timestamp("11662322688380341727:00:01.000", '.').is_err());
        assert!(parse_timestamp("18446744073709551615:00:00.000", '.').is_err());
    }

    #[test]
    fn the_wrong_separator_is_an_error_not_a_silent_accept() {
        assert_eq!(
            parse_timestamp("00:00:01,500", ','),
            Ok(MediaTime(90_000 + 45_000))
        );
        assert!(parse_timestamp("00:00:01,500", '.').is_err());
        assert!(parse_timestamp("00:00:01.500", ',').is_err());
    }

    /// CV-O1 (#1109): `str::parse::<u64>` accepts a leading `+`; the timestamp
    /// grammar is digits only.
    #[test]
    fn plus_sign_is_rejected_in_every_field() {
        for bad in [
            "+0:00:01.000",
            "00:+1:01.000",
            "00:00:+1.000",
            "00:00:01.+00",
            "+1:00:00:01.000",
        ] {
            assert!(parse_timestamp(bad, '.').is_err(), "{bad}");
        }
        assert!(parse_timestamp("00:00:01.000", '.').is_ok());
    }

    #[test]
    fn escape_then_unescape_is_the_identity() {
        let text = "a < b & c > d &amp; &lt; &gt; &unknown; &";
        assert_eq!(unescape_payload(&escape_payload(text)), text);
        assert_eq!(unescape_payload("a &lt; b &amp; c &gt; d"), "a < b & c > d");
        // Unknown entities and a trailing '&' pass through literally.
        assert_eq!(unescape_payload("&nbsp; &"), "&nbsp; &");
    }

    #[test]
    fn escape_order_is_safe() {
        assert_eq!(escape_payload("&lt;"), "&amp;lt;");
        assert_eq!(escape_payload("<&>"), "&lt;&amp;&gt;");
    }

    #[test]
    fn line_ending_normalisation_handles_all_three_terminator_forms() {
        assert_eq!(normalize_line_endings("a\nb"), "a\nb");
        assert_eq!(normalize_line_endings("a\r\nb"), "a\nb");
        assert_eq!(normalize_line_endings("a\rb"), "a\nb");
        assert_eq!(normalize_line_endings("a\r\r\nb"), "a\n\nb");
        assert_eq!(normalize_line_endings("a\n\rb"), "a\n\nb");
        assert_eq!(normalize_line_endings(""), "");
    }

    #[test]
    fn normalisation_borrows_when_there_is_no_cr() {
        assert!(matches!(normalize_line_endings("a\nb"), Cow::Borrowed(_)));
        assert!(matches!(normalize_line_endings("a\rb"), Cow::Owned(_)));
    }

    #[test]
    fn payload_lines_drops_blank_lines() {
        let n = normalize_line_endings("one\r\n\r\ntwo\rthree\n");
        let lines: alloc::vec::Vec<&str> = payload_lines(&n).collect();
        assert_eq!(lines, ["one", "two", "three"]);
    }
}
