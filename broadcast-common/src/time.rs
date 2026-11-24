//! UTC time and duration codecs for DVB wire fields.
//!
//! DVB carries wall-clock time as a 16-bit Modified Julian Date plus 24-bit BCD
//! HHMMSS (EN 300 468 Annex C), and event durations as 24-bit BCD HHMMSS. The
//! duration codec and the MJD↔calendar conversion are dependency-free; only the
//! functions that return or take a [`jiff::Timestamp`] live behind the `jiff`
//! feature.
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

/// Width in bytes of a DVB UTC time (16-bit MJD + 24-bit BCD `HHMMSS`),
/// EN 300 468 Annex C.
pub const MJD_BCD_LEN: usize = 5;
/// Width in bytes of a DVB BCD `HHMMSS` duration, EN 300 468 Annex C.
pub const BCD_DURATION_LEN: usize = 3;
/// Width of the 16-bit MJD field within the 5-byte time.
const MJD_BYTES: usize = 2;
/// Byte offsets of HH, MM, SS within the 5-byte time (after the MJD).
const HOUR_OFFSET: usize = MJD_BYTES;
const MINUTE_OFFSET: usize = HOUR_OFFSET + 1;
const SECOND_OFFSET: usize = MINUTE_OFFSET + 1;
/// Byte offsets of HH, MM, SS within the 3-byte duration.
const DURATION_HOUR: usize = 0;
const DURATION_MINUTE: usize = DURATION_HOUR + 1;
const DURATION_SECOND: usize = DURATION_MINUTE + 1;
/// Field bounds the codec enforces.
const MIN_MONTH: u8 = 1;
const MAX_MONTH: u8 = 12;
const MIN_DAY: u8 = 1;
const MAX_DAY: u8 = 31;
const MAX_HOUR: u8 = 23;
const MAX_MINUTE: u8 = 59;
const MAX_SECOND: u8 = 59;
/// Largest whole-hour duration the two BCD hour digits hold.
const MAX_DURATION_HOUR: u64 = 99;
/// Seconds per minute and per hour.
const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3600;

