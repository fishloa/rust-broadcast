//! Hand-computed vectors for the formulas of DESIGN 4.3 and 5.3, including
//! the DASH-IF IOP v4.3 Table 10 worked example.

mod common;

use dash_runtime::address::template::{
    TemplateFault, TemplateUse, scan, scan_initialization, scan_media, substitute,
};
use dash_runtime::address::timeline::TimelineIndex;
use dash_runtime::address::{AddressError, PlanOptions, plan_representation};
use dash_runtime::mpd::urls::{
    append_query, base_candidates, representation_candidates, segment_url_query,
};
use dash_runtime::mpd::{Mpd, SegmentTimeline, TimelineEntry};
use dash_runtime::selection::RepRef;
use dash_runtime::timing::datetime::parse_xs_datetime;
use dash_runtime::timing::live_edge::{
    join_time_us, newest_complete, oldest_in_window, presentation_delay_us,
};
use dash_runtime::timing::period::{PeriodKind, period_timings};
use url::Url;

const S: i128 = 1_000_000;
const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn mpd(attrs: &str, body: &str) -> Mpd {
    let xml = format!(r#"<?xml version="1.0"?><MPD xmlns="{NS}" {attrs}>{body}</MPD>"#);
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

fn unix(text: &str) -> i128 {
    i128::from(parse_xs_datetime(text).unwrap().unix_us)
}

fn table10() -> Mpd {
    mpd(
        r#"type="dynamic" availabilityStartTime="2026-10-26T00:00:00Z" mediaPresentationDuration="PT43S"
           timeShiftBufferDepth="PT25S" suggestedPresentationDelay="PT15S" minBufferTime="PT5S""#,
        r#"<Period id="p" start="PT0S"><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1000" duration="5000" media="$RepresentationID$/$Number$.m4s"
                            initialization="$RepresentationID$/init.mp4"/>
           <Representation id="v" bandwidth="1000"/></AdaptationSet></Period>"#,
    )
}

#[test]
fn iop_table_10_worked_example() {
    let start = unix("2026-10-26T00:00:00Z");
    let m = table10();
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    // k1 = 1, k2 = ceil(43 / 5) = 9.
    assert_eq!(p.plan.len(), Some(9));
    for k in 1..=8i128 {
        let seg = p.plan.get(u64::try_from(k - 1).unwrap()).unwrap();
        assert_eq!(seg.number, u64::try_from(k).unwrap());
        assert_eq!(seg.reference, format!("v/{k}.m4s"));
        assert_eq!(seg.available_from_us, Some(start + 5 * k * S), "SAST[{k}]");
        assert_eq!(
            seg.available_until_us,
            Some(start + 5 * k * S + 30 * S),
            "SAET[{k}]"
        );
    }
    // The 9th segment is 3 s long (43 s - 40 s). 5.3.9.5.3 uses its own
    // duration; IOP Table 10 prints the nominal 5 s (START + 45 s, +30 s).
    let last = p.plan.get(8).unwrap();
    assert_eq!(last.duration_us, 3 * S);
    assert_eq!(last.available_from_us, Some(start + 43 * S));
    assert_eq!(
        last.available_until_us,
        Some(start + 40 * S + 6 * S + 25 * S)
    );
    let init = p.plan.init().unwrap();
    assert_eq!(init.reference, "v/init.mp4");
    assert_eq!(init.available_from_us, Some(start));
    assert_eq!(init.available_until_us, last.available_until_us);
    // SPD = 15 s is inside TSB - 2 x 5 s = 15 s.
    let d = presentation_delay_us(m.suggested_presentation_delay, 5 * S, Some(25 * S), None);
    assert_eq!(d, 15 * S);
}

#[test]
fn newest_and_oldest_follow_the_closed_form_on_an_open_ended_plan() {
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="2026-10-26T00:00:00Z" timeShiftBufferDepth="PT25S""#,
        r#"<Period id="p" start="PT0S"><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1000" duration="5000" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let start = unix("2026-10-26T00:00:00Z");
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert!(p.plan.len().is_none());
    assert_eq!(newest_complete(&p.plan, start + 4 * S), None);
    assert_eq!(newest_complete(&p.plan, start + 5 * S), Some(0));
    assert_eq!(newest_complete(&p.plan, start + 43 * S), Some(7));
    // At START + 43 s: segment k ends at 5k+5+25 >= 43 for k >= 3 (index 2).
    assert_eq!(oldest_in_window(&p.plan, start + 43 * S), Some(2));
    assert_eq!(join_time_us(start + 43 * S, 15 * S, start, 0), 28 * S);
}

#[test]
fn derived_presentation_delay_and_clamp() {
    let d = |spd: Option<u64>, max: i128, tsb: Option<i128>, over: Option<i128>| {
        presentation_delay_us(
            spd.map(std::time::Duration::from_secs),
            max * S,
            tsb.map(|t| t * S),
            over.map(|o| o * S),
        ) / S
    };
    assert_eq!(d(None, 1, None, None), 4, "never under four seconds");
    assert_eq!(d(None, 2, None, None), 6, "three segment durations");
    assert_eq!(d(Some(9), 2, None, None), 9, "a suggested delay wins");
    assert_eq!(d(None, 2, Some(10), None), 6);
    assert_eq!(
        d(None, 2, Some(8), None),
        4,
        "clamped to TSB - 2 x maxSegmentDuration"
    );
    assert_eq!(d(Some(30), 2, Some(20), None), 16);
    assert_eq!(d(None, 2, Some(3), None), 0, "never negative");
    assert_eq!(
        d(Some(9), 2, Some(8), Some(3)),
        3,
        "an explicit override replaces it"
    );
    assert_eq!(d(None, 2, None, Some(-5)), 0);
}

#[test]
fn start_number_zero_and_one() {
    for (attr, first) in [
        (r#"startNumber="0""#, 0u64),
        ("", 1),
        (r#"startNumber="7""#, 7),
    ] {
        let m = mpd(
            r#"type="static" mediaPresentationDuration="PT10S""#,
            &format!(
                r#"<Period><AdaptationSet mimeType="video/mp4">
                   <SegmentTemplate timescale="1" duration="2" {attr} media="s$Number%03d$.m4s"/>
                   <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#
            ),
        );
        let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
        assert_eq!(p.plan.len(), Some(5));
        let seg = p.plan.get(0).unwrap();
        assert_eq!(seg.number, first);
        assert_eq!(seg.reference, format!("s{first:03}.m4s"));
        assert_eq!(seg.start_us, 0);
        assert_eq!(p.plan.get(4).unwrap().number, first + 4);
        assert_eq!(p.plan.get(4).unwrap().start_us, 8 * S);
    }
}

#[test]
fn end_number_caps_and_a_short_last_segment_is_shortened() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT11S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1" duration="2" endNumber="4" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert_eq!(p.plan.len(), Some(4));
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT11S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1" duration="2" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert_eq!(p.plan.len(), Some(6));
    assert_eq!(
        p.plan.get(5).unwrap().duration_us,
        S,
        "A.3.3 case 1: PeriodEnd - start"
    );
    assert_eq!(p.plan.get(4).unwrap().duration_us, 2 * S);
}

#[test]
fn single_segment_when_neither_duration_nor_timeline() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT9S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate media="whole-$Number$.m4s" startNumber="3"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert_eq!(p.plan.len(), Some(1));
    let seg = p.plan.get(0).unwrap();
    assert_eq!((seg.number, seg.start_us, seg.duration_us), (3, 0, 9 * S));
}

#[test]
fn period_start_rules_one_to_five() {
    // Rule 1 and 2: explicit start, then chained from the previous duration.
    let m = mpd(
        r#"type="static""#,
        r#"<Period id="a" start="PT10S" duration="PT5S"/><Period id="b" duration="PT7S"/><Period id="c"/>"#,
    );
    let t = period_timings(&m);
    assert_eq!(t[0].start_us, Some(10 * S));
    assert_eq!(t[1].start_us, Some(15 * S));
    assert_eq!(t[1].end_us, Some(22 * S));
    assert_eq!(t[2].start_us, Some(22 * S));
    // Rule 3: the first Period of a static MPD starts at 0.
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT30S""#,
        r#"<Period/>"#,
    );
    let t = period_timings(&m);
    assert_eq!((t[0].start_us, t[0].end_us), (Some(0), Some(30 * S)));
    // Rule 4: dynamic, no start, first Period: early available.
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="1970-01-01T00:00:00Z""#,
        r#"<Period id="x"/>"#,
    );
    let t = period_timings(&m);
    assert_eq!(
        (t[0].start_us, t[0].kind),
        (None, PeriodKind::EarlyAvailable)
    );
    // Rule 5: @duration and a next Period with @start (or an update period).
    let m = mpd(
        r#"type="static""#,
        r#"<Period start="PT0S" duration="PT4S"/><Period start="PT6S" duration="PT3S"/>"#,
    );
    let t = period_timings(&m);
    assert_eq!(t[0].kind, PeriodKind::EarlyTerminated);
    assert_eq!(
        t[0].end_us,
        Some(4 * S),
        "ends at its own duration, not at the next start"
    );
    assert_eq!(t[1].kind, PeriodKind::Regular);
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="1970-01-01T00:00:00Z" minimumUpdatePeriod="PT2S""#,
        r#"<Period start="PT0S" duration="PT4S"/>"#,
    );
    assert_eq!(period_timings(&m)[0].kind, PeriodKind::EarlyTerminated);
    // Regular: ends where the next one starts.
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT9S""#,
        r#"<Period start="PT0S"/><Period start="PT6S"/>"#,
    );
    let t = period_timings(&m);
    assert_eq!((t[0].end_us, t[1].end_us), (Some(6 * S), Some(9 * S)));
}

