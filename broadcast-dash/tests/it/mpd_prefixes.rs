use std::time::Duration;

use broadcast_dash::Mpd;
use broadcast_dash::model::*;

type Place = fn(&mut Mpd, Descriptor);
type XlinkPlace = fn(&mut Mpd);
type BasePlace = fn(&mut Mpd, BaseUrl);

fn skeleton() -> Mpd {
    let mut rep = Representation::default();
    rep.id = "r".to_string();
    let mut aset = AdaptationSet::default();
    aset.representations = vec![rep];
    let mut event = StreamEvent::default();
    event.id = Some(1);
    let mut es = EventStream::default();
    es.scheme_id_uri = "urn:e".to_string();
    es.events = vec![event];
    let mut pre = Preselection::default();
    pre.preselection_components = vec!["1".to_string()];
    let mut period = Period::default();
    period.adaptation_sets = vec![aset];
    period.event_streams = vec![es];
    period.preselections = vec![pre];
    let mut mpd = Mpd::default();
    mpd.min_buffer_time = Some(Duration::from_secs(1));
    mpd.periods = vec![period];
    mpd
}

fn cenc_descriptor() -> Descriptor {
    let mut d = Descriptor::default();
    d.scheme_id_uri = "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed".to_string();
    d.default_kid = Some("10000000-1000-1000-1000-100000000000".to_string());
    d.pssh = Some("AAAA".to_string());
    d
}

fn query_descriptor() -> Descriptor {
    let mut q = UrlQueryInfo::default();
    q.use_mpd_url_query = true;
    let mut d = Descriptor::default();
    d.scheme_id_uri = "urn:mpeg:dash:urlparam:2014".to_string();
    d.url_query = Some(q);
    d
}

fn places() -> Vec<(&'static str, Place)> {
    vec![
        ("mpd essential", |m, d| m.essential_properties = vec![d]),
        ("mpd supplemental", |m, d| {
            m.supplemental_properties = vec![d]
        }),
        ("event stream essential", |m, d| {
            m.periods[0].event_streams[0].essential_properties = vec![d];
        }),
        ("event stream supplemental", |m, d| {
            m.periods[0].event_streams[0].supplemental_properties = vec![d];
        }),
        ("event essential", |m, d| {
            m.periods[0].event_streams[0].events[0].essential_properties = vec![d];
        }),
        ("event supplemental", |m, d| {
            m.periods[0].event_streams[0].events[0].supplemental_properties = vec![d];
        }),
        ("preselection frame packing", |m, d| {
            m.periods[0].preselections[0].frame_packings = vec![d];
        }),
        ("preselection audio channel", |m, d| {
            m.periods[0].preselections[0].audio_channel_configurations = vec![d];
        }),
        ("preselection content protection", |m, d| {
            m.periods[0].preselections[0].content_protections = vec![d];
        }),
        ("preselection output protection", |m, d| {
            m.periods[0].preselections[0].output_protection = Some(d);
        }),
        ("preselection essential", |m, d| {
            m.periods[0].preselections[0].essential_properties = vec![d];
        }),
        ("preselection supplemental", |m, d| {
            m.periods[0].preselections[0].supplemental_properties = vec![d];
        }),
        ("preselection inband", |m, d| {
            m.periods[0].preselections[0].inband_event_streams = vec![d];
        }),
        ("preselection accessibility", |m, d| {
            m.periods[0].preselections[0].accessibilities = vec![d];
        }),
        ("preselection role", |m, d| {
            m.periods[0].preselections[0].roles = vec![d];
        }),
        ("preselection rating", |m, d| {
            m.periods[0].preselections[0].ratings = vec![d];
        }),
        ("preselection viewpoint", |m, d| {
            m.periods[0].preselections[0].viewpoints = vec![d];
        }),
        ("set role", |m, d| {
            m.periods[0].adaptation_sets[0].roles = vec![d];
        }),
        ("set accessibility", |m, d| {
            m.periods[0].adaptation_sets[0].accessibilities = vec![d];
        }),
        ("set content protection", |m, d| {
            m.periods[0].adaptation_sets[0].content_protections = vec![d];
        }),
        ("set essential", |m, d| {
            m.periods[0].adaptation_sets[0].essential_properties = vec![d];
        }),
        ("set supplemental", |m, d| {
            m.periods[0].adaptation_sets[0].supplemental_properties = vec![d];
        }),
        ("set audio channel", |m, d| {
            m.periods[0].adaptation_sets[0].audio_channel_configurations = vec![d];
        }),
        ("set inband", |m, d| {
            m.periods[0].adaptation_sets[0].inband_event_streams = vec![d];
        }),
        ("representation content protection", |m, d| {
            m.periods[0].adaptation_sets[0].representations[0].content_protections = vec![d];
        }),
        ("representation essential", |m, d| {
            m.periods[0].adaptation_sets[0].representations[0].essential_properties = vec![d];
        }),
        ("representation supplemental", |m, d| {
            m.periods[0].adaptation_sets[0].representations[0].supplemental_properties = vec![d];
        }),
        ("representation audio channel", |m, d| {
            m.periods[0].adaptation_sets[0].representations[0].audio_channel_configurations =
                vec![d];
        }),
        ("representation inband", |m, d| {
            m.periods[0].adaptation_sets[0].representations[0].inband_event_streams = vec![d];
        }),
    ]
}

