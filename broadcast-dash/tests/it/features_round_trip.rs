use broadcast_dash::Mpd;

const DROPPED_SERVICE_DESCRIPTION: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><Period><AdaptationSet><ServiceDescription id="0"/></AdaptationSet></Period></MPD>"#;

const RETAINED_SERVICE_DESCRIPTION: &str =
    r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><ServiceDescription id="0"/></MPD>"#;

const DROPPED_INBAND_EVENT_STREAM: &str =
    r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><InbandEventStream schemeIdUri="urn:x"/></MPD>"#;

const RETAINED_INBAND_EVENT_STREAM: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><InbandEventStream schemeIdUri="urn:x"/></Representation></AdaptationSet></Period></MPD>"#;

const DROPPED_XLINK_RESOLVE_TO_ZERO: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" xmlns:xlink="http://www.w3.org/1999/xlink"><Period xlink:href="urn:mpeg:dash:resolve-to-zero:2013"/></MPD>"#;

const RETAINED_XLINK_RESOLVE_TO_ZERO: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" xmlns:xlink="http://www.w3.org/1999/xlink"><Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentList xlink:href="urn:mpeg:dash:resolve-to-zero:2013"/></Representation></AdaptationSet></Period></MPD>"#;

fn round_trip(xml: &str) -> (Mpd, Mpd) {
    let mpd = Mpd::parse(xml.as_bytes()).expect("the MPD parses");
    let rendered = mpd.to_xml().expect("the model writes");
    let again = Mpd::parse(rendered.as_bytes()).expect("the writer output parses");
    (mpd, again)
}

#[test]
fn a_service_description_the_model_drops_raises_no_feature() {
    let (mpd, again) = round_trip(DROPPED_SERVICE_DESCRIPTION);
    assert!(mpd.service_descriptions.is_empty());
    assert!(mpd.periods[0].service_descriptions.is_empty());
    assert!(!mpd.features.service_description);
    assert!(!again.features.service_description);
    assert_eq!(again, mpd);
}

#[test]
fn a_service_description_the_model_keeps_still_raises_the_feature() {
    let (mpd, again) = round_trip(RETAINED_SERVICE_DESCRIPTION);
    assert_eq!(mpd.service_descriptions.len(), 1);
    assert!(mpd.features.service_description);
    assert!(again.features.service_description);
    assert_eq!(again, mpd);
}

#[test]
fn an_inband_event_stream_the_model_drops_raises_no_feature() {
    let (mpd, again) = round_trip(DROPPED_INBAND_EVENT_STREAM);
    assert!(mpd.periods.is_empty());
    assert!(!mpd.features.inband_event_stream);
    assert!(!again.features.inband_event_stream);
    assert_eq!(again, mpd);
}

#[test]
fn an_inband_event_stream_the_model_keeps_still_raises_the_feature() {
    let (mpd, again) = round_trip(RETAINED_INBAND_EVENT_STREAM);
    assert_eq!(
        mpd.periods[0].adaptation_sets[0].representations[0]
            .inband_event_streams
            .len(),
        1
    );
    assert!(mpd.features.inband_event_stream);
    assert!(again.features.inband_event_stream);
    assert_eq!(again, mpd);
}

#[test]
fn a_resolve_to_zero_xlink_the_model_drops_raises_no_feature() {
    let (mpd, again) = round_trip(DROPPED_XLINK_RESOLVE_TO_ZERO);
    assert!(mpd.periods.is_empty());
    assert!(!mpd.features.xlink_resolve_to_zero);
    assert!(!again.features.xlink_resolve_to_zero);
    assert_eq!(again, mpd);
}

#[test]
fn a_resolve_to_zero_xlink_the_model_keeps_still_raises_the_feature() {
    let (mpd, again) = round_trip(RETAINED_XLINK_RESOLVE_TO_ZERO);
    assert_eq!(
        mpd.periods[0].adaptation_sets[0].representations[0]
            .segment
            .list
            .as_ref()
            .expect("the SegmentList is kept")
            .xlink
            .as_ref()
            .expect("its xlink is kept")
            .href,
        "urn:mpeg:dash:resolve-to-zero:2013"
    );
    assert!(mpd.features.xlink_resolve_to_zero);
    assert!(again.features.xlink_resolve_to_zero);
    assert_eq!(again, mpd);
}

const RETAINED_LOW_LATENCY: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><BaseURL availabilityTimeOffset="1.0">http://cdn/</BaseURL><Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentTemplate media="s-$Number$.m4s" availabilityTimeComplete="false"/></Representation></AdaptationSet></Period></MPD>"#;

#[test]
fn a_low_latency_marker_the_model_keeps_still_raises_the_feature() {
    let (mpd, again) = round_trip(RETAINED_LOW_LATENCY);
    assert_eq!(mpd.base_urls[0].availability_time_offset, Some(1.0));
    assert!(mpd.features.low_latency);
    assert!(again.features.low_latency);
    assert_eq!(again, mpd);
}