fn timeline(entries: &[(Option<u64>, Option<u64>, u64, i64)]) -> SegmentTimeline {
    let mut tl = SegmentTimeline::default();
    for &(t, n, d, r) in entries {
        let mut e = TimelineEntry::new(d);
        e.t = t;
        e.n = n;
        e.r = r;
        tl.entries.push(e);
    }
    tl
}

#[test]
fn timeline_numbers_gaps_negative_r_and_n() {
    // Plain: t=100 d=10 r=2, then a gap to t=200 d=5.
    let ix = TimelineIndex::build(
        &timeline(&[(Some(100), None, 10, 2), (Some(200), None, 5, 0)]),
        1,
        None,
    )
    .unwrap();
    let all: Vec<_> = ix.iter().map(|s| (s.number, s.time, s.duration)).collect();
    assert_eq!(
        all,
        vec![(1, 100, 10), (2, 110, 10), (3, 120, 10), (4, 200, 5)]
    );
    // Negative r runs to the next S@t: ceil((205 - 100) / 10) = 11 segments.
    let ix = TimelineIndex::build(
        &timeline(&[(Some(100), None, 10, -1), (Some(205), None, 5, 0)]),
        1,
        None,
    )
    .unwrap();
    assert_eq!(ix.len(), 12);
    assert_eq!(ix.segment(10).unwrap().time, 200);
    assert_eq!(
        ix.segment(11).unwrap().time,
        205,
        "the next S@t restarts the clock"
    );
    // Trailing negative r runs to the Period end (media timeline).
    let ix = TimelineIndex::build(&timeline(&[(Some(0), None, 10, -1)]), 1, Some(95)).unwrap();
    assert_eq!(ix.len(), 10, "ceil(95 / 10)");
    let ix = TimelineIndex::build(&timeline(&[(Some(0), None, 10, -1)]), 1, Some(100)).unwrap();
    assert_eq!(ix.len(), 10);
    // Without an end it is a single occurrence.
    let ix = TimelineIndex::build(&timeline(&[(Some(0), None, 10, -1)]), 1, None).unwrap();
    assert_eq!(ix.len(), 1);
    // @n restarts the numbering.
    let ix = TimelineIndex::build(
        &timeline(&[(Some(0), None, 10, 1), (None, Some(50), 10, 1)]),
        1,
        None,
    )
    .unwrap();
    let nums: Vec<u64> = ix.iter().map(|s| s.number).collect();
    assert_eq!(nums, vec![1, 2, 50, 51]);
    // First S without @t starts at 0; later ones continue.
    let ix = TimelineIndex::build(
        &timeline(&[(None, None, 10, 1), (None, None, 5, 0)]),
        0,
        None,
    )
    .unwrap();
    let times: Vec<u64> = ix.iter().map(|s| s.time).collect();
    assert_eq!(times, vec![0, 10, 20]);
    // index_at_or_before finds the containing segment, and None before the first.
    let ix = TimelineIndex::build(&timeline(&[(Some(100), None, 10, 2)]), 1, None).unwrap();
    assert_eq!(ix.index_at_or_before(99), None);
    assert_eq!(ix.index_at_or_before(100), Some(0));
    assert_eq!(ix.index_at_or_before(119), Some(1));
    assert_eq!(ix.index_at_or_before(10_000), Some(2));
}

