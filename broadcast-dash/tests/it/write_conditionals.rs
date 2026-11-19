//! The writer's "emit this element/attribute only if the model has content"
//! decisions for `BaseURL`, `SegmentBase`, `SegmentList` and
//! `SegmentTemplate`: both branches, asserted on the XML text and on the
//! re-parsed model.

use broadcast_dash::Mpd;
use broadcast_dash::inherit::SegmentInfo;
use broadcast_dash::limits::Limits;

const HEAD: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" xmlns:dvb="urn:dvb:dash:dash-extensions:2014-1" xmlns:xlink="http://www.w3.org/1999/xlink" type="static" minBufferTime="PT1S" mediaPresentationDuration="PT1S"><Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">"#;
const TAIL: &str = "</Representation></AdaptationSet></Period></MPD>";

fn round_trip(segment: &str) -> (Mpd, String) {
    let xml = format!("{HEAD}{segment}{TAIL}");
    let first = Mpd::parse_with(xml.as_bytes(), &Limits::default())
        .unwrap_or_else(|e| panic!("{segment}: {e}"));
    let out = first.to_xml().unwrap();
    let second = Mpd::parse_with(out.as_bytes(), &Limits::default())
        .unwrap_or_else(|e| panic!("{segment} wrote {out}: {e}"));
    assert_eq!(second, first, "{segment} -> {out}");
    (first, out)
}

