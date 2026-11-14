//! HTTP `Date` and `Retry-After` date readers (RFC 9110 §5.6.7).
//!
//! Calendar arithmetic is jiff's `strptime`; nothing here reads a clock.

use thiserror::Error;

const HTTP_DATE_RFC850: &str = "%A, %d-%b-%y %H:%M:%S GMT";
const HTTP_DATE_ASCTIME: &str = "%a %b %e %H:%M:%S %Y";

/// Why an HTTP date was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum HttpDateError {
    /// Not an HTTP `Date` value.
    #[error("not an HTTP Date value")]
    HttpDate,
}

/// A typed `Date` as Unix microseconds: the one conversion from the `headers` crate's already
/// parsed value (what an HTTP client holds) to the numeric instant the rest of the workspace
/// uses. It never reads a clock; the value is the server's. A `Date` before 1970 or outside the
/// microsecond range is refused rather than wrapped.
///
/// # Errors
/// [`HttpDateError::HttpDate`] when the instant cannot be expressed in microseconds.
pub fn date_to_epoch_us(date: headers::Date) -> Result<i64, HttpDateError> {
    let at = std::time::UNIX_EPOCH;
    let since = std::time::SystemTime::from(date);
    let after = since
        .duration_since(at)
        .map_err(|_| HttpDateError::HttpDate)?;
    i64::try_from(after.as_micros()).map_err(|_| HttpDateError::HttpDate)
}

/// Parse an HTTP-date (RFC 9110 §5.6.7) to Unix microseconds: the preferred
/// IMF-fixdate and the two obsolete forms a recipient must still accept, the
/// RFC 850 form (two-digit years follow jiff's POSIX window: 69-99 are 19xx,
/// 00-68 are 20xx, so no clock is needed) and the asctime form (always UTC).
/// Resolution is whole seconds. Used for the `Date` and `Retry-After` headers.
///
/// # Errors
/// [`HttpDateError::HttpDate`] when `text` is not a date.
pub fn parse_http_date(text: &str) -> Result<i64, HttpDateError> {
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
        .ok_or(HttpDateError::HttpDate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_forms_agree() {
        let expected = 784_111_777 * 1_000_000;
        for text in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(parse_http_date(text), Ok(expected), "{text}");
        }
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse_http_date("yesterday"), Err(HttpDateError::HttpDate));
    }

    /// The typed `Date` a client holds converts to the same instant the parser names.
    #[test]
    fn a_typed_date_is_the_parsed_instant() {
        use headers::HeaderMapExt;
        let text = "Sun, 06 Nov 1994 08:49:37 GMT";
        let mut map = http::HeaderMap::new();
        map.insert(http::header::DATE, http::HeaderValue::from_static(text));
        let date = map.typed_get::<headers::Date>().unwrap();
        assert_eq!(date_to_epoch_us(date), parse_http_date(text));
    }
}
