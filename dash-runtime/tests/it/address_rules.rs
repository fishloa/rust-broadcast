//! Segment addressing arithmetic the MPDs of the fixtures do not pin:
//! `@endNumber`, the short last segment of a Period, a negative `@r` cut at
//! the next `S@t`, `@presentationTimeOffset`, the dynamic availability window
//! and the per-level `@timeShiftBufferDepth`.

use dash_runtime::address::timeline::TimelineIndex;
use dash_runtime::address::{AddressError, PlanOptions, plan_representation};
use dash_runtime::mpd::{Mpd, SegmentTimeline, TimelineEntry};
use dash_runtime::selection::RepRef;
use dash_runtime::timing::datetime::parse_xs_datetime;

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn mpd(attrs: &str, body: &str) -> Mpd {
    let xml = format!(r#"<?xml version="1.0"?><MPD xmlns="{NS}" {attrs}>{body}</MPD>"#);
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

fn static_video(template: &str) -> Mpd {
    mpd(
        r#"type="static" mediaPresentationDuration="PT6S""#,
        &format!(
            r#"<Period><AdaptationSet mimeType="video/mp4">{template}
               <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#
        ),
    )
}

fn plan(m: &Mpd) -> dash_runtime::address::SegmentPlan {
    plan_representation(m, RepRef::at(0, 0, 0), &PlanOptions::default())
        .unwrap_or_else(|e| panic!("{e}"))
        .plan
}

fn unix(text: &str) -> i128 {
    i128::from(parse_xs_datetime(text).unwrap().unix.as_microsecond())
}

fn utc(us: i128) -> jiff::Timestamp {
    jiff::Timestamp::from_microsecond(
        i64::try_from(us.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap(),
    )
    .unwrap()
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
fn end_number_bounds_the_count_and_a_lower_one_empties_the_plan() {
    let three = static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" startNumber="5" endNumber="7" media="$Number$.m4s"/>"#,
    );
    let p = plan(&three);
    assert_eq!(p.len(), Some(3));
    let nums: Vec<u64> = p.iter().map(|s| s.number).collect();
    assert_eq!(nums, vec![5, 6, 7]);
    let refs: Vec<String> = p.iter().map(|s| s.reference).collect();
    assert_eq!(refs, vec!["5.m4s", "6.m4s", "7.m4s"]);
    assert!(!p.is_empty());

    let one = static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" startNumber="5" endNumber="5" media="$Number$.m4s"/>"#,
    );
    let p = plan(&one);
    assert_eq!(p.len(), Some(1), "@endNumber == @startNumber keeps one");
    assert_eq!(p.get(0).unwrap().unwrap().number, 5);

    let none = static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" startNumber="5" endNumber="4" media="$Number$.m4s"/>"#,
    );
    let p = plan(&none);
    assert_eq!(p.len(), Some(0));
    assert!(p.is_empty());
    assert!(p.get(0).unwrap().is_none());
}

#[test]
fn the_last_segment_ends_at_the_period_duration() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT5S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1000" duration="2000" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan(&m);
    assert_eq!(p.len(), Some(3));
    let dur = |i: u64| p.get(i).unwrap().unwrap().duration.as_micros();
    assert_eq!(dur(0), 2_000_000);
    assert_eq!(dur(1), 2_000_000);
    assert_eq!(dur(2), 1_000_000, "5 s = 2 + 2 + 1");
}

#[test]
fn a_negative_r_stops_at_the_next_forward_t_and_keeps_a_short_tail() {
    let ix = TimelineIndex::build(
        &timeline(&[(Some(0), None, 100, -1), (Some(1050), None, 100, 0)]),
        1,
        None,
    )
    .unwrap();
    assert_eq!(ix.len(), 12, "eleven 100-tick segments then the next S");
    let tenth = ix.segment(10).unwrap();
    assert_eq!((tenth.number, tenth.time, tenth.duration), (11, 1000, 50));
    let last = ix.segment(11).unwrap();
    assert_eq!((last.number, last.time, last.duration), (12, 1050, 100));
}

#[test]
fn a_run_without_t_starts_where_the_previous_run_ends() {
    let ix = TimelineIndex::build(
        &timeline(&[
            (Some(0), None, 100, 1),
            (None, None, 50, 1),
            (None, None, 10, 0),
        ]),
        1,
        None,
    )
    .unwrap();
    let times: Vec<u64> = ix.iter().map(|s| s.time).collect();
    assert_eq!(times, vec![0, 100, 200, 250, 300]);
}

