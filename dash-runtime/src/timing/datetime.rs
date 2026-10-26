//! `xs:dateTime` (accepted subset), HTTP `Date` and NTP timestamp readers.
//!
//! The accepted `xs:dateTime` subset is project policy, recorded with its
//! rationale and the full accept/reject table in
//! `docs/specs/dash/xs-datetime-subset.md` (W3C XML Schema Part 2, 3.2.7;
//! ISO/IEC 23009-1 5.8.5.7 `DIRECT-VALUE`). Calendar arithmetic is jiff's;
//! nothing here reads a clock.

use jiff::Span;
use jiff::civil::{Date, DateTime};
use jiff::tz::{Offset, TimeZone};
use thiserror::Error;

const MICROS_PER_SECOND: i64 = 1_000_000;
const NANOS_DIGITS: usize = 9;
const MICROS_DIGITS: usize = 6;
const SECONDS_PER_HOUR: i32 = 3_600;
const SECONDS_PER_MINUTE: i32 = 60;
const MAX_ZONE_HOUR: u8 = 14;
const MAX_MINUTE: u8 = 59;
const MAX_SECOND: u8 = 59;
const END_OF_DAY_HOUR: u8 = 24;
const FIRST_YEAR: i16 = 1;
const YEAR_DIGITS: usize = 4;
const NTP_FRACTION_BITS: u32 = 32;
const NTP_TIMESTAMP_LEN: usize = 8;
const HTTP_DATE_RFC850: &str = "%A, %d-%b-%y %H:%M:%S GMT";
const HTTP_DATE_ASCTIME: &str = "%a %b %e %H:%M:%S %Y";

/// Why a time string or timestamp was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DateTimeError {
    /// Not in the accepted lexical form.
    #[error("not an accepted xs:dateTime: {0}")]
    Syntax(&'static str),
    /// A field is outside its range (month 13, 29 February in a common year,
    /// hour 25, zone beyond +14:00 ...).
    #[error("xs:dateTime field out of range: {0}")]
    Field(&'static str),
    /// The instant cannot be represented.
    #[error("instant outside the representable range")]
    OutOfRange,
    /// Not an HTTP `Date` value.
    #[error("not an HTTP Date value")]
    HttpDate,
    /// Not an 8-byte NTP timestamp.
    #[error("not an 8-byte NTP timestamp")]
    Ntp,
}

/// A parsed instant plus how finely the source stated it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct XsDateTime {
    /// Microseconds since 1970-01-01T00:00:00Z; sub-microsecond digits are
    /// truncated.
    pub unix_us: i64,
    /// Number of fraction digits the text carried (0 when none, capped at 9).
    pub fraction_digits: u8,
}

impl XsDateTime {
    /// The resolution the text implies, in microseconds: one second without a
    /// fraction, 10^(6 - digits) microseconds with one (1 for 6 or more digits).
    #[must_use]
    pub fn resolution_us(&self) -> u64 {
        let digits = usize::from(self.fraction_digits).min(MICROS_DIGITS);
        let mut res = MICROS_PER_SECOND.unsigned_abs();
        if self.fraction_digits > 0 {
            res = 10u64.pow(u32::try_from(MICROS_DIGITS - digits).unwrap_or(0));
        }
        res
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn eat(&mut self, c: u8) -> bool {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn digits(&mut self, n: usize) -> Result<u32, DateTimeError> {
        let end = self.i + n;
        let slice = self
            .b
            .get(self.i..end)
            .ok_or(DateTimeError::Syntax("truncated"))?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return Err(DateTimeError::Syntax("expected digits"));
        }
        self.i = end;
        Ok(slice
            .iter()
            .fold(0u32, |acc, d| acc * 10 + u32::from(d - b'0')))
    }

    fn expect(&mut self, c: u8, what: &'static str) -> Result<(), DateTimeError> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(DateTimeError::Syntax(what))
        }
    }
}

