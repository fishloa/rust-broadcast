#![cfg(feature = "http-date")]

//! The HTTP `Date` reader (RFC 9110 5.6.7): three accepted forms, the POSIX
//! two-digit-year window, and rejection of garbage.

use broadcast_common::http_date::{HttpDateError, parse_http_date};

const SECOND: i64 = 1_000_000;

#[test]
fn http_date_is_whole_seconds() {
    assert_eq!(
        parse_http_date("Mon, 26 Oct 2026 08:39:04 GMT").unwrap(),
        1_793_003_944 * SECOND
    );
    assert!(matches!(
        parse_http_date("yesterday"),
        Err(HttpDateError::HttpDate)
    ));
}

#[test]
fn http_date_accepts_the_three_forms_of_rfc_9110_5_6_7() {
    let expected = 784_111_777 * SECOND;
    for text in [
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        "  Sun, 06 Nov 1994 08:49:37 GMT  ",
        "  Sunday, 06-Nov-94 08:49:37 GMT\t",
        " Sun Nov  6 08:49:37 1994 ",
    ] {
        assert_eq!(parse_http_date(text).unwrap(), expected, "{text}");
    }
    assert_eq!(
        parse_http_date("Mon Oct 26 08:39:04 2026").unwrap(),
        1_793_003_944 * SECOND
    );
}

#[test]
fn http_date_two_digit_years_use_the_posix_window() {
    assert_eq!(
        parse_http_date("Thursday, 01-Jan-70 00:00:00 GMT").unwrap(),
        0
    );
    assert_eq!(
        parse_http_date("Monday, 26-Oct-26 08:39:04 GMT").unwrap(),
        1_793_003_944 * SECOND
    );
}

#[test]
fn http_date_rejects_garbage_and_impossible_dates() {
    for text in [
        "",
        "now",
        "Sun, 31 Feb 1994 08:49:37 GMT",
        "Sunday, 31-Feb-94 08:49:37 GMT",
        "Sun Feb 31 08:49:37 1994",
        "Sun, 06 Nov 1994 25:49:37 GMT",
        "Sun Nov  6 08:49:37",
    ] {
        assert!(
            matches!(parse_http_date(text), Err(HttpDateError::HttpDate)),
            "{text:?}"
        );
    }
}
