//! Shared cue-timestamp parsing for the WebVTT and SRT parsers.
//!
//! Both formats use the same `(hh:)?mm:ss<sep>ttt` grammar (W3C WebVTT §4.3.1
//! uses `.`; SRT's de facto convention uses `,`); the caller picks the
//! separator it expects, and both write paths (`timed_metadata::webvtt`'s
//! `format_timestamp` for WebVTT, [`crate::srt::format_srt_timestamp`] for
//! SRT) share the exact same `hh:mm:ss` digit math so the two formats never
//! drift on rounding.

use crate::error::Error;
use alloc::string::ToString;

/// 90 kHz ticks per millisecond -- the same ratio `timed-metadata`'s WebVTT
/// writer uses, derived from the shared [`timed_metadata::PTS_HZ`] constant
/// rather than a re-typed literal.
pub(crate) const TICKS_PER_MS: u64 = timed_metadata::PTS_HZ / 1000;

const MS_PER_HOUR: u64 = 3_600_000;
const MS_PER_MIN: u64 = 60_000;
const MS_PER_SEC: u64 = 1_000;

/// Parse a `(hh:)?mm:ss.ttt` timestamp (the separator before the
/// milliseconds field must already be normalised to `.` by the caller — SRT
/// callers pass the `,`-separated string through
/// [`normalize_comma_separator`] first) into 90 kHz ticks.
pub(crate) fn parse_timestamp(s: &str) -> Result<u64, Error> {
    let invalid = || Error::InvalidTimestamp(s.to_string());
    let (rest, ms_str) = s.split_once('.').ok_or_else(invalid)?;
    if ms_str.len() != 3 || !ms_str.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let ms: u64 = ms_str.parse().map_err(|_| invalid())?;

    let fields: alloc::vec::Vec<&str> = rest.split(':').collect();
    let (h, m, sec): (u64, &str, &str) = match fields.as_slice() {
        [m, s] => (0, m, s),
        [h, m, s] => (h.parse().map_err(|_| invalid())?, m, s),
        _ => return Err(invalid()),
    };
    if m.len() != 2 || sec.len() != 2 {
        return Err(invalid());
    }
    let m: u64 = m.parse().map_err(|_| invalid())?;
    let s: u64 = sec.parse().map_err(|_| invalid())?;
    if m >= 60 || s >= 60 {
        return Err(invalid());
    }
    // WebVTT's hour field has no digit-count cap (W3C WebVTT SS4.3.1: "one
    // or more" digits), so an attacker-controlled `h` up to `u64::MAX` is a
    // *validly parsed* value that still overflows the `h*3600 + m*60 + s`
    // to-milliseconds, and then the `* TICKS_PER_MS` to-90kHz-ticks, widening
    // multiplications. Debug-assertions builds (this crate's fuzz targets;
    // any downstream debug build) would panic on that overflow; release
    // builds would silently wrap to a bogus timestamp -- checked arithmetic
    // turns both into the same explicit, documented `InvalidTimestamp`.
    let total_ms = h
        .checked_mul(MS_PER_HOUR)
        .and_then(|v| v.checked_add(m * MS_PER_MIN))
        .and_then(|v| v.checked_add(s * MS_PER_SEC))
        .and_then(|v| v.checked_add(ms))
        .ok_or_else(invalid)?;
    total_ms.checked_mul(TICKS_PER_MS).ok_or_else(invalid)
}

/// Replace the SRT `,` millisecond separator with `.` so
/// [`parse_timestamp`] can be shared between both formats.
pub(crate) fn normalize_comma_separator(s: &str) -> alloc::string::String {
    // SRT timestamps have exactly one comma (the ms separator); a plain
    // single-pass replace is exact and needs no escaping (unlike
    // `webvtt::escape_payload`'s multi-entity payload text).
    s.replace(',', ".")
}

