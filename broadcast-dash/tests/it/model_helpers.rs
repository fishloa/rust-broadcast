//! The feature-detection helpers consumers branch on, `ByteRange`'s display
//! and emptiness, and the label of every error enum variant.

use broadcast_dash::Mpd;
use broadcast_dash::error::{Limit, Unsupported};
use broadcast_dash::model::ByteRange;

const HEAD: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" minBufferTime="PT1S" mediaPresentationDuration="PT1S">"#;
const TAIL: &str = "</MPD>";

fn parse(body: &str) -> Mpd {
    let xml = format!("{HEAD}{body}{TAIL}");
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{xml}: {e}"))
}

#[test]
fn event_stream_helpers_report_both_carriages() {
    let plain = parse("<Period id=\"0\"/>");
    assert!(!plain.has_event_streams());

    let in_period =
        parse(r#"<Period id="0"><EventStream schemeIdUri="urn:e" value="v"/></Period>"#);
    assert!(in_period.has_event_streams());

    let inband = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><InbandEventStream schemeIdUri="urn:e"/></AdaptationSet></Period>"#,
    );
    assert!(inband.features.inband_event_stream);
    assert!(inband.has_event_streams());
}

#[test]
fn preselection_and_patch_location_helpers_are_not_constants() {
    let plain = parse("<Period id=\"0\"/>");
    assert!(!plain.has_preselections());
    assert!(!plain.has_patch_location());

    let preselected =
        parse(r#"<Period id="0"><Preselection id="p" preselectionComponents="1"/></Period>"#);
    assert!(preselected.has_preselections());

    let patched =
        Mpd::parse(format!("{HEAD}<PatchLocation>http://patch/</PatchLocation>{TAIL}").as_bytes())
            .unwrap();
    assert!(patched.has_patch_location());
}

#[test]
fn byte_range_display_and_emptiness() {
    let closed = ByteRange::new(5, 9);
    assert!(!closed.is_empty());
    assert_eq!(closed.to_string(), "5-9");

    let mut open = ByteRange::default();
    open.first = 5;
    assert!(open.is_empty());
    assert_eq!(open.to_string(), "5-");

    let inverted = ByteRange::new(9, 3);
    assert!(inverted.is_empty());
    assert_eq!(inverted.to_string(), "9-3");
}

#[test]
fn every_limit_labels_itself() {
    for (limit, label) in [
        (Limit::InputBytes, "input_bytes"),
        (Limit::Depth, "depth"),
        (Limit::Elements, "elements"),
        (Limit::AttributeBytes, "attribute_bytes"),
        (Limit::Attributes, "attributes"),
        (Limit::Periods, "periods"),
        (Limit::AdaptationSets, "adaptation_sets"),
        (Limit::Representations, "representations"),
        (Limit::TimelineEntries, "timeline_entries"),
        (Limit::BaseUrls, "base_urls"),
        (Limit::SegmentUrls, "segment_urls"),
    ] {
        assert_eq!(limit.name(), label);
        assert_eq!(limit.to_string(), label);
    }
}

#[test]
fn every_unsupported_construct_labels_itself() {
    for (unsupported, label) in [
        (Unsupported::PresentationTypeList, "presentation_type_list"),
        (Unsupported::SegmentSequence, "segment_sequence"),
        (Unsupported::DataUrl, "data_url"),
        (Unsupported::OpenEndedRange, "open_ended_range"),
        (Unsupported::HierarchicalSidx, "hierarchical_sidx"),
        (Unsupported::EventXmlPayload, "event_xml_payload"),
        (
            Unsupported::RepresentationBaseChild,
            "representation_base_child",
        ),
    ] {
        assert_eq!(unsupported.name(), label);
        assert_eq!(unsupported.to_string(), label);
    }
}
