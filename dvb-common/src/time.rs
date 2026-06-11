//! UTC time and duration codecs for DVB wire fields.
//!
//! DVB carries wall-clock time as a 16-bit Modified Julian Date plus 24-bit BCD
//! HHMMSS (EN 300 468 Annex C), and event durations as 24-bit BCD HHMMSS. The
//! duration codec is dependency-free; the MJD↔calendar conversion needs a date
//! library and so lives behind the `chrono` feature.

use crate::bcd::{from_bcd_byte, to_bcd_byte};
use core::time::Duration;

/// Decoded 5-byte DVB UTC time (16-bit MJD + 24-bit BCD `HHMMSS`).
///
/// Produced by [`decode_mjd_bcd`]; field values are validated (months 1–12,
/// days 1–31, hours 0–23, minutes/seconds 0–59). The year is the full
/// calendar year (e.g. 2023, not an offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MjdBcdDateTime {
    /// Calendar year (full, e.g. 2023).
    pub year: u16,
    /// Month of year (1–12).
    pub month: u8,
    /// Day of month (1–31).
    pub day: u8,
    /// Hour of day (0–23).
    pub hour: u8,
    /// Minute of hour (0–59).
    pub minute: u8,
    /// Second of minute (0–59).
    pub second: u8,
}

/// Decode a 5-byte DVB UTC time (16-bit MJD + 24-bit BCD `HHMMSS`) to a
/// plain [`MjdBcdDateTime`].
///
/// Unlike [`decode_mjd_bcd_utc`], this is dependency-free (no `chrono`
/// feature required). Returns `None` if the BCD nibbles are out of range
/// or the minute/second fields exceed 59. The MJD→calendar conversion
/// follows ETSI EN 300 468 Annex C.
#[must_use]
pub fn decode_mjd_bcd(raw: [u8; 5]) -> Option<MjdBcdDateTime> {
    let mjd = u16::from_be_bytes([raw[0], raw[1]]);
    let h = from_bcd_byte(raw[2])?;
    let mi = from_bcd_byte(raw[3])?;
    let s = from_bcd_byte(raw[4])?;
    if mi > 59 || s > 59 || h > 23 {
        return None;
    }
    let (year, month, day) = mjd_to_ymd_nogate(mjd)?;
    Some(MjdBcdDateTime {
        year,
        month,
        day,
        hour: h,
        minute: mi,
        second: s,
    })
}

/// Encode a [`MjdBcdDateTime`] to a 5-byte DVB UTC time.
///
/// Returns `None` if any field is out of the representable range.
#[must_use]
pub fn encode_mjd_bcd(dt: MjdBcdDateTime) -> Option<[u8; 5]> {
    let mjd = ymd_to_mjd_nogate(i32::from(dt.year), u32::from(dt.month), u32::from(dt.day))?;
    let [m0, m1] = mjd.to_be_bytes();
    Some([
        m0,
        m1,
        to_bcd_byte(dt.hour)?,
        to_bcd_byte(dt.minute)?,
        to_bcd_byte(dt.second)?,
    ])
}

/// Convert a 16-bit Modified Julian Date to `(year, month, day)`.
///
/// MJD→calendar per ETSI EN 300 468 Annex C. This is the dependency-free
/// version of the chrono-gated [`mjd_to_ymd`].
fn mjd_to_ymd_nogate(mjd: u16) -> Option<(u16, u8, u8)> {
    let mjd = i64::from(mjd);
    let y_prime = ((mjd as f64 - 15_078.2) / 365.25) as i64;
    let m_prime = ((mjd as f64 - 14_956.1 - (y_prime as f64 * 365.25).floor()) / 30.6001) as i64;
    let d = mjd
        - 14_956
        - (y_prime as f64 * 365.25).floor() as i64
        - (m_prime as f64 * 30.6001).floor() as i64;
    let k = i64::from(m_prime == 14 || m_prime == 15);
    let y = y_prime + k + 1900;
    let m = m_prime - 1 - k * 12;
    let y_u16 = u16::try_from(y).ok()?;
    let m_u8 = u8::try_from(m).ok()?;
    let d_u8 = u8::try_from(d).ok()?;
    if !(1..=12).contains(&m_u8) || !(1..=31).contains(&d_u8) {
        return None;
    }
    Some((y_u16, m_u8, d_u8))
}

/// Convert a `(year, month, day)` date to a 16-bit Modified Julian Date.
///
/// Calendar→MJD per ETSI EN 300 468 Annex C. This is the dependency-free
/// version of the chrono-gated [`ymd_to_mjd`].
fn ymd_to_mjd_nogate(year: i32, month: u32, day: u32) -> Option<u16> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let l = if month <= 2 { 1.0 } else { 0.0 };
    let y = f64::from(year - 1900);
    let m = f64::from(month);
    let mjd = 14_956.0
        + f64::from(day)
        + ((y - l) * 365.25).floor()
        + ((m + 1.0 + l * 12.0) * 30.6001).floor();
    if (0.0..=f64::from(u16::MAX)).contains(&mjd) {
        Some(mjd as u16)
    } else {
        None
    }
}