#[test]
fn a_next_t_well_before_the_run_start_is_rejected() {
    let e = TimelineIndex::build(
        &timeline(&[(Some(100), None, 100, -1), (Some(90), None, 100, 0)]),
        1,
        None,
    );
    assert!(matches!(
        e,
        Err(AddressError::TimelineNotIncreasing { t: 90, floor: 100 })
    ));
}

#[test]
fn a_number_equal_to_the_next_expected_one_is_accepted() {
    let ix = TimelineIndex::build(
        &timeline(&[(Some(0), None, 100, 1), (None, Some(3), 100, 0)]),
        1,
        None,
    )
    .unwrap();
    let nums: Vec<u64> = ix.iter().map(|s| s.number).collect();
    assert_eq!(nums, vec![1, 2, 3]);
}

#[test]
fn an_empty_timeline_has_no_segment_and_no_index_to_find() {
    let ix = TimelineIndex::build(&timeline(&[(Some(0), None, 100, -1)]), 1, Some(0)).unwrap();
    assert!(ix.is_empty());
    assert_eq!(ix.len(), 0);
    assert_eq!(ix.index_at_or_before(50), None);
    let ix = TimelineIndex::build(&timeline(&[(Some(0), None, 100, 1)]), 1, None).unwrap();
    assert!(!ix.is_empty());
    assert_eq!(ix.len(), 2);
}

#[test]
fn the_total_segment_bound_applies_across_runs() {
    let e = TimelineIndex::build(
        &timeline(&[(Some(0), None, 1, 599_999), (None, None, 1, 599_999)]),
        1,
        None,
    );
    assert!(matches!(e, Err(AddressError::TimelineTooLong { .. })));
}

#[test]
fn a_representation_time_offset_shifts_the_period_relative_start() {
    let m = static_video(
        r#"<SegmentTemplate timescale="1000" presentationTimeOffset="500" media="$Time$.m4s">
           <SegmentTimeline><S t="1000" d="2000" r="1"/></SegmentTimeline></SegmentTemplate>"#,
    );
    let p = plan(&m);
    assert_eq!(p.presentation_time_offset(), 500);
    assert_eq!(p.get(0).unwrap().unwrap().start.as_micros(), 500_000);
    assert_eq!(p.get(1).unwrap().unwrap().start.as_micros(), 2_500_000);

    let big = static_video(
        r#"<SegmentTemplate timescale="90000" presentationTimeOffset="90000" media="$Time$.m4s">
           <SegmentTimeline><S t="90000" d="180000" r="0"/></SegmentTimeline></SegmentTemplate>"#,
    );
    assert_eq!(plan(&big).presentation_time_offset(), 90_000);
}

#[test]
fn a_segment_list_shortens_its_last_entry_to_the_period_end() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT5S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentList timescale="1000" duration="2000">
           <SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/><SegmentURL media="c.m4s"/>
           </SegmentList><Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan(&m);
    assert_eq!(p.len(), Some(3));
    let durs: Vec<u128> = (0..3)
        .map(|i| p.get(i).unwrap().unwrap().duration.as_micros())
        .collect();
    assert_eq!(durs, vec![2_000_000, 2_000_000, 1_000_000]);
}

#[test]
fn a_single_segment_list_entry_needs_no_duration() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT5S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentList timescale="1000"><SegmentURL media="only.m4s"/></SegmentList>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan(&m);
    assert_eq!(p.len(), Some(1));
    let s = p.get(0).unwrap().unwrap();
    assert_eq!(s.reference, "only.m4s");
    assert_eq!(s.duration.as_micros(), 5_000_000);
}

#[test]
fn a_segment_list_timeline_is_shifted_by_the_presentation_time_offset() {
    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT4S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4">
           <SegmentList timescale="1000" presentationTimeOffset="500">
           <SegmentTimeline><S t="500" d="2000" r="1"/></SegmentTimeline>
           <SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/>
           </SegmentList><Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let p = plan(&m);
    assert_eq!(p.len(), Some(2));
    assert_eq!(p.get(0).unwrap().unwrap().start.as_micros(), 0);
    assert_eq!(p.get(1).unwrap().unwrap().start.as_micros(), 2_000_000);
}

#[test]
fn a_dynamic_plan_carries_the_availability_window() {
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" timeShiftBufferDepth="PT30S""#,
        r#"<Period id="p" start="PT10S"><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1000" duration="2000" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let ast = unix("2026-01-01T00:00:00Z");
    let p = plan(&m);
    assert!(p.is_dynamic());
    let from = |i: u64| p.available_from(i).unwrap().unwrap();
    let until = |i: u64| p.available_until(i).unwrap().unwrap();
    assert_eq!(from(0), utc(ast + 12_000_000), "AST + 10 s + 1 x 2 s");
    assert_eq!(from(3), utc(ast + 18_000_000), "AST + 10 s + 4 x 2 s");
    assert_eq!(
        until(0),
        utc(ast + 44_000_000),
        "AST + 10 s + 2 x 2 s + 30 s"
    );
    assert_eq!(
        until(3),
        utc(ast + 50_000_000),
        "AST + 10 s + 5 x 2 s + 30 s"
    );

    let static_p = plan(&static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" media="$Number$.m4s"/>"#,
    ));
    assert!(!static_p.is_dynamic());
    assert_eq!(static_p.available_from(0).unwrap(), None);
    assert_eq!(static_p.available_until(0).unwrap(), None);
}

