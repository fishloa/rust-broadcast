//! Decisions the fixture-driven suites leave open: the join instant, the
//! time-shift window boundary, the UTC body bound, the static Period kind,
//! AdaptationSet classification by `@contentType` alone, BaseURL
//! `@availabilityTimeOffset` accumulation and the synthetic-scheme guard.

use core::time::Duration;

use dash_runtime::client::{DashConfig, PullLimits};
use dash_runtime::mpd::Mpd;
use dash_runtime::mpd::urls::BaseCandidate;
use dash_runtime::selection::{DropReason, Policy, SetClass, classify, select};
use dash_runtime::timing::live_edge::{join_time, oldest_in_window};
use dash_runtime::timing::period::{PeriodKind, period_timings};
use dash_runtime::timing::utc_timing::{Exchange, UTC_BODY_MAX_BYTES, UtcScheme, sample_from_body};

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";
const S: i128 = 1_000_000;

fn mpd(attrs: &str, body: &str) -> Mpd {
    let xml = format!(r#"<?xml version="1.0"?><MPD xmlns="{NS}" {attrs}>{body}</MPD>"#);
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

fn utc(us: i128) -> jiff::Timestamp {
    jiff::Timestamp::from_microsecond(
        i64::try_from(us.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap(),
    )
    .unwrap()
}

fn mono(us: u64) -> broadcast_common::stage::Timestamp {
    broadcast_common::stage::Timestamp::from_micros(us)
}

fn secs(n: i128) -> Duration {
    Duration::from_micros(u64::try_from(n * S).unwrap())
}

const TPL: &str =
    r#"<SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>"#;

#[test]
fn the_join_instant_is_now_minus_the_delay_minus_the_presentation_origin() {
    let t = join_time(utc(1100 * S), secs(10), utc(1000 * S), secs(0));
    assert_eq!(t.as_micros(), 90 * S);
    let with_period = join_time(utc(1100 * S), secs(10), utc(1000 * S), secs(5));
    assert_eq!(with_period.as_micros(), 85 * S);
    let before = join_time(utc(10 * S), secs(10), utc(1000 * S), secs(0));
    assert_eq!(before.as_micros(), -1000 * S);
}

#[test]
fn a_segment_whose_availability_ends_now_is_still_in_the_window() {
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" timeShiftBufferDepth="PT30S""#,
        r#"<Period id="p" start="PT0S"><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1000" duration="2000" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = dash_runtime::address::plan_representation(
        &m,
        dash_runtime::selection::RepRef::at(0, 0, 0),
        &dash_runtime::address::PlanOptions::default(),
    )
    .unwrap()
    .plan;
    let ast = 1_767_225_600_000_000i128;
    // Segment k ends its window at AST + (k + 2) x 2 s + 30 s; k = 3 ends at 40 s.
    assert_eq!(oldest_in_window(&p, utc(ast + 40 * S)), Some(3));
    assert_eq!(oldest_in_window(&p, utc(ast + 40 * S + 1)), Some(4));
}

#[test]
fn the_utc_body_bound_is_four_kibibytes() {
    assert_eq!(UTC_BODY_MAX_BYTES, 4096);
    let ex = Exchange::new(mono(0), mono(10));
    let max_rtt = Duration::from_millis(50);
    let at_limit = vec![b' '; UTC_BODY_MAX_BYTES];
    assert!(
        !matches!(
            sample_from_body(UtcScheme::HttpXsdate, &at_limit, ex, max_rtt),
            Err(dash_runtime::timing::TimingError::BodyTooLarge { .. })
        ),
        "exactly the bound is read"
    );
    let over = vec![b' '; UTC_BODY_MAX_BYTES + 1];
    assert!(matches!(
        sample_from_body(UtcScheme::HttpXsdate, &over, ex, max_rtt),
        Err(dash_runtime::timing::TimingError::BodyTooLarge { .. })
    ));
}

#[test]
fn a_static_period_after_one_without_a_duration_is_regular_not_early_available() {
    let m = mpd(r#"type="static""#, r#"<Period id="a"/><Period id="b"/>"#);
    let t = period_timings(&m);
    assert_eq!(t[0].start, Some(Duration::ZERO));
    assert_eq!(t[1].start, None);
    assert_eq!(t[1].kind, PeriodKind::Regular);
}

#[test]
fn the_max_segment_body_default_is_sixty_four_mibibytes() {
    assert_eq!(PullLimits::default().max_segment_bytes, 67_108_864);
    assert_eq!(DashConfig::default().utc_timeout, Duration::from_secs(2));
}

fn set(attrs: &str, reps: &[(&str, u64, &str)]) -> String {
    let reps: String = reps
        .iter()
        .map(|(id, bw, codecs)| {
            let codecs = if codecs.is_empty() {
                String::new()
            } else {
                format!(r#" codecs="{codecs}""#)
            };
            format!(r#"<Representation id="{id}" bandwidth="{bw}"{codecs}/>"#)
        })
        .collect();
    format!(r#"<AdaptationSet {attrs}>{TPL}{reps}</AdaptationSet>"#)
}

fn with_period(sets: &str) -> String {
    format!("<Period>{sets}</Period>")
}

fn protected(attrs: &str, id: &str, bandwidth: u64, drm: &str) -> String {
    format!(
        r#"<AdaptationSet {attrs}>{TPL}<Representation id="{id}" bandwidth="{bandwidth}">{drm}</Representation></AdaptationSet>"#
    )
}

#[test]
fn a_set_is_classified_by_content_type_alone() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT10S""#,
        &with_period(&format!(
            "{}{}{}",
            set(r#"contentType="video""#, &[("v", 1, "")]),
            set(r#"contentType="audio""#, &[("a", 1, "")]),
            set(r#"contentType="text""#, &[("t", 1, "")])
        )),
    );
    assert_eq!(classify(&m.periods[0].adaptation_sets[0]), SetClass::Video);
    assert_eq!(classify(&m.periods[0].adaptation_sets[1]), SetClass::Audio);
    assert_eq!(classify(&m.periods[0].adaptation_sets[2]), SetClass::Text);
    assert_eq!(
        classify(
            &mpd(
                r#"type="static""#,
                &with_period(&set(r#"mimeType="video/mp2t""#, &[("t", 1, "")]))
            )
            .periods[0]
                .adaptation_sets[0]
        ),
        SetClass::Video,
        "the MIME type decides when @contentType is absent"
    );
}

#[test]
fn a_stpp_codec_alone_makes_a_set_text() {
    let m = mpd(
        r#"type="static""#,
        &with_period(&set(r#"codecs="stpp.ttml.im1t""#, &[("s", 1, "")])),
    );
    assert_eq!(classify(&m.periods[0].adaptation_sets[0]), SetClass::Text);
    let plain = mpd(r#"type="static""#, &with_period(&set("", &[("x", 1, "")])));
    assert_eq!(
        classify(&plain.periods[0].adaptation_sets[0]),
        SetClass::Other
    );
}

#[test]
fn a_text_mime_type_alone_makes_a_set_text() {
    let m = mpd(
        r#"type="static""#,
        &with_period(&set(r#"mimeType="text/vtt""#, &[("s", 1, "")])),
    );
    assert_eq!(classify(&m.periods[0].adaptation_sets[0]), SetClass::Text);
    let ttml = mpd(
        r#"type="static""#,
        &with_period(&set(r#"mimeType="application/ttml+xml""#, &[("t", 1, "")])),
    );
    assert_eq!(
        classify(&ttml.periods[0].adaptation_sets[0]),
        SetClass::Text,
        "an application MIME type naming TTML is text"
    );
    let app = mpd(
        r#"type="static""#,
        &with_period(&set(r#"mimeType="application/mp4""#, &[("m", 1, "")])),
    );
    assert_eq!(
        classify(&app.periods[0].adaptation_sets[0]),
        SetClass::Other,
        "an application MIME type is only text when it names TTML"
    );
}

#[test]
fn a_protected_set_needs_an_unprotected_alternative_in_the_same_group_and_class() {
    let drm = r#"<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"/>"#;
    let cases = [
        (
            r#"contentType="video" mimeType="video/mp4" group="1""#,
            r#"contentType="video" mimeType="video/mp4" group="1""#,
            DropReason::ProtectedAlternativeExists,
        ),
        (
            r#"contentType="video" mimeType="video/mp4" group="1""#,
            r#"contentType="audio" mimeType="audio/mp4" group="1""#,
            DropReason::UnsupportedEncryption,
        ),
        (
            r#"contentType="video" mimeType="video/mp4" group="1""#,
            r#"contentType="video" mimeType="video/mp4" group="2""#,
            DropReason::UnsupportedEncryption,
        ),
    ];
    for (attrs, sibling, expected) in cases {
        let m = mpd(
            r#"type="static" mediaPresentationDuration="PT10S""#,
            &with_period(&format!(
                "{}{}",
                protected(attrs, "drm", 1, drm),
                set(sibling, &[("plain", 2, "")])
            )),
        );
        let s = select(&m, 0, &Policy::default());
        assert_eq!(
            s.dropped.iter().find(|d| d.set == 0).map(|d| d.reason),
            Some(expected),
            "{attrs} / {sibling}: {:?}",
            s.dropped
        );
    }
}

#[test]
fn a_protected_set_without_a_group_has_no_alternative() {
    let drm = r#"<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"/>"#;
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT10S""#,
        &with_period(&format!(
            "{}{}",
            protected(r#"contentType="video" mimeType="video/mp4""#, "drm", 1, drm),
            set(
                r#"contentType="video" mimeType="video/mp4""#,
                &[("plain", 2, "")]
            )
        )),
    );
    let s = select(&m, 0, &Policy::default());
    assert!(
        s.dropped
            .iter()
            .any(|d| d.set == 0 && d.reason == DropReason::UnsupportedEncryption),
        "{:?}",
        s.dropped
    );
}

#[test]
fn base_url_availability_time_offsets_add_along_the_chain() {
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z""#,
        r#"<BaseURL availabilityTimeOffset="2.0">http://cdn.example/root/</BaseURL>
           <Period id="p" start="PT0S"><BaseURL availabilityTimeOffset="3.0">p/</BaseURL>
           <AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1" duration="2" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let c: &BaseCandidate =
        &dash_runtime::mpd::urls::base_candidates(&[&m.base_urls, &m.periods[0].base_urls])[0];
    assert_eq!(c.availability_time_offset_s, 5.0);
}
