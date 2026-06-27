//! Media-time ↔ wall-clock mapping for conversions that cross into UTC.
use crate::event::MediaTime;
use alloc::{format, string::String};

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
    /// Map a media instant to milliseconds since the Unix epoch.
    pub fn media_to_epoch_ms(&self, t: MediaTime) -> i64 {
        let delta_ticks = t.0 as i64 - self.pts_90k as i64;
        // ticks / 90_000 * 1000 == ticks / 90 ; do it in i128 to avoid overflow.
        self.utc_epoch_ms + (delta_ticks as i128 * 1000 / crate::PTS_HZ as i128) as i64
    }

    /// Map a media instant to an RFC3339 / ISO-8601 UTC string (millisecond precision).
    pub fn rfc3339(&self, t: MediaTime) -> String {
        format_rfc3339_ms(self.media_to_epoch_ms(t))
    }
}

/// Format milliseconds-since-epoch as `YYYY-MM-DDTHH:MM:SS.sssZ`.
pub fn format_rfc3339_ms(epoch_ms: i64) -> String {
    let (secs, ms) = (epoch_ms.div_euclid(1000), epoch_ms.rem_euclid(1000));
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y, mo, d, h, m, s, ms
    )
}

/// Convert days-since-Unix-epoch to (year, month, day). Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// chrono interop helpers can be added behind cfg(feature="chrono") later

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
}
