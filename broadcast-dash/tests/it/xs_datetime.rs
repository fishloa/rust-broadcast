//! `xs:dateTime` (accepted subset), clock resolution and the NTP timestamp
//! reader: the lexical and range rules of the module doc, and RFC 5905's
//! 1900 epoch with its 32-bit fraction.

use broadcast_dash::xs_datetime::{DateTimeError, parse_ntp_timestamp, parse_xs_datetime};

/// 2024-01-01T00:00:00Z.
const NEW_YEAR_2024: i64 = 1_704_067_200;

fn unix(text: &str) -> i64 {
    parse_xs_datetime(text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .unix
        .as_second()
}

fn field(text: &str) -> DateTimeError {
    match parse_xs_datetime(text) {
        Ok(t) => panic!("{text} parsed to {t:?}"),
        Err(e) => e,
    }
}

#[test]
fn a_zone_offset_is_applied_with_its_sign() {
    assert_eq!(unix("2024-01-01T00:00:00Z"), NEW_YEAR_2024);
    assert_eq!(unix("2024-01-01T00:00:00"), NEW_YEAR_2024);
    assert_eq!(unix("2024-01-01T00:00:00+00:00"), NEW_YEAR_2024);
    assert_eq!(unix("2024-01-01T00:00:00-05:00"), NEW_YEAR_2024 + 5 * 3_600);
    assert_eq!(
        unix("2024-01-01T00:00:00+05:30"),
        NEW_YEAR_2024 - 5 * 3_600 - 1_800
    );
    assert_eq!(
        unix("2024-01-01T00:00:00+14:00"),
        NEW_YEAR_2024 - 14 * 3_600
    );
    assert_eq!(
        unix("2024-01-01T00:00:00-14:00"),
        NEW_YEAR_2024 + 14 * 3_600
    );
    assert_eq!(
        unix("2024-01-01T00:00:00+05:59"),
        NEW_YEAR_2024 - 5 * 3_600 - 59 * 60
    );
    assert_eq!(
        unix("2024-01-01T00:00:00+13:30"),
        NEW_YEAR_2024 - 13 * 3_600 - 1_800
    );
}

#[test]
fn a_zone_offset_outside_pm_14_hours_is_a_field_error() {
    assert_eq!(
        field("2024-01-01T00:00:00+14:01"),
        DateTimeError::Field("zone offset")
    );
    assert_eq!(
        field("2024-01-01T00:00:00-14:01"),
        DateTimeError::Field("zone offset")
    );
    assert_eq!(
        field("2024-01-01T00:00:00+15:00"),
        DateTimeError::Field("zone offset")
    );
    assert_eq!(
        field("2024-01-01T00:00:00+13:60"),
        DateTimeError::Field("zone offset")
    );
    assert_eq!(
        field("2024-01-01T00:00:00+0500"),
        DateTimeError::Syntax("zone offset needs 'hh:mm'")
    );
    assert_eq!(
        field("2024-01-01T00:00:00x"),
        DateTimeError::Syntax("unexpected character after the time")
    );
}

#[test]
fn calendar_and_time_fields_are_range_checked() {
    assert_eq!(field("2024-13-01T00:00:00Z"), DateTimeError::Field("month"));
    assert_eq!(field("2024-00-01T00:00:00Z"), DateTimeError::Field("month"));
    assert_eq!(field("2024-02-30T00:00:00Z"), DateTimeError::Field("day"));
    assert_eq!(field("2023-02-29T00:00:00Z"), DateTimeError::Field("day"));
    assert_eq!(
        field("2024-01-01T23:60:00Z"),
        DateTimeError::Field("minute or second")
    );
    assert_eq!(
        field("2024-01-01T23:59:60Z"),
        DateTimeError::Field("minute or second")
    );
    assert_eq!(field("2024-01-01T25:00:00Z"), DateTimeError::Field("hour"));
    assert_eq!(field("0000-01-01T00:00:00Z"), DateTimeError::Field("year"));
    assert_eq!(
        field("2024-01-01T24:00:01Z"),
        DateTimeError::Field("hour 24 with a non-zero remainder")
    );
    assert_eq!(
        field("2024-01-01T24:00:00.1Z"),
        DateTimeError::Field("hour 24 with a non-zero remainder")
    );
    // The edges themselves are accepted.
    unix("0001-01-01T00:00:00Z");
    unix("2024-12-31T23:59:59Z");
    unix("2024-02-29T00:00:00Z");
}

#[test]
fn hour_24_is_the_next_midnight_only_with_a_zero_remainder() {
    assert_eq!(unix("2024-01-01T24:00:00Z"), NEW_YEAR_2024 + 86_400);
    assert_eq!(unix("2024-01-01T24:00:00.000Z"), NEW_YEAR_2024 + 86_400);
    assert_eq!(
        unix("2024-01-01T24:00:00.000000000Z"),
        NEW_YEAR_2024 + 86_400
    );
    assert_eq!(unix("2024-12-31T24:00:00Z"), NEW_YEAR_2024 + 366 * 86_400);
}

#[test]
fn lexical_errors_are_structured() {
    assert!(matches!(
        field("2024-01-01t00:00:00Z"),
        DateTimeError::Syntax(_)
    ));
    assert!(matches!(
        field("2024-01-01T00:00:00Zx"),
        DateTimeError::Syntax("trailing characters")
    ));
    assert!(matches!(
        field("2024-1-01T00:00:00Z"),
        DateTimeError::Syntax(_)
    ));
    assert!(matches!(field("nonsense"), DateTimeError::Syntax(_)));
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    assert_eq!(unix(" \t2024-01-01T00:00:00Z\n"), NEW_YEAR_2024);
}

#[test]
fn the_fraction_digits_state_the_clock_resolution() {
    let resolution = |text: &str| {
        parse_xs_datetime(text)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
            .resolution_us()
    };
    assert_eq!(resolution("2024-01-01T00:00:00Z"), 1_000_000);
    assert_eq!(resolution("2024-01-01T00:00:00.0Z"), 100_000);
    assert_eq!(resolution("2024-01-01T00:00:00.12Z"), 10_000);
    assert_eq!(resolution("2024-01-01T00:00:00.123Z"), 1_000);
    assert_eq!(resolution("2024-01-01T00:00:00.123456Z"), 1);
    assert_eq!(resolution("2024-01-01T00:00:00.1234567Z"), 1);
    assert_eq!(resolution("2024-01-01T00:00:00.123456789Z"), 1);
    assert_eq!(
        parse_xs_datetime("2024-01-01T00:00:00.123456789123Z")
            .unwrap()
            .fraction_digits,
        9
    );
}

#[test]
fn the_fraction_is_nanoseconds_within_the_second() {
    let t = parse_xs_datetime("2024-01-01T00:00:00.5Z").unwrap();
    assert_eq!(t.unix.as_microsecond(), NEW_YEAR_2024 * 1_000_000 + 500_000);
    let t = parse_xs_datetime("2024-01-01T00:00:00.000000001Z").unwrap();
    assert_eq!(
        t.unix.as_nanosecond(),
        i128::from(NEW_YEAR_2024) * 1_000_000_000 + 1
    );
}

#[test]
fn an_ntp_zero_is_the_1900_epoch() {
    let t = parse_ntp_timestamp(&[0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(t.as_second(), -2_208_988_800);
    assert_eq!(t.as_microsecond(), -2_208_988_800_000_000);
}

#[test]
fn an_ntp_second_count_is_measured_from_1900() {
    let t = parse_ntp_timestamp(&[0x83, 0xAA, 0x7E, 0x80, 0, 0, 0, 0]).unwrap();
    assert_eq!(t.as_microsecond(), 0);
    let t = parse_ntp_timestamp(&[0x83, 0xAA, 0x7E, 0x80, 0x80, 0, 0, 0]).unwrap();
    assert_eq!(t.as_microsecond(), 500_000);
    let t = parse_ntp_timestamp(&[0x83, 0xAA, 0x7E, 0x81, 0x40, 0, 0, 0]).unwrap();
    assert_eq!(t.as_microsecond(), 1_250_000);
}

#[test]
fn an_ntp_fraction_scales_by_rfc_5905_resolution() {
    let t = parse_ntp_timestamp(&[0x83, 0xAA, 0x7E, 0x80, 0x20, 0, 0, 0]).unwrap();
    assert_eq!(t.as_microsecond(), 125_000);
    let t = parse_ntp_timestamp(&[0x83, 0xAA, 0x7E, 0x80, 0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    assert_eq!(t.as_microsecond(), 999_999);
}

#[test]
fn a_short_or_long_ntp_timestamp_is_refused() {
    assert_eq!(parse_ntp_timestamp(&[]), Err(DateTimeError::Ntp));
    assert_eq!(parse_ntp_timestamp(&[0; 7]), Err(DateTimeError::Ntp));
    assert_eq!(parse_ntp_timestamp(&[0; 9]), Err(DateTimeError::Ntp));
}
