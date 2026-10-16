//! UTC time and duration codecs for DVB wire fields.
//!
//! DVB carries wall-clock time as a 16-bit Modified Julian Date plus 24-bit BCD
//! HHMMSS (EN 300 468 Annex C), and event durations as 24-bit BCD HHMMSS. The
//! duration codec is dependency-free; the MJD↔calendar conversion needs a date
//! library and so lives behind the `chrono` feature.
//!
//! # 2038 MJD wrap-around (EN 300 468 v1.20.1 clauses 5.2.4 / 5.2.5 / 5.2.6)
//!
//! The wire field carries only the 16 least-significant bits of the Modified
//! Julian Date. v1.20.1 defines how to recover the full MJD: if the most
//! significant bit of those 16 bits is one the 16 bits *are* the MJD (32768 =
//! 1948-08-05 ... 65535 = 2038-04-22), otherwise the MJD is `65536 (0x10000)`
//! plus the 16 bits (65536 = 2038-04-23 ... 98303 = 2128-01-09). Dates before
//! MJD 32768 are therefore no longer representable; `25 December 2040
//! 12:34:56` is coded `0x03D1123456`.

use crate::bcd::{from_bcd_byte, to_bcd_byte};
use core::time::Duration;

/// Most significant bit of the 16-bit MJD field (EN 300 468 v1.20.1 clause
/// 5.2.4): set means the field is the whole MJD, clear means the MJD is
/// [`MJD_WRAP_OFFSET`] plus the field.
const MJD_16_MSB: u16 = 0x8000;

/// MJD offset added when the MSB of the 16-bit MJD field is zero (`0x10000`,
/// EN 300 468 v1.20.1 clause 5.2.4).
const MJD_WRAP_OFFSET: u32 = 0x1_0000;

/// Recover the full MJD from the 16-bit wire field (v1.20.1 clause 5.2.4).
fn full_mjd(field: u16) -> u32 {
    if field & MJD_16_MSB != 0 {
        u32::from(field)
    } else {
        MJD_WRAP_OFFSET + u32::from(field)
    }
}

