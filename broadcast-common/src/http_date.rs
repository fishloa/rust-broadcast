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
}
