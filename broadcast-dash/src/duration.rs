//! The `xs:duration` reader every duration-valued MPD (ISO/IEC 23009-1)
//! attribute uses.
//!
//! The one reader in the workspace for this lexical form, shared by the MPD
//! parser here and by `transmux`'s MPD writer, which renders `.`/`S`
//! durations and reads them back to check the round trip.
//!
//! [`parse_iso8601_duration`] reads the `PnYnMnDTnHnMnS` lexical form (W3C XML
//! Schema 1.1 Part 2 §3.3.6): the `P`/`T` and every designator are UPPER case,
//! numbers are digits only in order, only seconds may carry a fraction (a digit
//! on both sides of the `.`), and a fraction longer than nine digits is
//! truncated to nanoseconds. Anything else — weeks, a sign, a comma, a
//! calendar `nY`/`nM` (ambiguous without a reference date), a negative value,
//! a magnitude beyond `jiff`'s span limits — is a structured error, never a
//! panic.

use std::fmt;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// The error [`parse_iso8601_duration`] returns for a value it cannot read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DashParseError {
    /// An `xs:duration` string (W3C XML Schema Part 2 §3.2.6) could not be
    /// parsed as a [`Duration`].
    InvalidDuration {
        /// The raw (unparsable) value.
        value: String,
    },
}

impl fmt::Display for DashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DashParseError::InvalidDuration { value } => {
                write!(f, "invalid xs:duration {value:?}")
            }
        }
    }
}

impl std::error::Error for DashParseError {}

// ---------------------------------------------------------------------------
// xs:duration
// ---------------------------------------------------------------------------

/// Parse an `xs:duration` string (W3C XML Schema 1.1 Part 2 §3.3.6, as used by
/// every duration-valued MPD attribute — ISO/IEC 23009-1 §5.3.1.2/§5.3.2.2/
/// §5.3.9.2.2): the `PnYnMnDTnHnMnS` lexical form, e.g. `PT1H2M3.5S`, `PT4S`,
/// `PT0S`, `P1DT2H`. Surrounding whitespace is trimmed.
///
/// The value is converted by `jiff`'s ISO 8601 span parser with a day counting
/// as 24 hours. Every input outside that is a [`DashParseError::InvalidDuration`]
/// (never a panic): a lexically valid but unrepresentable value — calendar
/// `nY`/`nM` (years/months are ambiguous without a reference date), a negative
/// duration (`-PT1S`, valid XSD but [`Duration`] is unsigned), or a magnitude
/// beyond `jiff`'s span limits (`PT999999999H`).
pub fn parse_iso8601_duration(s: &str) -> Result<Duration> {
    let trimmed = s.trim();
    let invalid = || DashParseError::InvalidDuration {
        value: trimmed.to_string(),
    };
    let normalised = normalise_xs_duration(trimmed).ok_or_else(invalid)?;
    let span = jiff::fmt::temporal::SpanParser::new()
        .parse_span(normalised.as_str())
        .map_err(|_| invalid())?;
    // Days count as 24 h; years and months are calendar units that need a
    // reference date, so `to_duration` refuses them.
    let signed = span
        .to_duration(jiff::SpanRelativeTo::days_are_24_hours())
        .map_err(|_| invalid())?;
    Duration::try_from(signed).map_err(|_| invalid())
}

/// Validate `s` against the xs:duration lexical grammar (see
/// [`parse_iso8601_duration`]) and return it with the seconds fraction
/// truncated to nine digits; `None` if it is not in the lexical space or is
/// negative (a leading `-` is lexically valid, but `Duration` cannot hold it).
fn normalise_xs_duration(s: &str) -> Option<String> {
    let rest = s.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    let mut components = 0usize;
    let mut out = String::from("P");
    // Date part: [nY][nM][nD], in order, digits only.
    let mut remaining = date_part;
    for designator in ['Y', 'M', 'D'] {
        if let Some((digits, tail)) = remaining.split_once(designator) {
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            out.push_str(digits);
            out.push(designator);
            remaining = tail;
            components += 1;
        }
    }
    if !remaining.is_empty() {
        return None;
    }
    if let Some(time) = time_part {
        out.push('T');
        let mut remaining = time;
        let mut time_components = 0usize;
        for designator in ['H', 'M'] {
            if let Some((digits, tail)) = remaining.split_once(designator) {
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                out.push_str(digits);
                out.push(designator);
                remaining = tail;
                time_components += 1;
            }
        }
        if let Some((number, tail)) = remaining.split_once('S') {
            if !tail.is_empty() {
                return None;
            }
            let (whole, frac) = match number.split_once('.') {
                Some((w, f)) => (w, Some(f)),
                None => (number, None),
            };
            if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            out.push_str(whole);
            if let Some(frac) = frac {
                if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                out.push('.');
                // Valid XSD allows any number of digits; Duration holds nanoseconds.
                out.push_str(&frac[..frac.len().min(9)]);
            }
            out.push('S');
            time_components += 1;
        } else if !remaining.is_empty() {
            return None;
        }
        if time_components == 0 {
            return None;
        }
        components += time_components;
    }
    (components > 0).then_some(out)
}

/// Crate-local result alias for this module.
type Result<T> = core::result::Result<T, DashParseError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Seconds in a day.
    const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
    /// Seconds in an hour.
    const SECONDS_PER_HOUR: u64 = 60 * 60;

    #[test]
    fn hours_minutes_fractional_seconds() {
        assert_eq!(
            parse_iso8601_duration("PT1H2M3.5S").unwrap(),
            Duration::new(3723, 500_000_000)
        );
    }

    #[test]
    fn seconds_only() {
        assert_eq!(parse_iso8601_duration("PT4S").unwrap(), Duration::new(4, 0));
    }

    #[test]
    fn zero() {
        assert_eq!(parse_iso8601_duration("PT0S").unwrap(), Duration::new(0, 0));
    }

    #[test]
    fn days_and_hours() {
        assert_eq!(
            parse_iso8601_duration("P1DT2H").unwrap(),
            Duration::new(SECONDS_PER_DAY + 2 * SECONDS_PER_HOUR, 0)
        );
    }

    #[test]
    fn writer_tenths_form() {
        assert_eq!(
            parse_iso8601_duration("PT2.0S").unwrap(),
            Duration::new(2, 0)
        );
    }

    #[test]
    fn rejects_bare_p() {
        assert!(matches!(
            parse_iso8601_duration("P"),
            Err(DashParseError::InvalidDuration { .. })
        ));
    }

    #[test]
    fn rejects_missing_prefix() {
        assert!(matches!(
            parse_iso8601_duration("1H2M3S"),
            Err(DashParseError::InvalidDuration { .. })
        ));
    }

    #[test]
    fn rejects_calendar_months() {
        assert!(matches!(
            parse_iso8601_duration("P1M"),
            Err(DashParseError::InvalidDuration { .. })
        ));
    }
}