#[test]
fn timeline_rejects_going_backwards_and_bounds_the_count() {
    let e = TimelineIndex::build(
        &timeline(&[(Some(100), None, 10, 1), (Some(110), None, 10, 0)]),
        1,
        None,
    );
    assert!(matches!(
        e,
        Err(AddressError::TimelineNotIncreasing { t: 110, floor: 120 })
    ));
    let e = TimelineIndex::build(
        &timeline(&[(Some(0), None, 10, 1), (None, Some(1), 10, 0)]),
        1,
        None,
    );
    assert!(matches!(e, Err(AddressError::NumberNotIncreasing { .. })));
    let ok = TimelineIndex::build(&timeline(&[(Some(0), None, 1, 999_999)]), 0, None).unwrap();
    assert_eq!(ok.len(), 1_000_000);
    assert_eq!(ok.segment(999_999).unwrap().time, 999_999);
    assert!(ok.segment(1_000_000).is_none());
    let too_long = TimelineIndex::build(&timeline(&[(Some(0), None, 1, 1_000_000)]), 0, None);
    assert!(matches!(
        too_long,
        Err(AddressError::TimelineTooLong { .. })
    ));
    let huge = TimelineIndex::build(&timeline(&[(Some(0), None, 1, i64::MAX)]), 0, None);
    assert!(matches!(huge, Err(AddressError::TimelineTooLong { .. })));
}

