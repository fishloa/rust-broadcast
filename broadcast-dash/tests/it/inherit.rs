//! Period -> AdaptationSet -> Representation inheritance (ISO/IEC 23009-1
//! 5.3.9.1, 5.3.7.2): a lower level wins field by field, and the lowest level
//! that lists SegmentURLs supplies them.

use broadcast_common::timescale::Timescale;
use broadcast_dash::Mpd;
use broadcast_dash::inherit::{SegmentInfo, effective_common, resolve_segment_info};
use broadcast_dash::limits::Limits;

fn parse(xml: &str) -> Mpd {
    Mpd::parse_with(xml.as_bytes(), &Limits::default()).unwrap_or_else(|e| panic!("{xml}: {e}"))
}

const HEAD: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" minBufferTime="PT1S" mediaPresentationDuration="PT1S"><Period id="0">"#;
const TAIL: &str = "</Period></MPD>";

#[test]
fn segment_template_fields_merge_from_every_level() {
    let xml = format!(
        r#"{HEAD}<SegmentTemplate duration="2" media="p$Number$.m4s"/><AdaptationSet mimeType="video/mp4"><SegmentTemplate timescale="90000" media="a$Number$.m4s"/><Representation id="r" bandwidth="1"><SegmentTemplate presentationTimeOffset="10"/></Representation></AdaptationSet>{TAIL}"#
    );
    let mpd = parse(&xml);
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let SegmentInfo::Template(t) = resolve_segment_info(period, set, rep).unwrap() else {
        panic!("expected a SegmentTemplate");
    };
    assert_eq!(t.multi.duration, Some(2));
    assert_eq!(t.multi.timescale, Timescale::try_new(90_000).unwrap());
    assert_eq!(t.multi.presentation_time_offset, 10);
    assert_eq!(t.media.as_deref(), Some("a$Number$.m4s"));
    assert_eq!(t.multi.start_number, 1);
}

#[test]
fn a_representation_segment_list_supplies_the_urls() {
    let xml = format!(
        r#"{HEAD}<SegmentList duration="2"/><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentList><SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/></SegmentList></Representation></AdaptationSet>{TAIL}"#
    );
    let mpd = parse(&xml);
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let SegmentInfo::List(list) = resolve_segment_info(period, set, rep).unwrap() else {
        panic!("expected a SegmentList");
    };
    let media: Vec<_> = list
        .urls
        .iter()
        .filter_map(|u| u.media.as_deref())
        .collect();
    assert_eq!(media, ["a.m4s", "b.m4s"]);
    assert_eq!(list.multi.duration, Some(2));
}

#[test]
fn a_period_segment_list_supplies_the_urls_when_no_lower_level_does() {
    let xml = format!(
        r#"{HEAD}<SegmentList duration="2"><SegmentURL media="p.m4s"/></SegmentList><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"/></AdaptationSet>{TAIL}"#
    );
    let mpd = parse(&xml);
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let SegmentInfo::List(list) = resolve_segment_info(period, set, rep).unwrap() else {
        panic!("expected a SegmentList");
    };
    let media: Vec<_> = list
        .urls
        .iter()
        .filter_map(|u| u.media.as_deref())
        .collect();
    assert_eq!(media, ["p.m4s"]);
}

#[test]
fn common_attributes_of_a_representation_fall_back_to_its_set() {
    let xml = format!(
        r#"{HEAD}<AdaptationSet mimeType="video/mp4" profiles="urn:p" width="640" height="480" sar="1:1"><Representation id="r" bandwidth="1" width="320"/></AdaptationSet>{TAIL}"#
    );
    let mpd = parse(&xml);
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let common = effective_common(set, rep);
    assert_eq!(common.width, Some(320));
    assert_eq!(common.height, Some(480));
    assert_eq!(common.profiles.as_deref(), Some("urn:p"));
    assert_eq!(common.sar.as_deref(), Some("1:1"));
    assert_eq!(common.mime_type.as_deref(), Some("video/mp4"));

    let xml = format!(
        r#"{HEAD}<AdaptationSet mimeType="video/mp4" height="480"><Representation id="r" bandwidth="1"/></AdaptationSet>{TAIL}"#
    );
    let mpd = parse(&xml);
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let common = effective_common(set, rep);
    assert_eq!(common.width, None);
    assert_eq!(common.height, Some(480));
    assert_eq!(common.profiles, None);
}