#[test]
fn an_availability_time_offset_is_subtracted_when_opted_in() {
    let m = mpd(
        r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" timeShiftBufferDepth="PT30S""#,
        r#"<Period id="p" start="PT10S"><AdaptationSet mimeType="video/mp4">
           <SegmentTemplate timescale="1000" duration="2000" availabilityTimeOffset="1.0" media="$Number$.m4s"/>
           <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#,
    );
    let ast = unix("2026-01-01T00:00:00Z");
    let p = plan_representation(
        &m,
        RepRef::at(0, 0, 0),
        &PlanOptions::default().with_availability_time_offset(true),
    )
    .unwrap()
    .plan;
    assert_eq!(
        p.available_from(0).unwrap(),
        Some(utc(ast + 11_000_000)),
        "AST + 10 s + 2 s - 1 s"
    );
}

#[test]
fn an_availability_time_offset_is_read_from_each_segment_information_kind() {
    let dynamic = |segment: &str| {
        mpd(
            r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" timeShiftBufferDepth="PT30S""#,
            &format!(
                r#"<Period id="p" start="PT0S"><AdaptationSet mimeType="video/mp4">{segment}
                   <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#
            ),
        )
    };
    let ast = unix("2026-01-01T00:00:00Z");
    let at = RepRef::at(0, 0, 0);
    let early = |segment: &str| {
        plan_representation(
            &dynamic(segment),
            at,
            &PlanOptions::default().with_availability_time_offset(true),
        )
        .unwrap_or_else(|e| panic!("{e}"))
        .plan
    };
    let template = early(
        r#"<SegmentTemplate timescale="1000" duration="2000" availabilityTimeOffset="1.0" media="$Number$.m4s"/>"#,
    );
    assert_eq!(
        template.available_from(0).unwrap(),
        Some(utc(ast + 1_000_000)),
        "AST + 2 s - 1 s"
    );
    let list = early(
        r#"<SegmentList timescale="1000" duration="2000" availabilityTimeOffset="1.0">
           <SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/></SegmentList>"#,
    );
    assert_eq!(
        list.available_from(0).unwrap(),
        Some(utc(ast + 1_000_000)),
        "the list-level offset is applied"
    );
    let base = early(r#"<SegmentBase timescale="1000" availabilityTimeOffset="1.0"/>"#);
    assert_eq!(
        base.available_from(0).unwrap(),
        Some(utc(ast - 1_000_000)),
        "the base-level offset is applied to its single undated segment"
    );
}

#[test]
fn the_time_shift_buffer_depth_is_taken_from_each_segment_information_kind() {
    let dynamic = |segment: &str| {
        mpd(
            r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z""#,
            &format!(
                r#"<Period id="p" start="PT0S"><AdaptationSet mimeType="video/mp4">{segment}
                   <Representation id="v" bandwidth="1"/></AdaptationSet></Period>"#
            ),
        )
    };
    let ast = unix("2026-01-01T00:00:00Z");
    let at = RepRef::at(0, 0, 0);
    let template = plan_representation(
        &dynamic(
            r#"<SegmentTemplate timescale="1000" duration="2000" timeShiftBufferDepth="PT30S" media="$Number$.m4s"/>"#,
        ),
        at,
        &PlanOptions::default(),
    )
    .unwrap()
    .plan;
    assert_eq!(
        template.available_until(0).unwrap(),
        Some(utc(ast + 34_000_000)),
        "template-level depth: 0 + 2 x 2 s + 30 s"
    );
    let list = plan_representation(
        &dynamic(
            r#"<SegmentList timescale="1000" duration="2000" timeShiftBufferDepth="PT20S">
               <SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/></SegmentList>"#,
        ),
        at,
        &PlanOptions::default(),
    )
    .unwrap()
    .plan;
    assert_eq!(
        list.available_until(0).unwrap(),
        Some(utc(ast + 24_000_000)),
        "list-level depth: 0 + 2 x 2 s + 20 s"
    );
    let base = plan_representation(
        &dynamic(r#"<SegmentBase timescale="1000" timeShiftBufferDepth="PT10S"/>"#),
        at,
        &PlanOptions::default(),
    )
    .unwrap()
    .plan;
    assert!(
        base.available_until(0).unwrap().is_some(),
        "base-level depth is honoured"
    );
}

#[test]
fn a_representation_index_template_is_used_only_without_number_or_time() {
    let plain = static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" index="$RepresentationID$/idx.mp4" media="$Number$.m4s"/>"#,
    );
    let idx = plan(&plain).index().expect("a static index template");
    assert_eq!(idx.reference, "v/idx.mp4");

    let numbered = static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" index="idx-$Number$.mp4" media="$Number$.m4s"/>"#,
    );
    assert!(
        plan(&numbered).index().is_none(),
        "a per-segment index template is not one index resource"
    );

    let timed = static_video(
        r#"<SegmentTemplate timescale="1000" duration="2000" index="idx-$Time$.mp4" media="$Number$.m4s"/>"#,
    );
    assert!(plan(&timed).index().is_none());
}