/// Richards / Fliegel & Van Flandern Julian-Day-Number coefficients.
const JDN_LEAP_YEARS: i64 = 4;
const JDN_CENTURY_YEARS: i64 = 100;
const JDN_LEAP_CENTURY_YEARS: i64 = 400;
const JDN_DAYS_PER_YEAR: i64 = 365;
const JDN_MONTHS_PER_YEAR: i64 = 12;
const JDN_MONTH_DAYS: i64 = 153;
const JDN_MONTH_CORRECTION: i64 = 5;
const JDN_MONTH_BIAS: i64 = 2;
const JDN_GREGORIAN_CORRECTION: i64 = 3;
const JDN_ERA_DAYS: i64 = 146_097;
const JDN_CYCLE_DAYS: i64 = 1461;
const JDN_YEAR_OFFSET: i64 = 14;
const JDN_YEAR_BASE: i64 = 4800;
const JDN_GREGORIAN_OFFSET: i64 = 32_045;

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
/// Unlike `decode_mjd_bcd_utc`, this is dependency-free (no `jiff`
/// feature required). Returns `None` if the BCD nibbles are out of range
/// or the minute/second fields exceed 59. The MJD→calendar conversion
/// is proleptic Gregorian (EN 300 468 Annex C's formula is only valid to 2100-02-28; see `mjd_to_ymd_core`).
#[must_use]
pub fn decode_mjd_bcd(raw: [u8; MJD_BCD_LEN]) -> Option<MjdBcdDateTime> {
    let (mjd_bytes, _) = raw.split_first_chunk::<MJD_BYTES>().unwrap();
    let mjd = u16::from_be_bytes(*mjd_bytes);
    let h = from_bcd_byte(raw[HOUR_OFFSET])?;
    let mi = from_bcd_byte(raw[MINUTE_OFFSET])?;
    let s = from_bcd_byte(raw[SECOND_OFFSET])?;
    if mi > MAX_MINUTE || s > MAX_SECOND || h > MAX_HOUR {
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
pub fn encode_mjd_bcd(dt: MjdBcdDateTime) -> Option<[u8; MJD_BCD_LEN]> {
    if dt.hour > MAX_HOUR || dt.minute > MAX_MINUTE || dt.second > MAX_SECOND {
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
/// [`mjd_to_ymd_nogate`] and [`mjd_to_ymd`] share it.
fn mjd_to_ymd_core(field: u16) -> (i64, i64, i64) {
    const Y_BASE: i64 = 4716;
    const J_OFFSET: i64 = 1401;
    const B_OFFSET: i64 = 274_277;
    const C_CORR: i64 = -38;
    let jdn = i64::from(full_mjd(field)) + JDN_OF_MJD_0;
    let f = jdn
        + J_OFFSET
        + (((JDN_LEAP_YEARS * jdn + B_OFFSET) / JDN_ERA_DAYS) * JDN_GREGORIAN_CORRECTION)
            / JDN_LEAP_YEARS
        + C_CORR;
    let e = JDN_LEAP_YEARS * f + JDN_GREGORIAN_CORRECTION;
    let g = (e % JDN_CYCLE_DAYS) / JDN_LEAP_YEARS;
    let h = JDN_MONTH_CORRECTION * g + JDN_MONTH_BIAS;
    let d = (h % JDN_MONTH_DAYS) / JDN_MONTH_CORRECTION + 1;
    let m = (h / JDN_MONTH_DAYS + JDN_MONTH_BIAS) % JDN_MONTHS_PER_YEAR + 1;
    let y = e / JDN_CYCLE_DAYS - Y_BASE
        + (JDN_MONTHS_PER_YEAR + JDN_MONTH_BIAS - m) / JDN_MONTHS_PER_YEAR;
    (y, m, d)
}

/// Convert a 16-bit Modified Julian Date to `(year, month, day)`.
///
/// MJD→calendar per ETSI EN 300 468 Annex C, narrowed to the
/// [`MjdBcdDateTime`] field widths.
fn mjd_to_ymd_nogate(mjd: u16) -> Option<(u16, u8, u8)> {
    let (y, m, d) = mjd_to_ymd_core(mjd);
    let y_u16 = u16::try_from(y).ok()?;
    let m_u8 = u8::try_from(m).ok()?;
    let d_u8 = u8::try_from(d).ok()?;
    if !(MIN_MONTH..=MAX_MONTH).contains(&m_u8) || !(MIN_DAY..=MAX_DAY).contains(&d_u8) {
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
    let a = (JDN_YEAR_OFFSET - i64::from(month)) / JDN_MONTHS_PER_YEAR;
    let y = i64::from(year) + JDN_YEAR_BASE - a;
    let m = i64::from(month) + JDN_MONTHS_PER_YEAR * a - JDN_GREGORIAN_CORRECTION;
    let jdn = i64::from(day)
        + (JDN_MONTH_DAYS * m + JDN_MONTH_BIAS) / JDN_MONTH_CORRECTION
        + JDN_DAYS_PER_YEAR * y
        + y / JDN_LEAP_YEARS
        - y / JDN_CENTURY_YEARS
        + y / JDN_LEAP_CENTURY_YEARS
        - JDN_GREGORIAN_OFFSET;
    jdn - JDN_OF_MJD_0
}

/// Convert a `(year, month, day)` date to a 16-bit Modified Julian Date.
///
/// Calendar→MJD per ETSI EN 300 468 Annex C; [`ymd_to_mjd`] is its public
/// face.
fn ymd_to_mjd_nogate(year: i32, month: u32, day: u32) -> Option<u16> {
    if !(u32::from(MIN_MONTH)..=u32::from(MAX_MONTH)).contains(&month)
        || !(u32::from(MIN_DAY)..=u32::from(MAX_DAY)).contains(&day)
    {
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
pub fn decode_bcd_duration(raw: [u8; BCD_DURATION_LEN]) -> Option<Duration> {
    let h = u64::from(from_bcd_byte(raw[DURATION_HOUR])?);
    let m = u64::from(from_bcd_byte(raw[DURATION_MINUTE])?);
    let s = u64::from(from_bcd_byte(raw[DURATION_SECOND])?);
    if m > u64::from(MAX_MINUTE) || s > u64::from(MAX_SECOND) {
        return None;
    }
    Some(Duration::from_secs(
        h * SECONDS_PER_HOUR + m * SECONDS_PER_MINUTE + s,
    ))
}

/// Encode a whole-second [`Duration`] to a 24-bit BCD `HHMMSS` (`[HH, MM, SS]`).
///
/// Sub-second precision is truncated. Returns `None` if the duration is 100
/// hours or longer (`HH` only holds two BCD digits).
#[must_use]
pub fn encode_bcd_duration(duration: Duration) -> Option<[u8; BCD_DURATION_LEN]> {
    let secs = duration.as_secs();
    let h = secs / SECONDS_PER_HOUR;
    if h > MAX_DURATION_HOUR {
        return None;
    }
    let m = (secs % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let s = secs % SECONDS_PER_MINUTE;
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
#[must_use]
pub fn ymd_to_mjd(year: i32, month: u32, day: u32) -> Option<u16> {
    ymd_to_mjd_nogate(year, month, day)
}

/// Decode a 5-byte DVB UTC time (16-bit MJD + 24-bit BCD `HHMMSS`) to a
/// [`jiff::Timestamp`].
///
/// Returns `None` if the BCD nibbles are out of range or the date/time is
/// invalid.
#[cfg(feature = "jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
#[must_use]
pub fn decode_mjd_bcd_utc(raw: [u8; MJD_BCD_LEN]) -> Option<jiff::Timestamp> {
    let [m0, m1, h, mi, s] = raw;
    let (y, m, d) = mjd_to_ymd(u16::from_be_bytes([m0, m1]));
    let dt = jiff::civil::DateTime::new(
        i16::try_from(y).ok()?,
        i8::try_from(m).ok()?,
        i8::try_from(d).ok()?,
        i8::try_from(from_bcd_byte(h)?).ok()?,
        i8::try_from(from_bcd_byte(mi)?).ok()?,
        i8::try_from(from_bcd_byte(s)?).ok()?,
        0,
    )
    .ok()?;
    jiff::tz::TimeZone::UTC.to_timestamp(dt).ok()
}

/// Encode a [`jiff::Timestamp`] to a 5-byte DVB UTC time (16-bit MJD + 24-bit
/// BCD `HHMMSS`).
///
/// Sub-second precision is truncated. Returns `None` if the date is not
/// representable in a 16-bit MJD.
#[cfg(feature = "jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
#[must_use]
pub fn encode_mjd_bcd_utc(ts: jiff::Timestamp) -> Option<[u8; MJD_BCD_LEN]> {
    let dt = jiff::tz::TimeZone::UTC.to_datetime(ts);
    let mjd = ymd_to_mjd(
        i32::from(dt.year()),
        u32::try_from(dt.month()).ok()?,
        u32::try_from(dt.day()).ok()?,
    )?;
    let [m0, m1] = mjd.to_be_bytes();
    Some([
        m0,
        m1,
        to_bcd_byte(u8::try_from(dt.hour()).ok()?)?,
        to_bcd_byte(u8::try_from(dt.minute()).ok()?)?,
        to_bcd_byte(u8::try_from(dt.second()).ok()?)?,
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
/// [`jiff::Timestamp`], applying a `utco` leap-second offset.
///
/// The T2-MI spec (ETSI TS 102 773 §5.2.7) defines:
/// - `seconds_since_2000`: whole seconds since 2000-01-01T00:00:00 UTC (the
///   timestamp's own time base, not civil UTC).
/// - `utco`: the number of leap seconds to **subtract** from the timestamp's
///   time base to obtain civil UTC. At publication of the spec this was 34 s.
/// - `subsec_nanos`: sub-second component in nanoseconds (caller converts from
///   the bandwidth-dependent `subseconds` field).
///
/// Returns `None` if the resulting timestamp is out of range or
/// `subsec_nanos` is a whole second or more.
#[cfg(feature = "jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
#[must_use]
pub fn decode_seconds_since_2000_utc(
    seconds_since_2000: u64,
    subsec_nanos: u32,
    utco: u16,
) -> Option<jiff::Timestamp> {
    let unix_secs = SECS_2000_EPOCH
        .checked_add(i64::try_from(seconds_since_2000).ok()?)?
        .checked_sub(i64::from(utco))?;
    jiff::Timestamp::new(unix_secs, i32::try_from(subsec_nanos).ok()?).ok()
}

/// Largest `seconds_since_2000` the 40-bit T2-MI field holds (ETSI TS 102 773
/// §5.2.7).
#[cfg(feature = "jiff")]
const SECONDS_SINCE_2000_MAX: u64 = (1 << 40) - 1;

/// Encode a [`jiff::Timestamp`] to `seconds_since_2000` with a `utco`
/// leap-second offset.
///
/// Inverse of [`decode_seconds_since_2000_utc`]. Returns `None` if the date
/// is before the 2000 epoch or `seconds_since_2000` would not fit in 40 bits.
#[cfg(feature = "jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
#[must_use]
pub fn encode_seconds_since_2000_utc(ts: jiff::Timestamp, utco: u16) -> Option<(u64, u32)> {
    let raw = ts
        .as_second()
        .checked_sub(SECS_2000_EPOCH)?
        .checked_add(i64::from(utco))?;
    let secs = u64::try_from(raw).ok()?;
    if secs > SECONDS_SINCE_2000_MAX {
        return None;
    }
    Some((secs, u32::try_from(ts.subsec_nanosecond()).ok()?))
}

/// A pure (no clock read) mapping between a monotonic microsecond counter and
/// UTC, anchored by one observed pair: at monotonic [`mono`](Self::mono) the
/// UTC clock read [`utc`](Self::utc). Maps in both directions with wrap-safe
/// `i128` arithmetic, saturating at the range of each output type. The caller
/// supplies the anchor (this type never reads a clock).
#[cfg(feature = "jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonoUtc {
    /// Monotonic instant at the anchor.
    pub mono: crate::stage::Timestamp,
    /// UTC instant at the anchor.
    pub utc: jiff::Timestamp,
}

#[cfg(feature = "jiff")]
impl MonoUtc {
    /// Build an anchor from a sampled pair.
    pub fn new(mono: crate::stage::Timestamp, utc: jiff::Timestamp) -> Self {
        Self { mono, utc }
    }

    /// UTC at monotonic instant `now`, saturating at the wall-clock range.
    #[must_use]
    pub fn utc_at(&self, now: crate::stage::Timestamp) -> jiff::Timestamp {
        let delta = i128::from(now.as_micros()) - i128::from(self.mono.as_micros());
        let us = i128::from(self.utc.as_microsecond()) + delta;
        let us = us.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
        let us = i64::try_from(us).unwrap_or(i64::MAX);
        jiff::Timestamp::from_microsecond(us).unwrap_or(if us < 0 {
            jiff::Timestamp::MIN
        } else {
            jiff::Timestamp::MAX
        })
    }

    /// Monotonic instant at which UTC reaches `utc`, saturating at the
    /// monotonic range.
    #[must_use]
    pub fn mono_for_utc(&self, utc: jiff::Timestamp) -> crate::stage::Timestamp {
        let delta = i128::from(utc.as_microsecond()) - i128::from(self.utc.as_microsecond());
        let mono = i128::from(self.mono.as_micros()) + delta;
        let mono = mono.clamp(0, i128::from(u64::MAX));
        crate::stage::Timestamp::from_micros(u64::try_from(mono).unwrap_or(u64::MAX))
    }
}

/// Alias kept for callers that name the mapping a `WallClock`.
#[cfg(feature = "jiff")]
pub type WallClock = MonoUtc;
/// Alias kept for callers that name the mapping an `Anchor`.
#[cfg(feature = "jiff")]
pub type Anchor = MonoUtc;

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

    #[cfg(feature = "jiff")]
    #[test]
    fn ymd_to_mjd_matches_jiff_epoch_arithmetic() {
        use jiff::civil::date;
        // MJD epoch is 1858-11-17.
        let epoch = date(1858, 11, 17);
        for &(y, m, d) in &[(1993, 10, 13), (2000, 1, 1), (2023, 6, 8), (1949, 1, 23)] {
            let day = date(y as i16, m as i8, d as i8);
            let full = (day - epoch).get_days() as u32;
            let expected = u16::try_from(if full >= 0x1_0000 {
                full - 0x1_0000
            } else {
                full
            })
            .unwrap();
            assert_eq!(ymd_to_mjd(y, m, d), Some(expected), "{y}-{m}-{d}");
        }
    }

    #[test]
    fn mjd_ymd_round_trips() {
        for mjd in [40_587u16, 49_273, 51_544, 59_945, 60_000] {
            let (y, m, d) = mjd_to_ymd(mjd);
            assert_eq!(ymd_to_mjd(y, m, d), Some(mjd), "mjd {mjd}");
        }
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn utc_round_trips() {
        let raw = [0xE4, 0x09, 0x12, 0x34, 0x56];
        let dt = decode_mjd_bcd_utc(raw).expect("decodes");
        assert_eq!(encode_mjd_bcd_utc(dt), Some(raw));
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn utc_decode_known_vector() {
        // MJD 0xE409 = 58377, BCD 12:34:56.
        let ts = decode_mjd_bcd_utc([0xE4, 0x09, 0x12, 0x34, 0x56]).expect("decodes");
        let dt = jiff::tz::TimeZone::UTC.to_datetime(ts);
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (12, 34, 56));
        assert_eq!(dt.year(), 2018);
        assert_eq!(ts.as_second(), 1_537_101_296);
        assert_eq!(decode_mjd_bcd_utc([0xE4, 0x09, 0x24, 0x00, 0x00]), None);
        assert_eq!(decode_mjd_bcd_utc([0xE4, 0x09, 0x12, 0x60, 0x00]), None);
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
    fn mjd_bcd_matches_jiff_when_available() {
        let raw = [0xE4, 0x09, 0x12, 0x34, 0x56];
        let plain = decode_mjd_bcd(raw).expect("decodes");
        #[cfg(feature = "jiff")]
        {
            let dt = jiff::tz::TimeZone::UTC.to_datetime(decode_mjd_bcd_utc(raw).expect("decodes"));
            assert_eq!(plain.year as i16, dt.year());
            assert_eq!(plain.month as i8, dt.month());
            assert_eq!(plain.day as i8, dt.day());
            assert_eq!(plain.hour as i8, dt.hour());
            assert_eq!(plain.minute as i8, dt.minute());
            assert_eq!(plain.second as i8, dt.second());
        }
        // Even without jiff, the plain decode must produce 2018-09-16 12:34:56.
        assert_eq!(plain.year, 2018);
        assert_eq!(plain.month, 9);
        assert_eq!(plain.day, 16);
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn secs_2000_epoch_is_correct() {
        // SECS_2000_EPOCH must decode to 2000-01-01T00:00:00 UTC.
        let dt = jiff::tz::TimeZone::UTC
            .to_datetime(jiff::Timestamp::from_second(SECS_2000_EPOCH).unwrap());
        assert_eq!((dt.year(), dt.month(), dt.day()), (2000, 1, 1));
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (0, 0, 0));
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn decode_seconds_since_2000_utc_known_value() {
        // seconds_since_2000 = 0, utco = 0 → 2000-01-01T00:00:00 UTC.
        let ts = decode_seconds_since_2000_utc(0, 0, 0).expect("decodes");
        assert_eq!(ts.as_second(), SECS_2000_EPOCH);
        let dt = jiff::tz::TimeZone::UTC.to_datetime(ts);
        assert_eq!((dt.year(), dt.month(), dt.day()), (2000, 1, 1));
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (0, 0, 0));
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn mono_utc_maps_both_directions() {
        let mono = crate::stage::Timestamp::from_micros(1_000);
        let utc = jiff::Timestamp::from_microsecond(5).unwrap();
        let m = MonoUtc::new(mono, utc);
        assert_eq!(m.utc_at(crate::stage::Timestamp::from_micros(1_000)), utc);
        assert_eq!(
            m.utc_at(crate::stage::Timestamp::from_micros(1_005)),
            jiff::Timestamp::from_microsecond(10).unwrap()
        );
        assert_eq!(m.mono_for_utc(utc), mono);
        assert_eq!(
            m.mono_for_utc(jiff::Timestamp::from_microsecond(10).unwrap()),
            crate::stage::Timestamp::from_micros(1_005)
        );
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn mono_utc_before_anchor_is_negative() {
        let mono = crate::stage::Timestamp::from_micros(1_000);
        let utc = jiff::Timestamp::from_microsecond(5).unwrap();
        let m = MonoUtc::new(mono, utc);
        assert_eq!(
            m.utc_at(crate::stage::Timestamp::from_micros(995)),
            jiff::Timestamp::from_microsecond(0).unwrap()
        );
        assert!(m.utc_at(crate::stage::Timestamp::from_micros(995)) < m.utc);
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn encode_decode_seconds_since_2000_utc_round_trips() {
        // 2023-06-08T12:34:56 UTC, utco = 37 (current at time of writing).
        let dt = jiff::tz::TimeZone::UTC
            .to_timestamp(jiff::civil::date(2023, 6, 8).at(12, 34, 56, 0))
            .unwrap();
        let (secs, nanos) = encode_seconds_since_2000_utc(dt, 37).expect("encodes");
        let decoded = decode_seconds_since_2000_utc(secs, nanos, 37).expect("decodes");
        assert_eq!(decoded, dt);
    }
}