#[test]
fn template_widths_escapes_and_invalid_forms() {
    let sub = |t: &str| substitute(t, "rep1", Some(7), Some(1234), 99);
    assert_eq!(sub("a$Number$"), "a7");
    assert_eq!(sub("a$Number%05d$"), "a00007");
    assert_eq!(sub("a$Number%02d$"), "a07");
    assert_eq!(sub("a$Number%01d$"), "a7");
    assert_eq!(sub("a$Number%010d$.m4s"), "a0000000007.m4s");
    assert_eq!(sub("$RepresentationID$/$Time$"), "rep1/1234");
    assert_eq!(sub("$Bandwidth%06d$"), "000099");
    assert_eq!(sub("cost$$5"), "cost$5");
    assert_eq!(sub("$$Number$$"), "$Number$");
    assert!(scan("a$Number%05d$").unwrap().number);
    let u: TemplateUse = scan("$RepresentationID$/$Bandwidth$/$Time$").unwrap();
    assert!(u.representation_id && u.bandwidth && u.time && !u.number);
    assert!(scan("$SubNumber$").unwrap().sub_number);
    assert_eq!(
        scan("$Number").unwrap_err(),
        TemplateFault::UnterminatedIdentifier
    );
    assert_eq!(
        scan("$Numbr$").unwrap_err(),
        TemplateFault::UnknownIdentifier
    );
    assert_eq!(
        scan("$number$").unwrap_err(),
        TemplateFault::UnknownIdentifier
    );
    assert_eq!(
        scan("$RepresentationID%05d$").unwrap_err(),
        TemplateFault::FormatOnRepresentationId
    );
    assert_eq!(
        scan("$Number%5d$").unwrap_err(),
        TemplateFault::BadFormatTag
    );
    assert_eq!(
        scan("$Number%05x$").unwrap_err(),
        TemplateFault::BadFormatTag
    );
    assert_eq!(
        scan("$Number%0999d$").unwrap_err(),
        TemplateFault::WidthTooLarge
    );
    assert!(scan("$Number%020d$").is_ok(), "the largest supported width");
    assert_eq!(
        scan("$Number%021d$").unwrap_err(),
        TemplateFault::WidthTooLarge
    );
    let escaped = scan("cost$$5-$$Number$$").unwrap();
    assert!(
        !escaped.number && !escaped.time,
        "$$ is an escape, not an identifier"
    );
    assert_eq!(
        scan("$$$Number$").map(|u| u.number),
        Ok(true),
        "an escape followed by an identifier"
    );
    assert_eq!(
        scan_media("a.m4s").unwrap_err(),
        TemplateFault::MissingNumberOrTime
    );
    assert_eq!(
        scan_media("$Number$$Time$").unwrap_err(),
        TemplateFault::NumberAndTime
    );
    assert_eq!(
        scan_initialization("i$Number$").unwrap_err(),
        TemplateFault::DynamicIdentifierInInitialization
    );
    assert!(scan_media("$Time$").is_ok());
}