fn xlink() -> Xlink {
    let mut x = Xlink::default();
    x.href = "http://x/y.xml".to_string();
    x
}

fn xlink_places() -> Vec<(&'static str, XlinkPlace)> {
    vec![
        ("period", |m| m.periods[0].xlink = Some(xlink())),
        ("adaptation set", |m| {
            m.periods[0].adaptation_sets[0].xlink = Some(xlink());
        }),
        ("event stream", |m| {
            m.periods[0].event_streams[0].xlink = Some(xlink());
        }),
        ("request param", |m| {
            let mut r = RequestParam::default();
            r.xlink = Some(xlink());
            m.periods[0].event_streams[0].request_params = vec![r];
        }),
        ("segment list", |m| {
            let mut sl = SegmentList::default();
            sl.xlink = Some(xlink());
            m.periods[0].adaptation_sets[0].segment.list = Some(sl);
        }),
    ]
}

fn assert_declares_what_it_emits(name: &str, mpd: &Mpd) {
    let xml = mpd.to_xml().unwrap();
    let again = Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}\n{xml}"));
    let mut expected = mpd.clone();
    expected.features = again.features;
    assert_eq!(again, expected, "{name}\n{xml}");
}

#[test]
fn a_cenc_descriptor_anywhere_declares_its_prefix() {
    for (name, place) in places() {
        let mut mpd = skeleton();
        place(&mut mpd, cenc_descriptor());
        assert_declares_what_it_emits(name, &mpd);
    }
}

#[test]
fn a_url_query_descriptor_anywhere_declares_its_prefix() {
    for (name, place) in places() {
        let mut mpd = skeleton();
        place(&mut mpd, query_descriptor());
        assert_declares_what_it_emits(name, &mpd);
    }
}

#[test]
fn an_xlink_anywhere_declares_its_prefix() {
    for (name, place) in xlink_places() {
        let mut mpd = skeleton();
        place(&mut mpd);
        assert_declares_what_it_emits(name, &mpd);
    }
}

#[test]
fn a_dvb_base_url_anywhere_declares_its_prefix() {
    let mut b = BaseUrl::default();
    b.url = "http://b/".to_string();
    b.priority = Some(1);
    let places: Vec<(&str, BasePlace)> = vec![
        ("mpd", |m, b| m.base_urls = vec![b]),
        ("period", |m, b| m.periods[0].base_urls = vec![b]),
        ("event stream", |m, b| {
            m.periods[0].event_streams[0].base_urls = vec![b];
        }),
        ("set", |m, b| {
            m.periods[0].adaptation_sets[0].base_urls = vec![b]
        }),
        ("representation", |m, b| {
            m.periods[0].adaptation_sets[0].representations[0].base_urls = vec![b];
        }),
    ];
    for (name, place) in places {
        let mut mpd = skeleton();
        place(&mut mpd, b.clone());
        assert_declares_what_it_emits(name, &mpd);
    }
}

#[test]
fn a_document_using_every_prefix_validates_against_the_schema() {
    let Some(schema) = test_fixtures::process::xsd::dash_mpd_schema("mpd_prefixes") else {
        return;
    };
    let mut mpd = skeleton();
    for (name, place) in places() {
        if name.contains("content protection") {
            place(&mut mpd, cenc_descriptor());
        }
    }
    mpd.profiles = vec!["urn:mpeg:dash:profile:isoff-live:2011".to_string()];
    for (_, place) in xlink_places() {
        place(&mut mpd);
    }
    let mut b = BaseUrl::default();
    b.url = "http://b/".to_string();
    b.priority = Some(1);
    mpd.base_urls = vec![b];
    let xml = mpd.to_xml().unwrap();
    for prefix in ["xmlns:cenc=", "xmlns:xlink=", "xmlns:dvb="] {
        assert!(xml.contains(prefix), "{prefix} missing from {xml}");
    }
    Mpd::parse(xml.as_bytes()).unwrap();
    if let Err(e) = test_fixtures::process::xsd::validate(&schema, "every prefix", &xml) {
        panic!("{e}\n{xml}");
    }
}
