//! Media-time ↔ wall-clock mapping for conversions that cross into UTC.
use crate::error::{Error, Result};
use crate::event::MediaTime;
use alloc::string::String;
use jiff::SignedDuration;
use jiff::civil::{DateTime, date};
use jiff::fmt::temporal::DateTimePrinter;

/// Three fractional digits (`…:SS.sss`); the `Z` is appended by the caller.
const PRINTER: DateTimePrinter = DateTimePrinter::new().precision(Some(3));

/// `-9999-01-01T00:00:00Z` in milliseconds since the Unix epoch: the first
/// instant of `jiff::civil::DateTime`'s range.
#[cfg(test)]
const MIN_EPOCH_MS: i64 = -377_705_116_800_000;
/// `9999-12-31T23:59:59.999Z` in milliseconds since the Unix epoch: the last
/// whole millisecond of `jiff::civil::DateTime`'s range.
#[cfg(test)]
const MAX_EPOCH_MS: i64 = 253_402_300_799_999;

/// The UTC calendar date-time `epoch_ms` milliseconds after the Unix epoch, or
/// `None` outside years -9999..=9999. (`jiff::Timestamp` is deliberately NOT
/// used: its range stops at 9999-12-30T22:00Z, which would break the
/// byte-identical output for the last day of year 9999.)
/// Digits jiff prints for a negative year, which this renderer trims back.
const PRINTED_YEAR_DIGITS: usize = 6;

fn civil_from_epoch_ms(epoch_ms: i64) -> Option<DateTime> {
    date(1970, 1, 1)
        .at(0, 0, 0, 0)
        .checked_add(SignedDuration::from_millis(epoch_ms))
        .ok()
}

fn render(dt: &DateTime) -> String {
    let printed = PRINTER.datetime_to_string(dt);
    let mut out = match printed.strip_prefix('-') {
        // jiff prints a negative year as `-` + 6 digits (`-000001`); this crate
        // has always printed `{year:04}`, i.e. `-001` (sign + at least 3
        // digits), and keeps doing so. (Negative years are not RFC 3339.)
        Some(rest) => {
            let (year6, tail) = rest.split_at(PRINTED_YEAR_DIGITS);
            let digits = year6.trim_start_matches('0');
            alloc::format!("-{digits:0>3}{tail}")
        }
        None => printed,
    };
    out.push('Z');
    out
}

/// Maps a known 90 kHz PTS to the UTC instant it represents (linear at 90 kHz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeAnchor {
    /// A reference PTS, in 90 kHz ticks.
    pub pts_90k: u64,
    /// The UTC time that `pts_90k` corresponds to, in milliseconds since the Unix epoch.
    pub utc_epoch_ms: i64,
}

impl TimeAnchor {
    /// Map a media instant to milliseconds since the Unix epoch, saturating at
    /// the `i64` limits.
    ///
    /// The millisecond is floored, so an instant that lies between two
    /// milliseconds maps to the same value whether it is before or after the
    /// anchor: the mapping has no skew at the anchor and is monotonic.
    pub fn media_to_epoch_ms(&self, t: MediaTime) -> i64 {
        let delta_ticks = i128::from(t.0) - i128::from(self.pts_90k);
        // ticks / 90_000 * 1000 == ticks / 90 ; done in i128 so nothing overflows.
        let ms = i128::from(self.utc_epoch_ms)
            + delta_ticks
                .saturating_mul(i128::from(broadcast_common::units::MILLIS_PER_SEC))
                .div_euclid(i128::from(crate::PTS_HZ));
        ms.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// Map a media instant to an RFC3339 / ISO-8601 UTC string (millisecond
    /// precision); an instant outside years -9999..=9999 is
    /// [`Error::TimestampOutOfRange`].
    pub fn try_rfc3339(&self, t: MediaTime) -> Result<String> {
        try_format_rfc3339_ms(self.media_to_epoch_ms(t))
    }
}

/// Format milliseconds-since-epoch as `YYYY-MM-DDTHH:MM:SS.sssZ`; an instant
/// outside years -9999..=9999 is [`Error::TimestampOutOfRange`].
pub fn try_format_rfc3339_ms(epoch_ms: i64) -> Result<String> {
    let dt = civil_from_epoch_ms(epoch_ms).ok_or(Error::TimestampOutOfRange(epoch_ms))?;
    Ok(render(&dt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::MediaTime;

    #[test]
    fn epoch_zero_formats_unix_epoch() {
        assert_eq!(
            try_format_rfc3339_ms(0).unwrap(),
            "1970-01-01T00:00:00.000Z"
        );
        assert_eq!(
            try_format_rfc3339_ms(86_400_000).unwrap(),
            "1970-01-02T00:00:00.000Z"
        );
        assert_eq!(
            try_format_rfc3339_ms(1_000).unwrap(),
            "1970-01-01T00:00:01.000Z"
        );
    }

    #[test]
    fn anchor_maps_media_to_wallclock() {
        // anchor: pts 0 == epoch 1000ms. +90000 ticks (1s) -> 2000ms.
        let a = TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000,
        };
        assert_eq!(a.media_to_epoch_ms(MediaTime(90_000)), 2_000);
        assert_eq!(
            a.try_rfc3339(MediaTime(0)).unwrap(),
            "1970-01-01T00:00:01.000Z"
        );
    }

    #[test]
    fn sub_millisecond_offsets_floor_on_both_sides_of_the_anchor() {
        // 1 ms is 90 ticks: one tick before the anchor must not round up to
        // the anchor's own millisecond, and one tick after must not round
        // down to it. Truncating the tick delta toward zero got the "before"
        // side wrong (it took the magnitude first), a 1 ms skew at the anchor.
        let a = TimeAnchor {
            pts_90k: 900_000,
            utc_epoch_ms: 1_000,
        };
        assert_eq!(a.media_to_epoch_ms(MediaTime(900_001)), 1_000);
        assert_eq!(a.media_to_epoch_ms(MediaTime(899_999)), 999);
        assert_eq!(a.media_to_epoch_ms(MediaTime(899_911)), 999);
        assert_eq!(a.media_to_epoch_ms(MediaTime(899_910)), 999);
        assert_eq!(a.media_to_epoch_ms(MediaTime(899_909)), 998);
    }

    #[test]
    fn range_constants_are_exactly_the_civil_range() {
        assert!(civil_from_epoch_ms(MIN_EPOCH_MS).is_some());
        assert!(civil_from_epoch_ms(MIN_EPOCH_MS - 1).is_none());
        assert!(civil_from_epoch_ms(MAX_EPOCH_MS).is_some());
        assert!(civil_from_epoch_ms(MAX_EPOCH_MS + 1).is_none());
        assert_eq!(
            try_format_rfc3339_ms(MAX_EPOCH_MS).unwrap(),
            "9999-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn negative_years_keep_the_historical_four_wide_signed_form() {
        // 0000-12-31, -0001-12-31, -0010, -9999 in the old `{y:04}` spelling.
        assert_eq!(
            try_format_rfc3339_ms(-62_167_219_200_000).unwrap(),
            "0000-01-01T00:00:00.000Z"
        );
        assert_eq!(
            try_format_rfc3339_ms(-62_167_219_200_001).unwrap(),
            "-001-12-31T23:59:59.999Z"
        );
        assert_eq!(
            try_format_rfc3339_ms(-62_798_785_600_000).unwrap(),
            "-021-12-27T04:53:20.000Z"
        );
        assert_eq!(
            try_format_rfc3339_ms(MIN_EPOCH_MS).unwrap(),
            "-9999-01-01T00:00:00.000Z"
        );
    }
}