#[test]
fn invalid_template_drops_the_representation_not_the_mpd() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT4S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <Representation id="bad" bandwidth="1"><SegmentTemplate duration="2" timescale="1" media="$Nope$.m4s"/></Representation>
           <Representation id="good" bandwidth="2"><SegmentTemplate duration="2" timescale="1" media="$Number$.m4s"/></Representation>
           </AdaptationSet></Period>"#,
    );
    let bad = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default());
    assert!(matches!(
        bad,
        Err(dash_runtime::address::PlanError::Address(
            AddressError::InvalidTemplate { .. }
        ))
    ));
    assert!(plan_representation(&m, RepRef::at(0, 0, 1), &PlanOptions::default()).is_ok());
    let seq = mpd(
        r#"type="static" mediaPresentationDuration="PT4S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">
           <SegmentTemplate duration="2" timescale="1" media="$Number$-$SubNumber$.m4s"/></Representation></AdaptationSet></Period>"#,
    );
    assert!(matches!(
        plan_representation(&seq, RepRef::at(0, 0, 0), &PlanOptions::default()),
        Err(dash_runtime::address::PlanError::Address(
            AddressError::Unsupported(dash_runtime::error::Unsupported::SegmentSequence)
        ))
    ));
}

#[test]
fn inheritance_lower_level_wins_attribute_by_attribute() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT6S""#,
        r#"<Period><SegmentTemplate timescale="1000" startNumber="5" media="p-$Number$.m4s" duration="2000"/>
           <AdaptationSet mimeType="video/mp4"><SegmentTemplate startNumber="9"/>
           <Representation id="a" bandwidth="1"/>
           <Representation id="b" bandwidth="2"><SegmentTemplate media="b-$Number$.m4s"/></Representation>
           </AdaptationSet></Period>"#,
    );
    let a = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    let b = plan_representation(&m, RepRef::at(0, 0, 1), &PlanOptions::default()).unwrap();
    assert_eq!(a.plan.get(0).unwrap().reference, "p-9.m4s");
    assert_eq!(b.plan.get(0).unwrap().reference, "b-9.m4s");
    assert_eq!(a.plan.len(), Some(3));
    assert_eq!(a.plan.timescale(), 1000);
    // A level that introduces a timeline does not inherit @duration.
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT6S""#,
        r#"<Period><SegmentTemplate timescale="1" duration="2" media="$Number$.m4s"/>
           <AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">
           <SegmentTemplate><SegmentTimeline><S t="0" d="3" r="1"/></SegmentTimeline></SegmentTemplate>
           </Representation></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert_eq!(p.plan.len(), Some(2));
    assert_eq!(p.plan.get(1).unwrap().start_us, 3 * S);
    // A SegmentList below a SegmentTemplate is a violation.
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT6S""#,
        r#"<Period><SegmentTemplate duration="2" media="$Number$.m4s"/>
           <AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">
           <SegmentList duration="2"><SegmentURL media="a.m4s"/></SegmentList>
           </Representation></AdaptationSet></Period>"#,
    );
    assert!(matches!(
        plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()),
        Err(dash_runtime::address::PlanError::Inherit(_))
    ));
}

