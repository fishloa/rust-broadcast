//! SP5 (jiff). The exact wire spellings of `availabilityStartTime` and the
//! DASH `@duration` (`PT…S`), pinned so the jiff migration changes no byte.
//! `format_iso8601_for_test`/`xs_duration_for_test`/`parse_iso8601_utc_for_test`
//! exercise the same code the DASH renderers and the DASH-pull client use.

use std::time::{Duration, UNIX_EPOCH};

#[test]
fn availability_start_time_is_jiff_and_matches_the_pre_jiff_spelling() {
    let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert_eq!(
        multimux::output::dash::format_iso8601_for_test(t),
        "2023-11-14T22:13:20Z"
    );
}

#[test]
fn a_duration_uses_jiff_shortest_iso_8601_spelling() {
    assert_eq!(
        multimux::output::dash::xs_duration_for_test(Duration::from_secs(2)),
        "PT2S"
    );
    assert_eq!(
        multimux::output::dash::xs_duration_for_test(Duration::from_millis(500)),
        "PT0.5S"
    );
}

#[test]
fn availability_start_time_parses_back_to_the_same_instant() {
    assert_eq!(
        multimux::source::dash_pull::parse_iso8601_utc_for_test("2023-11-14T22:13:20Z"),
        Some(1_700_000_000)
    );
    assert_eq!(
        multimux::source::dash_pull::parse_iso8601_utc_for_test("not-a-date"),
        None
    );
    // A fractional-second UTC form is accepted (doc claims fractional
    // round-trip), truncated to whole seconds.
    assert_eq!(
        multimux::source::dash_pull::parse_iso8601_utc_for_test("2023-11-14T22:13:20.500Z"),
        Some(1_700_000_000)
    );
    // An OFFSET form is rejected, not silently converted: the UTC `Z` guard is
    // lexical, so a `+01:00` timestamp falls back rather than being
    // reinterpreted.
    assert_eq!(
        multimux::source::dash_pull::parse_iso8601_utc_for_test("2023-11-14T22:13:20+01:00"),
        None
    );
    assert_eq!(
        multimux::source::dash_pull::parse_iso8601_utc_for_test("2023-11-14T22:13:20-05:00"),
        None
    );
    // No `Z` and no offset is also rejected (the shape is not the one
    // `availabilityStartTime` uses).
    assert_eq!(
        multimux::source::dash_pull::parse_iso8601_utc_for_test("2023-11-14T22:13:20"),
        None
    );
}