/// Normalise every WebVTT line-terminator form to a single `\n`.
///
/// W3C WebVTT SS4 defines a "line terminator" as one of three forms: a lone
/// LF, a lone CR *not* followed by LF, or a CRLF pair -- i.e. a bare `\r` is
/// itself a valid line break, not just the `\r` half of `\r\n`. Rust's
/// `str::lines()` (and a plain `"\r\n" -> "\n"` substring replace, SRT's own
/// de facto convention) only recognise LF and CRLF: a lone CR, or a
/// malformed doubled `\r\r\n`, survives as a literal control character
/// embedded in a parsed cue's payload -- and since `str::lines()` also
/// swallows a `\r` immediately preceding a `\n` even when that `\r` is
/// genuine payload text (not a terminator), the embedded CR is then silently
/// dropped the next time that cue is written back out. Normalising every
/// terminator form up front, before either parser's own `.lines()` /
/// `"\n\n"`-splitting logic runs, closes both holes: no stray `\r` can ever
/// reach a `Cue`'s text, so writing and re-parsing a cue is always faithful.
/// Shared between [`crate::webvtt::parse_webvtt`] and [`crate::srt::parse_srt`]
/// (SRT has no formal spec, but real encoders/players are at least as
/// permissive about line endings as this).
pub(crate) fn normalize_line_endings(input: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(input.len());
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
    out
}

/// Render 90 kHz ticks as `hh:mm:ss` fields plus milliseconds, for the SRT
/// writer to join with a `,` (WebVTT's own `format_timestamp` does the same
/// math with a `.` join; kept in lock-step by both reading `PTS_HZ`).
pub(crate) fn to_hms_ms(ticks: u64) -> (u64, u64, u64, u64) {
    let total_ms = ticks / TICKS_PER_MS;
    let hours = total_ms / MS_PER_HOUR;
    let mins = (total_ms % MS_PER_HOUR) / MS_PER_MIN;
    let secs = (total_ms % MS_PER_MIN) / MS_PER_SEC;
    let ms = total_ms % MS_PER_SEC;
    (hours, mins, secs, ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_basic() {
        assert_eq!(parse_timestamp("00:00:01.000").unwrap(), 90_000);
        assert_eq!(parse_timestamp("01:00:00.000").unwrap(), 3_600 * 90_000);
        assert_eq!(parse_timestamp("00:00:00.001").unwrap(), 90);
    }

    #[test]
    fn rejects_bad_ms_length() {
        assert!(parse_timestamp("00:00:01.0").is_err());
        assert!(parse_timestamp("00:00:01.00000").is_err());
    }

    #[test]
    fn rejects_out_of_range_fields() {
        assert!(parse_timestamp("00:60:00.000").is_err());
        assert!(parse_timestamp("00:00:60.000").is_err());
    }

    #[test]
    fn rejects_missing_dot() {
        assert!(parse_timestamp("00:00:01").is_err());
    }

    #[test]
    fn rejects_hour_overflow_instead_of_panicking() {
        // W3C WebVTT SS4.3.1 puts no digit-count cap on the hour field, so
        // this is a validly-parsed `u64` that still overflows the
        // to-milliseconds/to-ticks widening multiplications (found by
        // fuzzing -- a debug-assertions build used to panic on this input
        // instead of returning `Err`).
        assert!(parse_timestamp("11662322688380341727:00:01.000").is_err());
        // u64::MAX itself, for the same reason.
        assert!(parse_timestamp("18446744073709551615:00:00.000").is_err());
    }

    #[test]
    fn comma_normalization() {
        assert_eq!(normalize_comma_separator("00:00:01,500"), "00:00:01.500");
        assert_eq!(
            parse_timestamp(&normalize_comma_separator("00:00:01,500")).unwrap(),
            90_000 + 45_000
        );
    }

    #[test]
    fn line_ending_normalization_handles_all_three_terminator_forms() {
        // LF-only: unchanged.
        assert_eq!(normalize_line_endings("a\nb"), "a\nb");
        // CRLF: the pair collapses to one LF (the `\r` is not duplicated).
        assert_eq!(normalize_line_endings("a\r\nb"), "a\nb");
        // Lone CR (W3C WebVTT SS4's third terminator form): becomes LF too.
        assert_eq!(normalize_line_endings("a\rb"), "a\nb");
        // A malformed doubled `\r\r\n` must not leave a stray `\r` behind --
        // this is the case issue found by fuzzing: `str::lines()` only
        // strips the *second* `\r` (the one immediately before `\n`),
        // leaving the first as literal text that then vanishes on rewrite.
        assert_eq!(normalize_line_endings("a\r\r\nb"), "a\n\nb");
        // Trailing lone CR with no following character at all.
        assert_eq!(normalize_line_endings("a\r"), "a\n");
    }

    #[test]
    fn hms_ms_matches_parse() {
        let ticks = parse_timestamp("01:02:03.456").unwrap();
        assert_eq!(to_hms_ms(ticks), (1, 2, 3, 456));
    }
}