#[test]
fn base_url_join_override_alternates_and_query() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT2S""#,
        r#"<BaseURL>http://cdn.example/root/</BaseURL>
           <Period><BaseURL>p1/</BaseURL><AdaptationSet mimeType="video/mp4">
           <Representation id="r" bandwidth="1"><BaseURL>rep/</BaseURL>
           <SegmentTemplate duration="2" timescale="1" media="$Number$.m4s"/></Representation>
           <Representation id="abs" bandwidth="1"><BaseURL>http://other.example/z/</BaseURL>
           <SegmentTemplate duration="2" timescale="1" media="$Number$.m4s"/></Representation>
           </AdaptationSet></Period>"#,
    );
    let doc = Url::parse("http://origin.example/live/stream.mpd?token=secret").unwrap();
    let (p, s, r) = RepRef::at(0, 0, 0).resolve(&m).unwrap();
    let c = representation_candidates(&m, p, s, r);
    assert_eq!(
        c[0].resolve(Some(&doc), "1.m4s").unwrap(),
        "http://cdn.example/root/p1/rep/1.m4s",
        "RFC 3986 join; the MPD URL query is not propagated"
    );
    let (p, s, r) = RepRef::at(0, 0, 1).resolve(&m).unwrap();
    let c = representation_candidates(&m, p, s, r);
    assert_eq!(
        c[0].resolve(Some(&doc), "1.m4s").unwrap(),
        "http://other.example/z/1.m4s"
    );
    // No BaseURL at all: resolves against the MPD URL (query dropped).
    let m2 = mpd(r#"type="static""#, r#"<Period/>"#);
    let c = base_candidates(&[&m2.base_urls]);
    assert_eq!(
        c[0].resolve(Some(&doc), "seg.m4s").unwrap(),
        "http://origin.example/live/seg.m4s"
    );
    assert_eq!(append_query("http://h/a.m4s", "q=1"), "http://h/a.m4s?q=1");
    assert_eq!(
        append_query("http://h/a.m4s?x=2", "q=1"),
        "http://h/a.m4s?x=2&q=1"
    );
}

#[test]
fn base_url_alternates_are_ordered_by_priority_then_document_order() {
    let m = mpd(
        r#"xmlns:dvb="urn:dvb:dash:dash-extensions:2014-1" type="static""#,
        r#"<BaseURL dvb:priority="2" serviceLocation="b">http://b.example/</BaseURL>
           <BaseURL dvb:priority="1" serviceLocation="a">http://a.example/</BaseURL>
           <BaseURL serviceLocation="c">http://c.example/</BaseURL>
           <Period><BaseURL serviceLocation="b">pb/</BaseURL><BaseURL serviceLocation="a">pa/</BaseURL></Period>"#,
    );
    let c = base_candidates(&[&m.base_urls, &m.periods[0].base_urls]);
    let urls: Vec<String> = c.iter().map(|c| c.resolve(None, "x").unwrap()).collect();
    assert_eq!(
        urls,
        vec![
            "http://b.example/pb/x".to_owned(),
            "http://a.example/pa/x".to_owned(),
        ],
        "Period alternates pair with the MPD alternate of the same serviceLocation, sorted by their own priority (default 1, document order)"
    );
    let top = base_candidates(&[&m.base_urls]);
    let order: Vec<_> = top
        .iter()
        .map(|c| c.service_location.clone().unwrap())
        .collect();
    assert_eq!(order, vec!["a", "c", "b"]);
}

