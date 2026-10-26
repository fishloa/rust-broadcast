//! The accept/reject table of `docs/specs/dash/xs-datetime-subset.md`, whose
//! instants were hand-checked against an independent calendar implementation.

use dash_runtime::timing::datetime::{
    DateTimeError, parse_http_date, parse_ntp_timestamp, parse_xs_datetime,
};

const SECOND: i64 = 1_000_000;

#[test]
fn accepted_table() {
    let cases: &[(&str, i64)] = &[
        ("1970-01-01T00:00:00Z", 0),
        ("2026-10-26T08:39:04.952Z", 1_793_003_944 * SECOND + 952_000),
        ("2026-10-26T09:18:20.766Z", 1_793_006_300 * SECOND + 766_000),
        ("2002-10-10T12:00:00-05:00", 1_034_269_200 * SECOND),
        ("2026-03-01T00:00:00+14:00", 1_772_272_800 * SECOND),
        ("2026-04-01T00:00:00+05:30", 1_774_981_800 * SECOND),
        ("2026-02-01T00:00:00", 1_769_904_000 * SECOND),
        ("2026-02-01T24:00:00Z", 1_769_990_400 * SECOND),
        ("2024-02-29T12:00:00Z", 1_709_208_000 * SECOND),
        ("0001-01-01T00:00:00Z", -62_135_596_800 * SECOND),
        ("9999-12-30T22:00:00Z", 253_402_207_200 * SECOND),
        (
            "2026-10-26T08:39:04.123456789012Z",
            1_793_003_944 * SECOND + 123_456,
        ),
        ("  2026-02-01T00:00:00Z\n", 1_769_904_000 * SECOND),
    ];
    for (text, us) in cases {
        let got = parse_xs_datetime(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(got.unix_us, *us, "{text}");
    }
}

#[test]
fn rejected_table() {
    for text in [
        "2025-02-29T12:00:00Z",
        "2026-13-01T00:00:00Z",
        "2016-12-31T23:59:60Z",
        "2026-02-01T24:00:01Z",
        "2026-02-01T24:00:00.5Z",
        "-0001-01-01T00:00:00Z",
        "12026-01-01T00:00:00Z",
        "0000-01-01T00:00:00Z",
        "2026-02-01T00:00:00+14:30",
        "2026-02-01T00:00:00+0100",
        "2026-02-01t00:00:00z",
        "2026-02-01 00:00:00Z",
        "20260201T000000Z",
        "2026-02-01T00:00:00.Z",
        "2026-02-01T00:00:00,5Z",
        "9999-12-31T23:59:59Z",
        "",
        "2026-02-01",
        "2026-02-01T00:00:00Z extra",
        "2026-02-01T00:00:00 Z",
    ] {
        assert!(
            parse_xs_datetime(text).is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn resolution_follows_fraction_digits() {
    let r = |t: &str| parse_xs_datetime(t).unwrap().resolution_us();
    assert_eq!(r("2026-10-26T08:39:04Z"), 1_000_000);
    assert_eq!(r("2026-10-26T08:39:04.9Z"), 100_000);
    assert_eq!(r("2026-10-26T08:39:04.952Z"), 1_000);
    assert_eq!(r("2026-10-26T08:39:04.952123Z"), 1);
    assert_eq!(r("2026-10-26T08:39:04.952123789Z"), 1);
}

#[test]
fn errors_are_typed() {
    assert!(matches!(
        parse_xs_datetime("2026-13-01T00:00:00Z"),
        Err(DateTimeError::Field(_))
    ));
    assert!(matches!(
        parse_xs_datetime("9999-12-31T23:59:59Z"),
        Err(DateTimeError::OutOfRange)
    ));
    assert!(matches!(
        parse_xs_datetime("nonsense"),
        Err(DateTimeError::Syntax(_))
    ));
}

#[test]
fn http_date_is_whole_seconds() {
    assert_eq!(
        parse_http_date("Mon, 26 Oct 2026 08:39:04 GMT").unwrap(),
        1_793_003_944 * SECOND
    );
    assert!(parse_http_date("yesterday").is_err());
}

#[test]
fn ntp_timestamp_maps_prime_epoch() {
    let unix_epoch_in_ntp_seconds: u32 = 2_208_988_800;
    let mut bytes = unix_epoch_in_ntp_seconds.to_be_bytes().to_vec();
    bytes.extend_from_slice(&0x8000_0000u32.to_be_bytes());
    assert_eq!(parse_ntp_timestamp(&bytes).unwrap(), 500_000);
    assert!(parse_ntp_timestamp(&bytes[..7]).is_err());
}