#[test]
fn a_sidx_plan_numbers_its_subsegments_from_one() {
    use dash_runtime::address::{PlanOptions, base_index};
    use isobmff::timing::SegmentIndexBox;
    use isobmff::timing::SidxReference;

    let m = mpd(
        r#"type="static" mediaPresentationDuration="PT6S""#,
        r#"<Period><AdaptationSet mimeType="video/mp4"><Representation id="v" bandwidth="1">
           <SegmentBase timescale="1000" indexRange="0-99"/>
           </Representation></AdaptationSet></Period>"#,
    );
    let mut sidx = SegmentIndexBox {
        version: 0,
        flags: 0,
        reference_id: 1,
        timescale: broadcast_common::timescale::Timescale::try_new(1000).unwrap(),
        earliest_presentation_time: 0,
        first_offset: 0,
        references: Vec::new(),
    };
    for _ in 0..3 {
        sidx.references.push(SidxReference {
            reference_type: 0,
            referenced_size: 10,
            subsegment_duration: 1000,
            starts_with_sap: 1,
            sap_type: 0,
            sap_delta_time: 0,
        });
    }
    let plan = plan_representation(
        &m,
        RepRef::at(0, 0, 0),
        &PlanOptions::default().with_index(&sidx, 100),
    )
    .unwrap()
    .plan;
    assert_eq!(plan.len(), Some(3));
    let nums: Vec<u64> = plan.iter().map(|s| s.number).collect();
    assert_eq!(nums, vec![1, 2, 3]);
    let ranges: Vec<(u64, u64)> = plan
        .iter()
        .map(|s| {
            let r = s.byte_range.unwrap();
            (r.first, r.last.unwrap())
        })
        .collect();
    assert_eq!(ranges, vec![(100, 109), (110, 119), (120, 129)]);
    let _ = base_index::anchor_after;
}

#[test]
fn a_sidx_reference_to_another_sidx_is_refused() {
    use dash_runtime::address::base_index;
    use isobmff::timing::SegmentIndexBox;
    use isobmff::timing::SidxReference;

    let sidx = SegmentIndexBox {
        version: 0,
        flags: 0,
        reference_id: 1,
        timescale: broadcast_common::timescale::Timescale::try_new(1000).unwrap(),
        earliest_presentation_time: 0,
        first_offset: 0,
        references: vec![SidxReference {
            reference_type: 1,
            referenced_size: 10,
            subsegment_duration: 1000,
            starts_with_sap: 1,
            sap_type: 0,
            sap_delta_time: 0,
        }],
    };
    assert!(matches!(
        base_index::segments(&sidx, 100),
        Err(AddressError::Unsupported(
            dash_runtime::error::Unsupported::HierarchicalSidx
        ))
    ));
}

#[test]
fn a_sidx_segment_without_starts_with_sap_is_marked_so() {
    use dash_runtime::address::base_index;
    use isobmff::timing::SegmentIndexBox;
    use isobmff::timing::SidxReference;

    let reference = |sap: u8| SidxReference {
        reference_type: 0,
        referenced_size: 10,
        subsegment_duration: 1000,
        starts_with_sap: sap,
        sap_type: 0,
        sap_delta_time: 0,
    };
    let sidx = SegmentIndexBox {
        version: 0,
        flags: 0,
        reference_id: 1,
        timescale: broadcast_common::timescale::Timescale::try_new(1000).unwrap(),
        earliest_presentation_time: 0,
        first_offset: 0,
        references: vec![reference(1), reference(0)],
    };
    let segs = base_index::segments(&sidx, 100).unwrap();
    assert_eq!(segs.len(), 2);
    assert!(segs[0].starts_with_sap);
    assert!(!segs[1].starts_with_sap);
}