#[test]
fn url_query_info_minimal_form() {
    let xml = |info: &str| {
        mpd(
            r#"type="static" mediaPresentationDuration="PT2S""#,
            &format!(
                r#"<Period><AdaptationSet mimeType="video/mp4">
                   <SupplementalProperty schemeIdUri="urn:mpeg:dash:urlparam:2014" xmlns:up="urn:mpeg:dash:schema:urlparam:2014">
                   <up:UrlQueryInfo {info}/></SupplementalProperty>
                   <Representation id="r" bandwidth="1"><SegmentTemplate duration="2" timescale="1" media="$Number$.m4s"/></Representation>
                   </AdaptationSet></Period>"#
            ),
        )
    };
    let get = |m: &Mpd, mpd_query: Option<&str>| {
        let (p, s, r) = RepRef::at(0, 0, 0).resolve(m).unwrap();
        segment_url_query(m, p, s, r, mpd_query)
    };
    let m = xml(r#"useMPDUrlQuery="true" queryTemplate="$querypart$""#);
    assert_eq!(
        get(&m, Some("token=1&ip=2")).unwrap().as_deref(),
        Some("token=1&ip=2")
    );
    let m = xml(r#"queryString="a=1""#);
    assert_eq!(get(&m, Some("token=1")).unwrap().as_deref(), Some("a=1"));
    let m = xml(r#"useMPDUrlQuery="true" queryString="a=1""#);
    assert_eq!(
        get(&m, Some("token=1")).unwrap().as_deref(),
        Some("token=1&a=1")
    );
    let m = xml(r#"useMPDUrlQuery="true" queryTemplate="token=$query:token$""#);
    assert!(
        get(&m, Some("token=1")).is_err(),
        "other templates are not understood"
    );
    let none = mpd(r#"type="static""#, r#"<Period/>"#);
    let m = xml(r#"useMPDUrlQuery="true""#);
    let _ = none;
    assert_eq!(get(&m, None).unwrap(), None);
}

#[test]
fn time_substitution_is_the_raw_s_at_even_with_a_presentation_time_offset() {
    // Decision D3 (docs/dash-runtime/DESIGN.md section 0 and 11): the standard's
    // three statements disagree when @presentationTimeOffset != 0; `$Time$`
    // takes the raw S@t, the Period-relative start takes S@t - PTO.
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT4S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">
           <SegmentTemplate timescale="1000" presentationTimeOffset="5000" media="$Time$.m4s">
           <SegmentTimeline><S t="5000" d="2000" r="1"/></SegmentTimeline></SegmentTemplate>
           </Representation></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    let first = p.plan.get(0).unwrap();
    assert_eq!(first.reference, "5000.m4s");
    assert_eq!(first.start_us, 0);
    assert_eq!(p.plan.get(1).unwrap().reference, "7000.m4s");
    assert_eq!(p.plan.get(1).unwrap().start_us, 2 * S);
}

#[test]
fn end_number_truncates_a_timeline() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT10S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">
           <SegmentTemplate timescale="1" endNumber="4" media="$Number$.m4s">
           <SegmentTimeline><S t="0" d="1" r="2"/><S d="1" r="5"/></SegmentTimeline></SegmentTemplate>
           </Representation></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert_eq!(
        p.plan.len(),
        Some(4),
        "a segment numbered above @endNumber is invalid"
    );
    assert_eq!(p.plan.get(3).unwrap().number, 4);
    assert!(p.plan.get(4).is_none());
}

#[test]
fn utc_timing_schemes_are_classified_from_the_real_mpd() {
    use dash_runtime::timing::utc_timing::UtcScheme;
    let l = common::load("dash/livesim2/segtimeline_time_a.mpd");
    let u = &l.mpd.utc_timings[0];
    assert_eq!(u.scheme, UtcScheme::HttpXsdate);
    assert_eq!(
        u.values().collect::<Vec<_>>(),
        ["https://time.akamai.com/?iso&ms"]
    );
    for (uri, scheme) in [
        ("urn:mpeg:dash:utc:ntp:2014", UtcScheme::Ntp),
        ("urn:mpeg:dash:utc:sntp:2014", UtcScheme::Sntp),
        ("urn:mpeg:dash:utc:http-head:2014", UtcScheme::HttpHead),
        ("urn:mpeg:dash:utc:http-xsdate:2014", UtcScheme::HttpXsdate),
        ("urn:mpeg:dash:utc:http-iso:2014", UtcScheme::HttpIso),
        ("urn:mpeg:dash:utc:http-ntp:2014", UtcScheme::HttpNtp),
        ("urn:mpeg:dash:utc:direct:2014", UtcScheme::Direct),
        ("urn:other", UtcScheme::Unknown),
    ] {
        assert_eq!(UtcScheme::from_scheme_id_uri(uri), scheme, "{uri}");
    }
    assert!(UtcScheme::HttpHead.needs_request() && !UtcScheme::Direct.needs_request());
    assert!(UtcScheme::Direct.is_actionable() && !UtcScheme::Ntp.is_actionable());
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="1970-01-01T00:00:00Z""#,
        r#"<UTCTiming schemeIdUri="urn:mpeg:dash:utc:http-head:2014" value="http://a/t http://b/t"/>
           <UTCTiming schemeIdUri="urn:mpeg:dash:utc:direct:2014" value="2026-10-26T08:39:04Z"/>"#,
    );
    assert_eq!(
        m.utc_timings.len(),
        2,
        "alternatives are kept in document order"
    );
    assert_eq!(m.utc_timings[0].values().count(), 2);
    assert_eq!(m.utc_timings[1].scheme, UtcScheme::Direct);
}

#[test]
fn timescale_defaults_to_one_when_no_level_declares_it() {
    // DESIGN section 11 item 7: livesim2's testpic_2s has `duration="2"` and no
    // @timescale at any level.
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT6S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate duration="2" startNumber="0" media="$RepresentationID$/$Number$.m4s"/>
           <Representation id="V300" bandwidth="300000"/></AdaptationSet></Period>"#,
    );
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    assert_eq!(p.plan.timescale(), 1);
    assert_eq!(p.plan.len(), Some(3));
    assert_eq!(p.plan.get(1).unwrap().start_us, 2 * S);
    assert_eq!(p.plan.get(1).unwrap().duration_us, 2 * S);
    assert_eq!(p.plan.get(2).unwrap().reference, "V300/2.m4s");
}

fn period_ids(ids: &[&str]) -> Result<Mpd, dash_runtime::error::MpdError> {
    let periods: String = ids
        .iter()
        .map(|id| format!(r#"<Period id="{id}"/>"#))
        .collect();
    let xml = format!(r#"<?xml version="1.0"?><MPD xmlns="{NS}">{periods}</MPD>"#);
    Mpd::parse(xml.as_bytes())
}

#[test]
fn two_periods_may_not_share_an_id() {
    use dash_runtime::error::MpdError;

    assert!(period_ids(&["a", "b", "c"]).is_ok());
    let err = period_ids(&["a", "b", "a"]).unwrap_err();
    assert!(
        matches!(&err, MpdError::Invalid { what: "Period@id", detail } if detail.contains('a')),
        "{err}"
    );
    let no_id = format!(r#"<?xml version="1.0"?><MPD xmlns="{NS}"><Period/><Period/></MPD>"#);
    assert!(
        Mpd::parse(no_id.as_bytes()).is_ok(),
        "Periods without an id never collide"
    );
}