fn narrow<T: TryFrom<u32>>(v: u32) -> Result<T, DateTimeError> {
    T::try_from(v).map_err(|_| DateTimeError::Field("value"))
}

/// Parse an `xs:dateTime` under the accepted-subset policy.
///
/// # Errors
/// [`DateTimeError`] for every form outside the subset; never a wrap.
pub fn parse_xs_datetime(text: &str) -> Result<XsDateTime, DateTimeError> {
    let trimmed = text.trim_matches([' ', '\t', '\n', '\r']);
    let mut c = Cursor {
        b: trimmed.as_bytes(),
        i: 0,
    };
    let year: i16 = narrow(c.digits(YEAR_DIGITS)?)?;
    if year < FIRST_YEAR {
        return Err(DateTimeError::Field("year"));
    }
    c.expect(b'-', "expected '-' after the year")?;
    let month: u8 = narrow(c.digits(2)?)?;
    c.expect(b'-', "expected '-' after the month")?;
    let day: u8 = narrow(c.digits(2)?)?;
    c.expect(b'T', "expected upper-case 'T'")?;
    let hour: u8 = narrow(c.digits(2)?)?;
    c.expect(b':', "expected ':' after the hour")?;
    let minute: u8 = narrow(c.digits(2)?)?;
    c.expect(b':', "expected ':' after the minute")?;
    let second: u8 = narrow(c.digits(2)?)?;

    let mut fraction_digits = 0usize;
    let mut nanos: i32 = 0;
    let mut fraction_zero = true;
    if c.eat(b'.') {
        let start = c.i;
        while c.b.get(c.i).is_some_and(u8::is_ascii_digit) {
            c.i += 1;
        }
        let frac = &c.b[start..c.i];
        if frac.is_empty() {
            return Err(DateTimeError::Syntax("empty fraction"));
        }
        fraction_digits = frac.len();
        fraction_zero = frac.iter().all(|d| *d == b'0');
        for d in frac.iter().take(NANOS_DIGITS) {
            nanos = nanos * 10 + i32::from(d - b'0');
        }
        for _ in frac.len().min(NANOS_DIGITS)..NANOS_DIGITS {
            nanos *= 10;
        }
    }

    let offset = parse_zone(&mut c)?;
    if c.i != c.b.len() {
        return Err(DateTimeError::Syntax("trailing characters"));
    }

    if month == 0 || month > 12 {
        return Err(DateTimeError::Field("month"));
    }
    let date = Date::new(
        year,
        i8::try_from(month).map_err(|_| DateTimeError::Field("month"))?,
        i8::try_from(day).map_err(|_| DateTimeError::Field("day"))?,
    )
    .map_err(|_| DateTimeError::Field("day"))?;
    if minute > MAX_MINUTE || second > MAX_SECOND {
        return Err(DateTimeError::Field("minute or second"));
    }
    let (date, hour) = if hour == END_OF_DAY_HOUR {
        if minute != 0 || second != 0 || !fraction_zero {
            return Err(DateTimeError::Field("hour 24 with a non-zero remainder"));
        }
        let next = date
            .checked_add(Span::new().days(1))
            .map_err(|_| DateTimeError::OutOfRange)?;
        (next, 0)
    } else if hour > END_OF_DAY_HOUR {
        return Err(DateTimeError::Field("hour"));
    } else {
        (date, hour)
    };
    let civil: DateTime = date.at(
        i8::try_from(hour).map_err(|_| DateTimeError::Field("hour"))?,
        i8::try_from(minute).map_err(|_| DateTimeError::Field("minute"))?,
        i8::try_from(second).map_err(|_| DateTimeError::Field("second"))?,
        nanos,
    );
    let ts = offset
        .to_timestamp(civil)
        .map_err(|_| DateTimeError::OutOfRange)?;
    Ok(XsDateTime {
        unix_us: ts.as_microsecond(),
        fraction_digits: u8::try_from(fraction_digits.min(NANOS_DIGITS)).unwrap_or(0),
    })
}

