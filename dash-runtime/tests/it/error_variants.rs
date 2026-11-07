use dash_runtime::address::{
    AddressError, PlanError, PlanOptions, base_index, plan_representation,
};
use dash_runtime::mpd::Mpd;
use dash_runtime::mpd::inherit::InheritError;
use dash_runtime::selection::RepRef;

fn mpd_full(kind: &str, period_attrs: &str, set_body: &str, representation_body: &str) -> Mpd {
    let xml = format!(
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="{kind}" mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
<Period {period_attrs}><AdaptationSet mimeType="video/mp4">{set_body}<Representation id="v" bandwidth="1000" codecs="avc1.42C01E">{representation_body}</Representation></AdaptationSet></Period>
</MPD>"#
    );
    Mpd::parse(xml.as_bytes()).expect("test MPD parses")
}

fn mpd_with(kind: &str, representation_body: &str) -> Mpd {
    mpd_full(kind, "", "", representation_body)
}

fn plan(mpd: &Mpd) -> Result<(), PlanError> {
    plan_representation(mpd, RepRef::at(0, 0, 0), &PlanOptions::default()).map(|_| ())
}

fn static_plan(representation_body: &str) -> Result<(), PlanError> {
    plan(&mpd_with("static", representation_body))
}

#[test]
fn a_template_and_a_list_on_one_chain_cannot_be_inherited() {
    let mpd = mpd_full(
        "static",
        "",
        r#"<SegmentTemplate media="a$Number$.m4s" duration="2" timescale="1"/>"#,
        r#"<SegmentList duration="2"><SegmentURL media="a"/></SegmentList>"#,
    );
    assert_eq!(
        plan(&mpd),
        Err(PlanError::Inherit(InheritError::TemplateAndList))
    );
}

#[test]
fn a_duration_and_a_timeline_on_one_template_cannot_be_inherited() {
    let body = r#"<SegmentTemplate media="a$Number$.m4s" duration="2" timescale="1"><SegmentTimeline><S d="2"/></SegmentTimeline></SegmentTemplate>"#;
    assert_eq!(
        static_plan(body),
        Err(PlanError::Inherit(InheritError::DurationAndTimeline))
    );
}

#[test]
fn a_template_without_media_has_nothing_to_address() {
    assert_eq!(
        static_plan(r#"<SegmentTemplate duration="2" timescale="1"/>"#),
        Err(PlanError::Address(AddressError::MissingMediaTemplate))
    );
}

#[test]
fn a_rep_ref_outside_the_mpd_is_no_such_representation() {
    let mpd = mpd_with("static", "");
    assert_eq!(
        plan_representation(&mpd, RepRef::at(5, 0, 0), &PlanOptions::default()).map(|_| ()),
        Err(PlanError::NoSuchRepresentation)
    );
}

#[test]
fn a_dynamic_mpd_without_availability_start_time_cannot_be_planned() {
    let mpd = mpd_full(
        "dynamic",
        r#"start="PT0S""#,
        "",
        r#"<SegmentTemplate media="a$Number$.m4s" duration="2" timescale="1"/>"#,
    );
    assert_eq!(plan(&mpd), Err(PlanError::MissingAvailabilityStartTime));
}

#[test]
fn a_multi_entry_list_without_duration_or_timeline_is_missing_its_duration() {
    let body = r#"<SegmentList><SegmentURL media="a"/><SegmentURL media="b"/></SegmentList>"#;
    assert_eq!(
        static_plan(body),
        Err(PlanError::Address(AddressError::MissingDuration))
    );
}

#[test]
fn a_segment_url_with_neither_media_nor_range_is_empty() {
    let body = r#"<SegmentList duration="2"><SegmentURL/></SegmentList>"#;
    assert_eq!(
        static_plan(body),
        Err(PlanError::Address(AddressError::EmptySegmentUrl))
    );
}

#[test]
fn a_list_and_its_timeline_must_agree_on_the_segment_count() {
    let body = r#"<SegmentList><SegmentTimeline><S d="2"/></SegmentTimeline><SegmentURL media="a"/><SegmentURL media="b"/></SegmentList>"#;
    assert_eq!(
        static_plan(body),
        Err(PlanError::Address(AddressError::ListTimelineMismatch {
            urls: 2,
            segments: 1,
        }))
    );
}

#[test]
fn a_list_duration_that_overflows_the_segment_start_is_rejected() {
    let body = r#"<SegmentList duration="9223372036854775808"><SegmentURL media="a"/><SegmentURL media="b"/><SegmentURL media="c"/></SegmentList>"#;
    assert_eq!(
        static_plan(body),
        Err(PlanError::Address(AddressError::Overflow))
    );
}

#[test]
fn bytes_that_are_not_a_sidx_box_are_a_sidx_error() {
    match base_index::parse_index(&[0u8; 4]) {
        Err(AddressError::Sidx(message)) => assert_eq!(
            message,
            "buffer too short: need 16 bytes, have 4 (while parsing sidx box)"
        ),
        other => panic!("expected AddressError::Sidx, got {other:?}"),
    }
}