/// Decode a 24-bit BCD `HHMMSS` duration (`[HH, MM, SS]`) to a [`Duration`].
///
/// Returns `None` if any nibble is non-decimal or the minute/second fields
/// exceed 59.
#[must_use]
pub fn decode_bcd_duration(raw: [u8; 3]) -> Option<Duration> {
    let h = u64::from(from_bcd_byte(raw[0])?);
    let m = u64::from(from_bcd_byte(raw[1])?);
    let s = u64::from(from_bcd_byte(raw[2])?);
    if m > 59 || s > 59 {
        return None;
    }
    Some(Duration::from_secs(h * 3600 + m * 60 + s))
}

/// Encode a whole-second [`Duration`] to a 24-bit BCD `HHMMSS` (`[HH, MM, SS]`).
///
/// Sub-second precision is truncated. Returns `None` if the duration is 100
/// hours or longer (`HH` only holds two BCD digits).
#[must_use]
pub fn encode_bcd_duration(duration: Duration) -> Option<[u8; 3]> {
    let secs = duration.as_secs();
    let h = secs / 3600;
    if h > 99 {
        return None;
    }
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    Some([
        to_bcd_byte(h as u8)?,
        to_bcd_byte(m as u8)?,
        to_bcd_byte(s as u8)?,
    ])
}

/// Convert a 16-bit Modified Julian Date to `(year, month, day)`.
///
/// Inverse of [`ymd_to_mjd`]; MJD→calendar per ETSI EN 300 468 Annex C.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn mjd_to_ymd(mjd: u16) -> (i32, u32, u32) {
    let mjd = i64::from(mjd);
    let y_prime = ((mjd as f64 - 15_078.2) / 365.25) as i64;
    let m_prime = ((mjd as f64 - 14_956.1 - (y_prime as f64 * 365.25).floor()) / 30.6001) as i64;
    let d = mjd
        - 14_956
        - (y_prime as f64 * 365.25).floor() as i64
        - (m_prime as f64 * 30.6001).floor() as i64;
    let k = i64::from(m_prime == 14 || m_prime == 15);
    let y = y_prime + k + 1900;
    let m = m_prime - 1 - k * 12;
    (y as i32, m as u32, d as u32)
}

/// Convert a `(year, month, day)` date to a 16-bit Modified Julian Date.
///
/// Forward of [`mjd_to_ymd`], calendar→MJD per ETSI EN 300 468 Annex C. Returns
/// `None` if the field is out of range or the date is not representable in a
/// 16-bit MJD.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn ymd_to_mjd(year: i32, month: u32, day: u32) -> Option<u16> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let l = if month <= 2 { 1.0 } else { 0.0 };
    let y = f64::from(year - 1900);
    let m = f64::from(month);
    let mjd = 14_956.0
        + f64::from(day)
        + ((y - l) * 365.25).floor()
        + ((m + 1.0 + l * 12.0) * 30.6001).floor();
    if (0.0..=f64::from(u16::MAX)).contains(&mjd) {
        Some(mjd as u16)
    } else {
        None
    }
}

/// Decode a 5-byte DVB UTC time (16-bit MJD + 24-bit BCD `HHMMSS`) to a
/// [`chrono::DateTime<chrono::Utc>`].
///
/// Returns `None` if the BCD nibbles are out of range or the date/time is
/// invalid.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn decode_mjd_bcd_utc(raw: [u8; 5]) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
    let mjd = u16::from_be_bytes([raw[0], raw[1]]);
    let (y, m, d) = mjd_to_ymd(mjd);
    let h = from_bcd_byte(raw[2])?;
    let mi = from_bcd_byte(raw[3])?;
    let s = from_bcd_byte(raw[4])?;
    let date = NaiveDate::from_ymd_opt(y, m, d)?;
    let time = NaiveTime::from_hms_opt(u32::from(h), u32::from(mi), u32::from(s))?;
    chrono::Utc
        .from_local_datetime(&NaiveDateTime::new(date, time))
        .single()
}