fn parse_zone(c: &mut Cursor<'_>) -> Result<Offset, DateTimeError> {
    if c.i == c.b.len() {
        return Ok(Offset::UTC);
    }
    if c.eat(b'Z') {
        return Ok(Offset::UTC);
    }
    let sign: i32 = if c.eat(b'+') {
        1
    } else if c.eat(b'-') {
        -1
    } else {
        return Err(DateTimeError::Syntax("unexpected character after the time"));
    };
    let h: u8 = narrow(c.digits(2)?)?;
    c.expect(b':', "zone offset needs 'hh:mm'")?;
    let m: u8 = narrow(c.digits(2)?)?;
    if h > MAX_ZONE_HOUR || m > MAX_MINUTE || (h == MAX_ZONE_HOUR && m != 0) {
        return Err(DateTimeError::Field("zone offset"));
    }
    let seconds = sign * (i32::from(h) * SECONDS_PER_HOUR + i32::from(m) * SECONDS_PER_MINUTE);
    Offset::from_seconds(seconds).map_err(|_| DateTimeError::Field("zone offset"))
}

/// Parse an HTTP-date (RFC 9110 5.6.7) to Unix microseconds: the preferred
/// IMF-fixdate and the two obsolete forms a recipient must still accept, the
/// RFC 850 form (two-digit years follow jiff's POSIX window: 69-99 are 19xx,
/// 00-68 are 20xx, so no clock is needed) and the asctime form (always UTC).
/// Resolution is whole seconds. Used for the `Date` and `Retry-After` headers.
///
/// # Errors
/// [`DateTimeError::HttpDate`] when `text` is not a date.
pub fn parse_http_date(text: &str) -> Result<i64, DateTimeError> {
    let text = text.trim();
    if let Ok(t) = jiff::fmt::rfc2822::DateTimeParser::new().parse_timestamp(text) {
        return Ok(t.as_microsecond());
    }
    [HTTP_DATE_RFC850, HTTP_DATE_ASCTIME]
        .iter()
        .find_map(|format| {
            jiff::fmt::strtime::parse(format, text)
                .and_then(|broken| broken.to_datetime())
                .and_then(|datetime| datetime.to_zoned(jiff::tz::TimeZone::UTC))
                .ok()
        })
        .map(|zoned| zoned.timestamp().as_microsecond())
        .ok_or(DateTimeError::HttpDate)
}

/// Read a 64-bit NTP timestamp (RFC 5905 Figure 3: 32-bit seconds since
/// 1900-01-01T00:00:00Z, 32-bit binary fraction; era 0) as Unix microseconds.
///
/// # Errors
/// [`DateTimeError::Ntp`] unless `bytes` is exactly eight bytes.
pub fn parse_ntp_timestamp(bytes: &[u8]) -> Result<i64, DateTimeError> {
    let arr: [u8; NTP_TIMESTAMP_LEN] = bytes.try_into().map_err(|_| DateTimeError::Ntp)?;
    let seconds = u32::from_be_bytes([arr[0], arr[1], arr[2], arr[3]]);
    let fraction = u32::from_be_bytes([arr[4], arr[5], arr[6], arr[7]]);
    let micros =
        (u128::from(fraction) * u128::from(MICROS_PER_SECOND.unsigned_abs())) >> NTP_FRACTION_BITS;
    let micros = i64::try_from(micros).map_err(|_| DateTimeError::Ntp)?;
    let prime_epoch = Date::new(1900, 1, 1)
        .map_err(|_| DateTimeError::Ntp)?
        .at(0, 0, 0, 0)
        .to_zoned(TimeZone::UTC)
        .map_err(|_| DateTimeError::Ntp)?
        .timestamp()
        .as_second();
    let unix_seconds = i64::from(seconds) + prime_epoch;
    unix_seconds
        .checked_mul(MICROS_PER_SECOND)
        .and_then(|us| us.checked_add(micros))
        .ok_or(DateTimeError::Ntp)
}
