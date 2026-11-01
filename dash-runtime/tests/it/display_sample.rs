//! `Display` agrees with `name()` for a sample of dash-runtime label enums.
//! (The source-scanning half of the #204 guard lives in `workspace-guards`.)

#[test]
fn display_matches_name_for_a_sample() {
    use dash_runtime::timing::clock::{AdoptReason, ClockSource};
    use dash_runtime::timing::utc_timing::UtcScheme;
    assert_eq!(
        ClockSource::UtcTiming.to_string(),
        ClockSource::UtcTiming.name()
    );
    assert_eq!(AdoptReason::Step.to_string(), "step");
    assert_eq!(UtcScheme::HttpXsdate.to_string(), "http_xsdate");
}
