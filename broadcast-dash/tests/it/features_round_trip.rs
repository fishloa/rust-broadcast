use broadcast_dash::Mpd;

const DROPPED_SERVICE_DESCRIPTION: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><Period><AdaptationSet><ServiceDescription id="0"/></AdaptationSet></Period></MPD>"#;

const RETAINED_SERVICE_DESCRIPTION: &str =
    r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><ServiceDescription id="0"/></MPD>"#;

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