/// Narrow a full MJD to the 16-bit wire field; `None` outside the
/// representable `0x8000 ..= 0x1_7FFF` range.
fn mjd_field(full: u32) -> Option<u16> {
    let field = if full >= MJD_WRAP_OFFSET {
        full - MJD_WRAP_OFFSET
    } else {
        full
    };
    let field = u16::try_from(field).ok()?;
    (full_mjd(field) == full).then_some(field)
}

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
/// is proleptic Gregorian (EN 300 468 Annex C's formula is only valid to 2100-02-28; see `mjd_to_ymd_core`).
#[must_use]
pub fn decode_mjd_bcd(raw: [u8; 5]) -> Option<MjdBcdDateTime> {
    let (mjd_bytes, _) = raw.split_first_chunk::<2>().unwrap();
    let mjd = u16::from_be_bytes(*mjd_bytes);
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
/// Returns `None` if any field is out of the representable range. Hour/minute/
/// second are range-checked against the same bounds [`decode_mjd_bcd`] enforces
/// (0–23 / 0–59 / 0–59) — `to_bcd_byte` alone would accept e.g. `90` (it only
/// rejects values above 99), producing bytes `decode_mjd_bcd` then rejects.
#[must_use]
pub fn encode_mjd_bcd(dt: MjdBcdDateTime) -> Option<[u8; 5]> {
    if dt.hour > 23 || dt.minute > 59 || dt.second > 59 {
        return None;
    }
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

/// Julian Day Number of Modified Julian Date 0 (1858-11-17): `JD = MJD +
/// 2_400_000.5`, and the integer day number of that civil day is one more.
const JDN_OF_MJD_0: i64 = 2_400_001;

/// Proleptic-Gregorian MJD→calendar conversion in pure integer arithmetic
/// (Richards' Julian-Day-Number algorithm), valid over the **whole**
/// representable range `0x8000 ..= 0x1_7FFF` (1948-08-05 ..= 2128-01-09).
///
/// EN 300 468 Annex C's closed-form formula is only valid 1900-03-01 ..=
/// 2100-02-28 (its NOTE 2: it treats 2100 as a leap year), which the v1.20.1
/// 2038 wrap rule outgrows; this conversion applies the full Gregorian leap
/// rule (divisible by 4, except centuries not divisible by 400). Both
/// [`mjd_to_ymd_nogate`] and the chrono-gated `mjd_to_ymd` share it.
fn mjd_to_ymd_core(field: u16) -> (i64, i64, i64) {
    const Y_BASE: i64 = 4716;
    const J_OFFSET: i64 = 1401;
    const B_OFFSET: i64 = 274_277;
    const C_CORR: i64 = -38;
    let jdn = i64::from(full_mjd(field)) + JDN_OF_MJD_0;
    let f = jdn + J_OFFSET + (((4 * jdn + B_OFFSET) / 146_097) * 3) / 4 + C_CORR;
    let e = 4 * f + 3;
    let g = (e % 1461) / 4;
    let h = 5 * g + 2;
    let d = (h % 153) / 5 + 1;
    let m = (h / 153 + 2) % 12 + 1;
    let y = e / 1461 - Y_BASE + (12 + 2 - m) / 12;
    (y, m, d)
}

/// Convert a 16-bit Modified Julian Date to `(year, month, day)`.
///
/// MJD→calendar per ETSI EN 300 468 Annex C. This is the dependency-free
/// version of the chrono-gated [`mjd_to_ymd`].
fn mjd_to_ymd_nogate(mjd: u16) -> Option<(u16, u8, u8)> {
    let (y, m, d) = mjd_to_ymd_core(mjd);
    let y_u16 = u16::try_from(y).ok()?;
    let m_u8 = u8::try_from(m).ok()?;
    let d_u8 = u8::try_from(d).ok()?;
    if !(1..=12).contains(&m_u8) || !(1..=31).contains(&d_u8) {
        return None;
    }
    Some((y_u16, m_u8, d_u8))
}

/// Proleptic-Gregorian calendar→full-MJD conversion in pure integer
/// arithmetic (Fliegel & Van Flandern Julian-Day-Number algorithm); the
/// inverse of [`mjd_to_ymd_core`], valid over the whole range (see there).
/// Total over `day` 1..=31 (e.g. `31 Feb` lands on March); callers validate
/// by round-tripping.
fn ymd_to_mjd_core(year: i32, month: u32, day: u32) -> i64 {
    let a = (14 - i64::from(month)) / 12;
    let y = i64::from(year) + 4800 - a;
    let m = i64::from(month) + 12 * a - 3;
    let jdn = i64::from(day) + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32_045;
    jdn - JDN_OF_MJD_0
}

/// Convert a `(year, month, day)` date to a 16-bit Modified Julian Date.
///
/// Calendar→MJD per ETSI EN 300 468 Annex C. This is the dependency-free
/// version of the chrono-gated [`ymd_to_mjd`].
fn ymd_to_mjd_nogate(year: i32, month: u32, day: u32) -> Option<u16> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let full = ymd_to_mjd_core(year, month, day);
    let mjd = mjd_field(u32::try_from(full).ok()?)?;
    // The core arithmetic is total over day 1..=31, so `31 Feb` silently
    // lands on 2 or 3 March. Accept only dates that decode back to the
    // same calendar day (`encode_mjd_bcd` must not accept what decode
    // would not return).
    let (y, m, d) = mjd_to_ymd_nogate(mjd)?;
    if i32::from(y) == year && u32::from(m) == month && u32::from(d) == day {
        Some(mjd)
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

/// Convert the 16-bit MJD wire field (16 lsb of the MJD, with the v1.20.1
/// 2038 wrap rule) to `(year, month, day)`.
///
/// Inverse of [`ymd_to_mjd`]; MJD→calendar per ETSI EN 300 468 Annex C.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn mjd_to_ymd(mjd: u16) -> (i32, u32, u32) {
    let (y, m, d) = mjd_to_ymd_core(mjd);
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
    ymd_to_mjd_nogate(year, month, day)
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
    let (mjd_bytes, _) = raw.split_first_chunk::<2>().unwrap();
    let mjd = u16::from_be_bytes(*mjd_bytes);
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

/// Unix timestamp (seconds since Unix epoch 1970-01-01T00:00:00 UTC) of the
/// T2-MI / DVB time-of-day epoch: 2000-01-01T00:00:00 UTC.
///
/// Computed as: days from 1970-01-01 to 2000-01-01 = 10957 days
/// (30 years, 8 of which are leap years: 1972, 1976, …, 1996 → 365×30 + 8 = 10958 - 1 day;
/// precise count 1970→2000 is 10957 days). 10957 × 86400 = 946684800.
pub const SECS_2000_EPOCH: i64 = 946_684_800;

/// Decode a T2-MI `seconds_since_2000` / subsecond-nanoseconds pair to a
/// [`chrono::DateTime<chrono::Utc>`], applying a `utco` leap-second offset.
///
/// The T2-MI spec (ETSI TS 102 773 §5.2.7) defines:
/// - `seconds_since_2000`: whole seconds since 2000-01-01T00:00:00 UTC (the
///   timestamp's own time base, not civil UTC).
/// - `utco`: the number of leap seconds to **subtract** from the timestamp's
///   time base to obtain civil UTC. At publication of the spec this was 34 s.
/// - `subsec_nanos`: sub-second component in nanoseconds (caller converts from
///   the bandwidth-dependent `subseconds` field).
///
/// Returns `None` if the resulting timestamp is out of range.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn decode_seconds_since_2000_utc(
    seconds_since_2000: u64,
    subsec_nanos: u32,
    utco: u16,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    // Convert to Unix timestamp: add the 2000-epoch offset, subtract utco.
    let unix_secs = SECS_2000_EPOCH
        .checked_add(i64::try_from(seconds_since_2000).ok()?)?
        .checked_sub(i64::from(utco))?;
    chrono::Utc.timestamp_opt(unix_secs, subsec_nanos).single()
}

/// Encode a [`chrono::DateTime<chrono::Utc>`] to `seconds_since_2000` with a
/// `utco` leap-second offset.
///
/// Inverse of [`decode_seconds_since_2000_utc`]. Returns `None` if the date
/// is before the 2000 epoch or `seconds_since_2000` would not fit in 40 bits.
#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[must_use]
pub fn encode_seconds_since_2000_utc(
    dt: chrono::DateTime<chrono::Utc>,
    utco: u16,
) -> Option<(u64, u32)> {
    use chrono::Timelike;
    let unix_secs = dt.timestamp();
    // seconds_since_2000 = (unix_secs - SECS_2000_EPOCH) + utco
    let raw = unix_secs
        .checked_sub(SECS_2000_EPOCH)?
        .checked_add(i64::from(utco))?;
    let secs = u64::try_from(raw).ok()?;
    // 40-bit field maximum
    if secs > 0xFF_FFFF_FFFF {
        return None;
    }
    Some((secs, dt.nanosecond()))
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
        for &(y, m, d) in &[(1993, 10, 13), (2000, 1, 1), (2023, 6, 8), (1949, 1, 23)] {
            let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
            let full = (date - epoch).num_days() as u32;
            let expected = u16::try_from(if full >= 0x1_0000 {
                full - 0x1_0000
            } else {
                full
            })
            .unwrap();
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

    /// EN 300 468 v1.20.1 clause 5.2.4 EXAMPLE 3: 25 December 2040 12:34:56
    /// is coded `0x03 D112 3456` (MSB of the 16-bit field is zero, so the MJD
    /// is 0x10000 + 0x03D1).
    #[test]
    fn spec_v1_20_1_2040_example() {
        let dt = MjdBcdDateTime {
            year: 2040,
            month: 12,
            day: 25,
            hour: 12,
            minute: 34,
            second: 56,
        };
        let raw = [0x03, 0xD1, 0x12, 0x34, 0x56];
        assert_eq!(decode_mjd_bcd(raw), Some(dt));
        assert_eq!(encode_mjd_bcd(dt), Some(raw));
        // Clause 5.2.5 EXAMPLE 1 (pre-wrap) still decodes: 93/10/13 12:45:00.
        let old = MjdBcdDateTime {
            year: 1993,
            month: 10,
            day: 13,
            hour: 12,
            minute: 45,
            second: 0,
        };
        assert_eq!(decode_mjd_bcd([0xC0, 0x79, 0x12, 0x45, 0x00]), Some(old));
        // Boundary: last pre-wrap day and first post-wrap day.
        let a = decode_mjd_bcd([0xFF, 0xFF, 0x00, 0x00, 0x00]).unwrap();
        let b = decode_mjd_bcd([0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!((a.year, a.month, a.day), (2038, 4, 22));
        assert_eq!((b.year, b.month, b.day), (2038, 4, 23));
        // Pre-1949 dates are no longer representable.
        assert_eq!(
            encode_mjd_bcd(MjdBcdDateTime {
                year: 1900,
                month: 3,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }),
            None
        );
    }

    #[test]
    fn encode_rejects_calendar_impossible_days() {
        let mk = |year, month, day| MjdBcdDateTime {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
            second: 0,
        };
        // 31 Feb / 31 Apr / 29 Feb in a non-leap year would silently encode
        // as a different (later) day.
        assert_eq!(encode_mjd_bcd(mk(2023, 2, 31)), None);
        assert_eq!(encode_mjd_bcd(mk(2023, 4, 31)), None);
        assert_eq!(encode_mjd_bcd(mk(2023, 2, 29)), None);
        assert!(encode_mjd_bcd(mk(2024, 2, 29)).is_some());
        assert!(encode_mjd_bcd(mk(2023, 1, 31)).is_some());
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

    /// r01-W16: `encode_mjd_bcd` must reject hour/minute/second values that
    /// `decode_mjd_bcd` rejects (0-23 / 0-59 / 0-59). Pre-fix, `encode_mjd_bcd`
    /// only bottomed out at `to_bcd_byte`'s `<=99` check, so it happily
    /// produced bytes for hour=90 that decode then refused: `encode_mjd_bcd`
    /// returned `Some(_)` and the round-trip `decode_mjd_bcd(raw)` was `None`,
    /// i.e. not the original value.
    #[test]
    fn mjd_bcd_encode_rejects_out_of_range_time_fields() {
        let base = MjdBcdDateTime {
            year: 2023,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(encode_mjd_bcd(MjdBcdDateTime { hour: 90, ..base }), None);
        assert_eq!(encode_mjd_bcd(MjdBcdDateTime { hour: 24, ..base }), None);
        assert_eq!(encode_mjd_bcd(MjdBcdDateTime { minute: 90, ..base }), None);
        assert_eq!(encode_mjd_bcd(MjdBcdDateTime { minute: 60, ..base }), None);
        assert_eq!(encode_mjd_bcd(MjdBcdDateTime { second: 90, ..base }), None);
        assert_eq!(encode_mjd_bcd(MjdBcdDateTime { second: 60, ..base }), None);
        // Boundary values remain accepted.
        assert!(encode_mjd_bcd(MjdBcdDateTime { hour: 23, ..base }).is_some());
        assert!(encode_mjd_bcd(MjdBcdDateTime { minute: 59, ..base }).is_some());
        assert!(encode_mjd_bcd(MjdBcdDateTime { second: 59, ..base }).is_some());
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

    #[cfg(feature = "chrono")]
    #[test]
    fn secs_2000_epoch_is_correct() {
        use chrono::{Datelike, TimeZone, Timelike};
        // SECS_2000_EPOCH must decode to 2000-01-01T00:00:00 UTC.
        let dt = chrono::Utc.timestamp_opt(SECS_2000_EPOCH, 0).unwrap();
        assert_eq!((dt.year(), dt.month(), dt.day()), (2000, 1, 1));
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (0, 0, 0));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn decode_seconds_since_2000_utc_known_value() {
        use chrono::{Datelike, Timelike};
        // seconds_since_2000 = 0, utco = 0 → 2000-01-01T00:00:00 UTC.
        let dt = decode_seconds_since_2000_utc(0, 0, 0).expect("decodes");
        assert_eq!((dt.year(), dt.month(), dt.day()), (2000, 1, 1));
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (0, 0, 0));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn encode_decode_seconds_since_2000_utc_round_trips() {
        use chrono::TimeZone;
        // 2023-06-08T12:34:56 UTC, utco = 37 (current at time of writing).
        let dt = chrono::Utc
            .with_ymd_and_hms(2023, 6, 8, 12, 34, 56)
            .unwrap();
        let (secs, nanos) = encode_seconds_since_2000_utc(dt, 37).expect("encodes");
        let decoded = decode_seconds_since_2000_utc(secs, nanos, 37).expect("decodes");
        assert_eq!(decoded, dt);
    }
}
