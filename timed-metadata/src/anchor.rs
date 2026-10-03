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
const MIN_EPOCH_MS: i64 = -377_705_116_800_000;
/// `9999-12-31T23:59:59.999Z` in milliseconds since the Unix epoch: the last
/// whole millisecond of `jiff::civil::DateTime`'s range.
const MAX_EPOCH_MS: i64 = 253_402_300_799_999;

/// The UTC calendar date-time `epoch_ms` milliseconds after the Unix epoch, or
/// `None` outside years -9999..=9999. (`jiff::Timestamp` is deliberately NOT
/// used: its range stops at 9999-12-30T22:00Z, which would break the
/// byte-identical output for the last day of year 9999.)
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
            let (year6, tail) = rest.split_at(6);
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
    pub fn media_to_epoch_ms(&self, t: MediaTime) -> i64 {
        let delta_ticks = i128::from(t.0) - i128::from(self.pts_90k);
        // ticks / 90_000 * 1000 == ticks / 90 ; done in i128 so nothing overflows.
        let ms = i128::from(self.utc_epoch_ms) + delta_ticks * 1000 / i128::from(crate::PTS_HZ);
        ms.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// Map a media instant to an RFC3339 / ISO-8601 UTC string (millisecond
    /// precision), clamped to the supported range (years -9999..=9999).
    pub fn rfc3339(&self, t: MediaTime) -> String {
        format_rfc3339_ms(self.media_to_epoch_ms(t))
    }

    /// Like [`Self::rfc3339`], but an unrepresentable instant is an error.
    pub fn try_rfc3339(&self, t: MediaTime) -> Result<String> {
        try_format_rfc3339_ms(self.media_to_epoch_ms(t))
    }
}

/// Format milliseconds-since-epoch as `YYYY-MM-DDTHH:MM:SS.sssZ`; values outside
/// years -9999..=9999 are clamped (use [`try_format_rfc3339_ms`] to detect them).
pub fn format_rfc3339_ms(epoch_ms: i64) -> String {
    let clamped = epoch_ms.clamp(MIN_EPOCH_MS, MAX_EPOCH_MS);
    // The clamped value is always in range, so this cannot fail.
    let dt = civil_from_epoch_ms(clamped).unwrap_or(DateTime::constant(1970, 1, 1, 0, 0, 0, 0));
    render(&dt)
}

/// Fallible [`format_rfc3339_ms`]: an instant outside years -9999..=9999 is
/// [`Error::TimestampOutOfRange`].
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
        assert_eq!(format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339_ms(86_400_000), "1970-01-02T00:00:00.000Z");
        assert_eq!(format_rfc3339_ms(1_000), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn anchor_maps_media_to_wallclock() {
        // anchor: pts 0 == epoch 1000ms. +90000 ticks (1s) -> 2000ms.
        let a = TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000,
        };
        assert_eq!(a.media_to_epoch_ms(MediaTime(90_000)), 2_000);
        assert_eq!(a.rfc3339(MediaTime(0)), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn range_constants_are_exactly_the_civil_range() {
        assert!(civil_from_epoch_ms(MIN_EPOCH_MS).is_some());
        assert!(civil_from_epoch_ms(MIN_EPOCH_MS - 1).is_none());
        assert!(civil_from_epoch_ms(MAX_EPOCH_MS).is_some());
        assert!(civil_from_epoch_ms(MAX_EPOCH_MS + 1).is_none());
        assert_eq!(format_rfc3339_ms(MAX_EPOCH_MS), "9999-12-31T23:59:59.999Z");
    }

    #[test]
    fn negative_years_keep_the_historical_four_wide_signed_form() {
        // 0000-12-31, -0001-12-31, -0010, -9999 in the old `{y:04}` spelling.
        assert_eq!(
            format_rfc3339_ms(-62_167_219_200_000),
            "0000-01-01T00:00:00.000Z"
        );
        assert_eq!(
            format_rfc3339_ms(-62_167_219_200_001),
            "-001-12-31T23:59:59.999Z"
        );
        assert_eq!(
            format_rfc3339_ms(-62_798_785_600_000),
            "-021-12-27T04:53:20.000Z"
        );
        assert_eq!(format_rfc3339_ms(MIN_EPOCH_MS), "-9999-01-01T00:00:00.000Z");
    }
}