/// The written document with its indentation folded to single spaces.
fn flat(xml: &str) -> String {
    xml.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn segment_of(mpd: &Mpd) -> SegmentInfo {
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    broadcast_dash::inherit::resolve_segment_info(period, set, rep).unwrap()
}

fn assert_flat(xml: &str, want: &str) {
    let flat = flat(xml);
    assert!(flat.contains(want), "{want} missing from {flat}");
}

#[test]
fn a_base_url_without_text_but_with_attributes_keeps_its_start_tag() {
    let (_, out) = round_trip(r#"<BaseURL serviceLocation="loc"></BaseURL>"#);
    assert_flat(&out, r#"<BaseURL serviceLocation="loc"> </BaseURL>"#);
    assert!(!flat(&out).contains(r#"serviceLocation="loc"/>"#), "{out}");
}

#[test]
fn a_base_url_keeps_every_dvb_attribute() {
    let (_, out) = round_trip(
        r#"<BaseURL serviceLocation="loc" dvb:priority="1" dvb:weight="2">http://x/</BaseURL>"#,
    );
    for want in [
        r#"serviceLocation="loc""#,
        r#"dvb:priority="1""#,
        r#"dvb:weight="2""#,
        ">http://x/</BaseURL>",
    ] {
        assert!(flat(&out).contains(want), "{want} missing from {out}");
    }
    let (_, bare) = round_trip("<BaseURL>http://y/</BaseURL>");
    assert_flat(&bare, ">http://y/</BaseURL>");
    assert!(!flat(&bare).contains("serviceLocation="), "{bare}");
}

#[test]
fn a_segment_base_with_only_base_attributes_is_an_empty_element() {
    let (_, out) = round_trip(r#"<SegmentBase indexRange="0-9"/>"#);
    assert_flat(&out, r#"<SegmentBase indexRange="0-9"/>"#);
}

#[test]
fn a_segment_base_with_only_an_initialization_keeps_it() {
    let (_, out) = round_trip(
        r#"<SegmentBase><Initialization sourceURL="init.mp4" range="0-9"/></SegmentBase>"#,
    );
    assert_flat(
        &out,
        r#"<SegmentBase> <Initialization sourceURL="init.mp4" range="0-9"/> </SegmentBase>"#,
    );
}

#[test]
fn a_segment_base_with_only_a_representation_index_keeps_it() {
    let (_, out) =
        round_trip(r#"<SegmentBase><RepresentationIndex sourceURL="idx.mp4"/></SegmentBase>"#);
    assert_flat(
        &out,
        r#"<SegmentBase> <RepresentationIndex sourceURL="idx.mp4"/> </SegmentBase>"#,
    );
}

#[test]
fn a_segment_list_with_only_duration_is_an_empty_element() {
    let (mpd, out) = round_trip(r#"<SegmentList duration="2"/>"#);
    assert_flat(&out, r#"<SegmentList duration="2"/>"#);
    let SegmentInfo::List(list) = segment_of(&mpd) else {
        panic!("expected a SegmentList");
    };
    assert_eq!(list.multi.duration, Some(2));
    assert!(list.urls.is_empty());
}

#[test]
fn a_segment_list_with_only_a_timeline_keeps_it() {
    let (_, out) = round_trip(
        r#"<SegmentList><SegmentTimeline><S t="0" d="2" r="1"/></SegmentTimeline></SegmentList>"#,
    );
    assert_flat(
        &out,
        r#"<SegmentList> <SegmentTimeline> <S t="0" d="2" r="1"/> </SegmentTimeline> </SegmentList>"#,
    );
}

#[test]
fn a_segment_list_with_only_urls_keeps_them() {
    let (_, out) = round_trip(
        r#"<SegmentList><SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/></SegmentList>"#,
    );
    assert_flat(
        &out,
        r#"<SegmentList> <SegmentURL media="a.m4s"/> <SegmentURL media="b.m4s"/> </SegmentList>"#,
    );
}

#[test]
fn a_segment_list_with_only_a_bitstream_switching_child_keeps_it() {
    let (_, out) =
        round_trip(r#"<SegmentList><BitstreamSwitching sourceURL="bs.mp4"/></SegmentList>"#);
    assert_flat(
        &out,
        r#"<SegmentList> <BitstreamSwitching sourceURL="bs.mp4"/> </SegmentList>"#,
    );
}

#[test]
fn a_segment_list_without_children_is_an_empty_element() {
    let (_, out) = round_trip("<SegmentList></SegmentList>");
    assert_flat(&out, "<SegmentList/>");
}

#[test]
fn a_segment_list_keeps_non_default_xlink_actuate() {
    let (_, out) = round_trip(r#"<SegmentList xlink:href="http://x" xlink:actuate="onLoad"/>"#);
    assert_flat(&out, r#"xlink:actuate="onLoad""#);
    let (_, out) = round_trip(r#"<SegmentList xlink:href="http://x" xlink:actuate="onRequest"/>"#);
    assert_flat(&out, r#"xlink:href="http://x""#);
    assert!(!flat(&out).contains("xlink:actuate="), "{out}");
}

#[test]
fn a_segment_template_with_only_attributes_is_an_empty_element() {
    let (mpd, out) = round_trip(
        r#"<SegmentTemplate timescale="1000" duration="2" media="m$Number$.m4s" initialization="init.mp4"/>"#,
    );
    assert_flat(&out, r#"media="m$Number$.m4s""#);
    assert_flat(&out, r#"initialization="init.mp4""#);
    assert_flat(&out, r#"timescale="1000""#);
    assert_flat(&out, r#"media="m$Number$.m4s"/>"#);
    let SegmentInfo::Template(t) = segment_of(&mpd) else {
        panic!("expected a SegmentTemplate");
    };
    assert_eq!(t.multi.duration, Some(2));
    assert_eq!(t.media.as_deref(), Some("m$Number$.m4s"));
}

#[test]
fn a_segment_template_with_only_an_initialization_child_keeps_it() {
    let (_, out) =
        round_trip(r#"<SegmentTemplate><Initialization sourceURL="init.mp4"/></SegmentTemplate>"#);
    assert_flat(
        &out,
        r#"<SegmentTemplate> <Initialization sourceURL="init.mp4"/> </SegmentTemplate>"#,
    );
}

#[test]
fn a_segment_template_with_only_a_timeline_keeps_it() {
    let (_, out) = round_trip(
        r#"<SegmentTemplate media="m$Number$.m4s"><SegmentTimeline><S d="2"/></SegmentTimeline></SegmentTemplate>"#,
    );
    assert_flat(&out, r#"<SegmentTimeline> <S d="2"/> </SegmentTimeline>"#);
}
