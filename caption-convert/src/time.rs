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
    let total_ms = ((h * 60 + m) * 60 + s) * 1000 + ms;
    Ok(total_ms * TICKS_PER_MS)
}

/// Replace the SRT `,` millisecond separator with `.` so
/// [`parse_timestamp`] can be shared between both formats.
pub(crate) fn normalize_comma_separator(s: &str) -> alloc::string::String {
    // SRT timestamps have exactly one comma (the ms separator); a plain
    // single-pass replace is exact and needs no escaping (unlike
    // `webvtt::escape_payload`'s multi-entity payload text).
    s.replace(',', ".")
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
    fn comma_normalization() {
        assert_eq!(normalize_comma_separator("00:00:01,500"), "00:00:01.500");
        assert_eq!(
            parse_timestamp(&normalize_comma_separator("00:00:01,500")).unwrap(),
            90_000 + 45_000
        );
    }

    #[test]
    fn hms_ms_matches_parse() {
        let ticks = parse_timestamp("01:02:03.456").unwrap();
        assert_eq!(to_hms_ms(ticks), (1, 2, 3, 456));
    }
}
