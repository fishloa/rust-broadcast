//! The configured hostile-input ceilings are enforced, at their exact
//! boundary, and the shared XML reader's failures map to their typed variants.

use broadcast_dash::Mpd;
use broadcast_dash::error::{Limit, MpdError};
use broadcast_dash::limits::Limits;

const OPEN: &str =
    r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" minBufferTime="PT1S">"#;

fn parse_with(xml: &str, limits: &Limits) -> Result<Mpd, MpdError> {
    Mpd::parse_with(xml.as_bytes(), limits)
}

fn limits(set: impl FnOnce(&mut Limits)) -> Limits {
    let mut limits = Limits::default();
    set(&mut limits);
    limits
}

fn assert_too_large(result: Result<Mpd, MpdError>, limit: Limit, max: usize) {
    match result {
        Ok(_) => panic!("expected TooLarge {{ {limit:?}, {max} }}"),
        Err(MpdError::TooLarge {
            limit: got,
            max: got_max,
        }) => {
            assert_eq!((got, got_max), (limit, max));
        }
        Err(e) => panic!("expected TooLarge {{ {limit:?}, {max} }}, got {e:?}"),
    }
}

#[test]
fn the_period_count_is_bounded() {
    let doc = |n: usize| {
        let periods: String = (0..n).map(|i| format!(r#"<Period id="{i}"/>"#)).collect();
        format!("{OPEN}{periods}</MPD>")
    };
    let at = limits(|l| l.max_periods = 2);
    assert!(parse_with(&doc(2), &at).is_ok());
    assert_too_large(parse_with(&doc(3), &at), Limit::Periods, 2);
}

#[test]
fn the_adaptation_set_count_is_bounded() {
    let sets = |n: usize| {
        let sets: String = (0..n)
            .map(|_| r#"<AdaptationSet mimeType="video/mp4"/>"#)
            .collect();
        format!(r#"{OPEN}<Period id="0">{sets}</Period></MPD>"#)
    };
    let at = limits(|l| l.max_adaptation_sets = 2);
    assert!(parse_with(&sets(2), &at).is_ok());
    assert_too_large(parse_with(&sets(3), &at), Limit::AdaptationSets, 2);
}

#[test]
fn the_representation_count_is_bounded() {
    let reps = |n: usize| {
        let reps: String = (0..n)
            .map(|i| format!(r#"<Representation id="r{i}" bandwidth="1"/>"#))
            .collect();
        format!(
            r#"{OPEN}<Period id="0"><AdaptationSet mimeType="video/mp4">{reps}</AdaptationSet></Period></MPD>"#
        )
    };
    let at = limits(|l| l.max_representations = 2);
    assert!(parse_with(&reps(2), &at).is_ok());
    assert_too_large(parse_with(&reps(3), &at), Limit::Representations, 2);
}

#[test]
fn the_timeline_entry_count_is_bounded() {
    let entries = |n: usize| {
        let entries: String = (0..n).map(|_| r#"<S d="2"/>"#).collect();
        format!(
            r#"{OPEN}<Period id="0"><AdaptationSet mimeType="video/mp4"><SegmentTemplate media="m$Number$.m4s"><SegmentTimeline>{entries}</SegmentTimeline></SegmentTemplate></AdaptationSet></Period></MPD>"#
        )
    };
    let at = limits(|l| l.max_timeline_entries = 2);
    assert!(parse_with(&entries(2), &at).is_ok());
    assert_too_large(parse_with(&entries(3), &at), Limit::TimelineEntries, 2);
}

#[test]
fn the_base_url_count_is_bounded() {
    let urls = |n: usize| {
        let urls: String = (0..n)
            .map(|i| format!("<BaseURL>http://{i}/</BaseURL>"))
            .collect();
        format!(r#"{OPEN}<Period id="0">{urls}</Period></MPD>"#)
    };
    let at = limits(|l| l.max_base_urls = 2);
    assert!(parse_with(&urls(2), &at).is_ok());
    assert_too_large(parse_with(&urls(3), &at), Limit::BaseUrls, 2);
}

#[test]
fn the_segment_url_count_is_bounded() {
    let urls = |n: usize| {
        let urls: String = (0..n)
            .map(|i| format!(r#"<SegmentURL media="{i}.m4s"/>"#))
            .collect();
        format!(
            r#"{OPEN}<Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentList>{urls}</SegmentList></Representation></AdaptationSet></Period></MPD>"#
        )
    };
    let at = limits(|l| l.max_segment_urls = 2);
    assert!(parse_with(&urls(2), &at).is_ok());
    assert_too_large(parse_with(&urls(3), &at), Limit::SegmentUrls, 2);
}

#[test]
fn the_xml_limits_forward_every_field() {
    let doc = format!(r#"{OPEN}<Period id="0" duration="PT1S"/></MPD>"#);
    assert!(doc.len() > 100);

    let mut l = Limits::default();
    let over_len = doc.len() - 1;
    l.max_bytes = over_len;
    assert_too_large(parse_with(&doc, &l), Limit::InputBytes, over_len);

    let mut l = Limits::default();
    l.max_depth = 1;
    assert_too_large(parse_with(&doc, &l), Limit::Depth, 1);

    let mut l = Limits::default();
    l.max_elements = 1;
    assert_too_large(parse_with(&doc, &l), Limit::Elements, 1);

    let long = format!(r#"{OPEN}<Period id="0123456789"/></MPD>"#);
    let mut l = Limits::default();
    l.max_attribute_bytes = 8;
    assert_too_large(parse_with(&long, &l), Limit::AttributeBytes, 8);

    let mut l = Limits::default();
    l.max_attributes = 1;
    assert_too_large(parse_with(&doc, &l), Limit::Attributes, 1);
}

#[test]
fn low_latency_is_detected_from_either_base_url_attribute() {
    let mpd = parse_with(
        &format!(r#"{OPEN}<BaseURL availabilityTimeComplete="false">http://x/</BaseURL></MPD>"#),
        &Limits::default(),
    )
    .unwrap();
    assert!(mpd.features.low_latency);

    let mpd = parse_with(
        &format!(r#"{OPEN}<BaseURL availabilityTimeOffset="0">http://x/</BaseURL></MPD>"#),
        &Limits::default(),
    )
    .unwrap();
    assert!(mpd.features.low_latency);

    let mpd = parse_with(
        &format!(r#"{OPEN}<BaseURL availabilityTimeComplete="true">http://x/</BaseURL></MPD>"#),
        &Limits::default(),
    )
    .unwrap();
    assert!(!mpd.features.low_latency);
}

#[test]
fn a_segment_sequence_is_flagged_only_above_k_one() {
    let timeline = |k: &str| {
        format!(
            r#"{OPEN}<Period id="0"><AdaptationSet mimeType="video/mp4"><SegmentTemplate media="m$Number$.m4s"><SegmentTimeline><S d="2" k="{k}"/></SegmentTimeline></SegmentTemplate></AdaptationSet></Period></MPD>"#
        )
    };
    let mpd = parse_with(&timeline("1"), &Limits::default()).unwrap();
    let tl = mpd.periods[0].adaptation_sets[0]
        .segment
        .template
        .as_ref()
        .unwrap()
        .multi
        .timeline
        .as_ref()
        .unwrap();
    assert_eq!(tl.unsupported, None);

    let mpd = parse_with(&timeline("2"), &Limits::default()).unwrap();
    let tl = mpd.periods[0].adaptation_sets[0]
        .segment
        .template
        .as_ref()
        .unwrap()
        .multi
        .timeline
        .as_ref()
        .unwrap();
    assert_eq!(
        tl.unsupported,
        Some(broadcast_dash::error::Unsupported::SegmentSequence)
    );
}

#[test]
fn shared_xml_failures_map_to_their_typed_variants() {
    let utf8 = Mpd::parse(b"\xFF<MPD/>").unwrap_err();
    assert_eq!(utf8, MpdError::InvalidUtf8 { valid_up_to: 0 });

    let malformed = Mpd::parse(b"<MPD").unwrap_err();
    assert!(
        matches!(&malformed, MpdError::Xml { message, .. } if !message.is_empty()),
        "{malformed:?}"
    );

    let wrong_root = Mpd::parse(b"<NotMpd/>").unwrap_err();
    assert_eq!(
        wrong_root,
        MpdError::NotMpd {
            found: "NotMpd".to_string()
        }
    );

    let mut l = Limits::default();
    l.max_depth = 1;
    let deep = format!("{OPEN}<Period/></MPD>");
    let limited = Mpd::parse_with(deep.as_bytes(), &l).unwrap_err();
    assert_eq!(
        limited,
        MpdError::TooLarge {
            limit: Limit::Depth,
            max: 1
        }
    );
}