/// Encode a [`chrono::DateTime<chrono::Utc>`] to a 5-byte DVB UTC time
/// (16-bit MJD + 24-bit BCD `HHMMSS`).
///
/// Sub-second precision is truncated. Returns `None` if the date is not
/// representable in a 16-bit MJD.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn encode_mjd_bcd_utc(dt: chrono::DateTime<chrono::Utc>) -> Option<[u8; 5]> {
    use chrono::{Datelike, Timelike};
    let naive = dt.naive_utc();
    let mjd = ymd_to_mjd(naive.year(), naive.month(), naive.day())?;
    let [m0, m1] = mjd.to_be_bytes();
    Some([
        m0,
        m1,
        to_bcd_byte(naive.hour() as u8)?,
        to_bcd_byte(naive.minute() as u8)?,
        to_bcd_byte(naive.second() as u8)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_round_trips() {
        for &(h, m, s) in &[(0u64, 0u64, 0u64), (1, 30, 45), (99, 59, 59), (2, 0, 0)] {
            let secs = h * 3600 + m * 60 + s;
            let raw = encode_bcd_duration(Duration::from_secs(secs)).expect("encodes");
            assert_eq!(decode_bcd_duration(raw), Some(Duration::from_secs(secs)));
        }
    }

    #[test]
    fn duration_decode_known_vector() {
        // 0x01 0x30 0x45 = 01:30:45 = 5445 s.
        assert_eq!(
            decode_bcd_duration([0x01, 0x30, 0x45]),
            Some(Duration::from_secs(5445))
        );
    }

    #[test]
    fn duration_rejects_over_99h_and_bad_fields() {
        assert_eq!(encode_bcd_duration(Duration::from_secs(100 * 3600)), None);
        assert_eq!(decode_bcd_duration([0x01, 0x75, 0x00]), None); // 75 minutes
        assert_eq!(decode_bcd_duration([0x01, 0x00, 0x1A]), None); // bad nibble
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn ymd_to_mjd_matches_chrono_epoch_arithmetic() {
        use chrono::NaiveDate;
        // MJD epoch is 1858-11-17.
        let epoch = NaiveDate::from_ymd_opt(1858, 11, 17).unwrap();
        for &(y, m, d) in &[(1993, 10, 13), (2000, 1, 1), (2023, 6, 8), (1900, 3, 1)] {
            let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
            let expected = (date - epoch).num_days() as u16;
            assert_eq!(ymd_to_mjd(y, m, d), Some(expected), "{y}-{m}-{d}");
        }
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn mjd_ymd_round_trips() {
        for mjd in [40_587u16, 49_273, 51_544, 59_945, 60_000] {
            let (y, m, d) = mjd_to_ymd(mjd);
            assert_eq!(ymd_to_mjd(y, m, d), Some(mjd), "mjd {mjd}");
        }
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn utc_round_trips() {
        let raw = [0xE4, 0x09, 0x12, 0x34, 0x56];
        let dt = decode_mjd_bcd_utc(raw).expect("decodes");
        assert_eq!(encode_mjd_bcd_utc(dt), Some(raw));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn utc_decode_known_vector() {
        use chrono::{Datelike, Timelike};
        // MJD 0xE409 = 58377, BCD 12:34:56.
        let dt = decode_mjd_bcd_utc([0xE4, 0x09, 0x12, 0x34, 0x56]).expect("decodes");
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (12, 34, 56));
        assert_eq!(dt.year(), 2018);
    }

    #[test]
    fn mjd_bcd_round_trips() {
        for &(y, m, d, h, mi, s) in &[
            (2023u16, 1u8, 1u8, 12u8, 34u8, 56u8),
            (2000, 1, 1, 0, 0, 0),
            (2023, 6, 8, 23, 59, 59),
        ] {
            let dt = MjdBcdDateTime {
                year: y,
                month: m,
                day: d,
                hour: h,
                minute: mi,
                second: s,
            };
            let raw = encode_mjd_bcd(dt).expect("encodes");
            let re = decode_mjd_bcd(raw).expect("decodes");
            assert_eq!(re, dt);
        }
    }

    #[test]
    fn mjd_bcd_rejects_invalid_bcd() {
        assert_eq!(decode_mjd_bcd([0xE4, 0x09, 0x1A, 0x34, 0x56]), None);
        assert_eq!(decode_mjd_bcd([0xE4, 0x09, 0x12, 0x75, 0x56]), None);
    }

    #[test]
    fn mjd_bcd_matches_chrono_when_available() {
        let raw = [0xE4, 0x09, 0x12, 0x34, 0x56];
        let plain = decode_mjd_bcd(raw).expect("decodes");
        #[cfg(feature = "chrono")]
        {
            use chrono::{Datelike, Timelike};
            let chrono_dt = decode_mjd_bcd_utc(raw).expect("decodes");
            assert_eq!(plain.year as i32, chrono_dt.year());
            assert_eq!(plain.month as u32, chrono_dt.month());
            assert_eq!(plain.day as u32, chrono_dt.day());
            assert_eq!(plain.hour as u32, chrono_dt.hour());
            assert_eq!(plain.minute as u32, chrono_dt.minute());
            assert_eq!(plain.second as u32, chrono_dt.second());
        }
        // Even without chrono, the plain decode must produce 2018-09-16 12:34:56.
        assert_eq!(plain.year, 2018);
        assert_eq!(plain.month, 9);
        assert_eq!(plain.day, 16);
    }
}
