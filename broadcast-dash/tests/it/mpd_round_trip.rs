use std::time::Duration;

use broadcast_dash::Mpd;
use broadcast_dash::error::{MpdError, Unsupported};
use broadcast_dash::limits::Limits;
use broadcast_dash::model::*;
use broadcast_dash::utc_timing::UtcScheme;

use super::corpus;

fn refused_event_payload(name: &str, mpd: &Mpd) -> Option<String> {
    match mpd.to_xml() {
        Ok(xml) => Some(xml),
        Err(MpdError::Unsupported(Unsupported::EventXmlPayload)) => None,
        Err(e) => panic!("{name}: {e}"),
    }
}

#[test]
fn every_parsed_fixture_round_trips_through_the_writer() {
    let mut count = 0usize;
    let mut refused = 0usize;
    for (name, path) in corpus::corpus() {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let Ok(mpd) = Mpd::parse_with(&bytes, &Limits::default()) else {
            continue;
        };
        let Some(xml) = refused_event_payload(&name, &mpd) else {
            refused += 1;
            continue;
        };
        let again = Mpd::parse_with(xml.as_bytes(), &Limits::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(again, mpd, "{name}");
        count += 1;
    }
    assert!(count >= 20, "round-tripped only {count} fixtures");
    assert!(refused > 0, "no fixture carries an XML Event payload");
}

#[test]
fn a_model_with_every_field_set_round_trips() {
    let mut urlq = UrlQueryInfo::default();
    urlq.use_mpd_url_query = true;
    urlq.query_string = Some("q".to_string());
    urlq.query_template = Some("t".to_string());

    let mut desc = Descriptor::default();
    desc.scheme_id_uri = "urn:x".to_string();
    desc.value = Some("v".to_string());
    desc.id = Some("d".to_string());
    desc.default_kid = Some("kid".to_string());
    desc.pssh = Some("AAAA".to_string());
    desc.url_query = Some(urlq);

    let mut bu = BaseUrl::default();
    bu.url = "http://example.com/".to_string();
    bu.service_location = Some("loc".to_string());
    bu.priority = Some(1);
    bu.weight = Some(2);
    bu.availability_time_offset = Some(1.5);
    bu.availability_time_complete = Some(true);
    bu.time_shift_buffer_depth = Some(Duration::from_secs(1));

    let mut ut = UtcTiming::default();
    ut.scheme = UtcScheme::Direct;
    ut.scheme_id_uri = "urn:mpeg:dash:utc:direct:2014".to_string();
    ut.value = "2020-01-01T00:00:00Z".to_string();
    ut.id = Some("u".to_string());

    let mut mpd = Mpd::default();
    mpd.id = Some("mpd".to_string());
    mpd.profiles = vec!["p1".to_string(), "p2".to_string()];
    mpd.kind = PresentationType::Dynamic;
    mpd.availability_start_time_us = Some(1_000_000);
    mpd.availability_end_time_us = Some(2_000_000);
    mpd.publish_time_us = Some(3_000_000);
    mpd.media_presentation_duration = Some(Duration::from_secs(10));
    mpd.minimum_update_period = Some(Duration::from_secs(1));
    mpd.min_buffer_time = Some(Duration::from_secs(2));
    mpd.time_shift_buffer_depth = Some(Duration::from_secs(3));
    mpd.suggested_presentation_delay = Some(Duration::from_secs(4));
    mpd.max_segment_duration = Some(Duration::from_secs(5));
    mpd.max_subsegment_duration = Some(Duration::from_secs(6));
    mpd.base_urls = vec![bu.clone()];
    mpd.locations = vec!["l1".to_string(), "l2".to_string()];
    let mut lat = Latency::default();
    lat.target = Some(1000);
    lat.min = Some(500);
    lat.max = Some(2000);
    lat.reference_id = Some(1);
    let mut pr = PlaybackRate::default();
    pr.min = Some("0.96".to_string());
    pr.max = Some("1.04".to_string());
    let mut sd = ServiceDescription::default();
    sd.id = Some(0);
    sd.latency = Some(lat);
    sd.playback_rate = Some(pr);
    mpd.service_descriptions = vec![sd];
    mpd.utc_timings = vec![ut];
    mpd.essential_properties = vec![desc.clone()];
    mpd.supplemental_properties = vec![desc.clone()];

    let mut period = Period::default();
    period.id = Some("period".to_string());
    period.start = Some(Duration::from_secs(0));
    period.duration = Some(Duration::from_secs(10));
    period.bitstream_switching = true;
    let mut xlink = Xlink::default();
    xlink.href = "http://x".to_string();
    xlink.actuate = Actuate::OnLoad;
    period.xlink = Some(xlink.clone());
    let mut pbu = BaseUrl::default();
    pbu.url = "http://p/".to_string();
    period.base_urls = vec![pbu];
    let mut base_info = BaseInfo::default();
    base_info.timescale = Some(1000);
    base_info.presentation_time_offset = Some(10);
    base_info.ept_delta = Some(1);
    base_info.pd_delta = Some(2);
    base_info.presentation_duration = Some(100);
    base_info.time_shift_buffer_depth = Some(Duration::from_secs(1));
    base_info.index_range = Some(ByteRange::new(0, 9));
    base_info.index_range_exact = Some(true);
    base_info.availability_time_offset = Some(1.5);
    base_info.availability_time_complete = Some(false);
    let mut u1 = UrlType::default();
    u1.source_url = Some("init.mp4".to_string());
    u1.range = Some(ByteRange::new(0, 9));
    base_info.initialization = Some(u1);
    let mut u2 = UrlType::default();
    u2.source_url = Some("index.mp4".to_string());
    base_info.representation_index = Some(u2);
    let mut sb = SegmentBase::default();
    sb.info = base_info.clone();
    let mut seg_layer_base = SegmentLayer::default();
    seg_layer_base.base = Some(sb);

    let mut multi = MultiInfo::default();
    multi.base = BaseInfo::default();
    multi.duration = Some(2000);
    multi.start_number = Some(1);
    multi.end_number = Some(2);
    let mut tl = SegmentTimeline::default();
    let mut te = TimelineEntry::new(1000);
    te.t = Some(0);
    te.n = Some(1);
    te.r = -1;
    te.k = Some(0);
    tl.entries = vec![te];
    multi.timeline = Some(tl);
    let mut u3 = UrlType::default();
    u3.source_url = Some("bs.mp4".to_string());
    multi.bitstream_switching = Some(u3);

    let mut seg_list = SegmentList::default();
    seg_list.multi = multi.clone();
    let mut xl2 = Xlink::default();
    xl2.href = "http://xl".to_string();
    xl2.actuate = Actuate::OnLoad;
    seg_list.xlink = Some(xl2);
    let mut su = SegmentUrl::default();
    su.media = Some("a.m4s".to_string());
    su.media_range = Some(ByteRange::new(0, 9));
    su.index = Some("i".to_string());
    let mut br = ByteRange::default();
    br.first = 0;
    br.last = None;
    su.index_range = Some(br);
    let mut su2 = SegmentUrl::default();
    su2.media = Some("b.m4s".to_string());
    seg_list.urls = vec![su, su2];

    let mut st = SegmentTemplate::default();
    let mut smi = MultiInfo::default();
    smi.base = BaseInfo::default();
    smi.duration = Some(2000);
    smi.start_number = Some(1);
    smi.end_number = Some(2);
    st.multi = smi;
    st.media = Some("m$Number$.m4s".to_string());
    st.index = Some("i$Number$".to_string());
    st.initialization = Some("init$Bandwidth$.mp4".to_string());
    st.bitstream_switching = Some("bs$Number$.mp4".to_string());

    let mut seg_layer_list = SegmentLayer::default();
    seg_layer_list.list = Some(seg_list.clone());

    let mut seg_layer_template = SegmentLayer::default();
    seg_layer_template.template = Some(st.clone());

    let mut aset = AdaptationSet::default();
    aset.id = Some(1);
    aset.group = Some(2);
    aset.lang = Some("en".to_string());
    aset.content_type = Some("video".to_string());
    aset.par = Some("16:9".to_string());
    aset.segment_alignment = true;
    aset.start_with_sap = Some(1);
    aset.bitstream_switching = Some(true);
    let mut cac = CommonAttributes::default();
    cac.profiles = Some("prof".to_string());
    cac.width = Some(1920);
    cac.height = Some(1080);
    cac.sar = Some("1:1".to_string());
    cac.frame_rate = Some("30".to_string());
    cac.audio_sampling_rate = Some("48000".to_string());
    cac.mime_type = Some("video/mp4".to_string());
    cac.codecs = Some("avc1".to_string());
    cac.max_playout_rate = Some(1.5);
    cac.coding_dependency = Some(false);
    aset.common = cac;
    let mut axl = Xlink::default();
    axl.href = "http://ax".to_string();
    axl.actuate = Actuate::OnLoad;
    aset.xlink = Some(axl);
    aset.roles = vec![desc.clone()];
    aset.accessibilities = vec![desc.clone()];
    aset.labels = vec!["lab".to_string(), "lab2".to_string()];
    let mut abu = BaseUrl::default();
    abu.url = "http://ab/".to_string();
    aset.base_urls = vec![abu];
    aset.content_protections = vec![desc.clone()];
    aset.essential_properties = vec![desc.clone()];
    aset.supplemental_properties = vec![desc.clone()];
    aset.audio_channel_configurations = vec![desc.clone()];
    let mut rep = Representation::default();
    rep.id = "r1".to_string();
    rep.bandwidth = 1000;
    rep.quality_ranking = Some(1);
    rep.dependency_ids = vec!["a".to_string(), "b".to_string()];
    let mut rcc = CommonAttributes::default();
    rcc.mime_type = Some("video/mp4".to_string());
    rcc.codecs = Some("avc1".to_string());
    rcc.coding_dependency = Some(true);
    rep.common = rcc;
    let mut rbu = BaseUrl::default();
    rbu.url = "http://rb/".to_string();
    rep.base_urls = vec![rbu];
    rep.content_protections = vec![desc.clone()];
    rep.essential_properties = vec![desc.clone()];
    rep.supplemental_properties = vec![desc.clone()];
    rep.audio_channel_configurations = vec![desc.clone()];
    rep.inband_event_streams = vec![desc.clone()];
    aset.representations = vec![rep];

    period.segment = seg_layer_base;
    period.asset_identifier = Some(desc.clone());
    let mut imp = ImportedMpd::default();
    imp.url = "http://imp.mpd".to_string();
    imp.earliest_resolution_time_offset = Some(60.0);
    period.imported_mpd = Some(imp);
    period.essential_properties = vec![desc.clone()];
    period.supplemental_properties = vec![desc.clone()];
    period.adaptation_sets = vec![aset];

    mpd.periods = vec![period];
    let mut steering = ContentSteering::default();
    steering.url = "https://steer.example/".to_string();
    steering.default_service_location = Some("cdn-a".to_string());
    steering.query_before_start = true;
    steering.client_requirement = Some(false);
    mpd.content_steering = Some(steering);
    let mut patch = PatchLocation::default();
    patch.url = "http://patch.example/p.mpp".to_string();
    patch.service_location = Some("cdn-a".to_string());
    patch.ttl = Some(30.5);
    mpd.patch_locations = vec![patch, PatchLocation::default()];
    mpd.features.inband_event_stream = true;
    mpd.features.service_description = true;
    mpd.features.low_latency = true;
    mpd.features.xlink_resolve_to_zero = false;

    let xml = mpd.to_xml().unwrap();
    let again = Mpd::parse(xml.as_bytes()).unwrap();
    assert_eq!(again, mpd);
}

fn every_field_event_stream(desc: &Descriptor) -> EventStream {
    let mut sd = ServiceDescription::default();
    sd.id = Some(3);
    let mut event = StreamEvent::default();
    event.presentation_time = 90_000;
    event.duration = Some(45_000);
    event.id = Some(7);
    event.status = Some(EventStatus::Update);
    event.message_data = Some("AQID".to_string());
    event.text = "a < b & c".to_string();
    event.supplemental_properties = vec![desc.clone()];
    event.essential_properties = vec![desc.clone()];
    event.service_descriptions = vec![sd];
    let mut bare = StreamEvent::default();
    bare.status = Some(EventStatus::None);
    let mut base_url = BaseUrl::default();
    base_url.url = "http://events/".to_string();
    let mut xlink = Xlink::default();
    xlink.href = "http://events/es.xml".to_string();
    xlink.actuate = Actuate::OnLoad;
    let mut es = EventStream::default();
    es.xlink = Some(xlink);
    es.scheme_id_uri = "urn:scte:scte35:2013:xml".to_string();
    es.value = Some("v".to_string());
    es.timescale = Some(90_000);
    es.presentation_time_offset = Some(5);
    es.events = vec![event, bare];
    es.base_urls = vec![base_url];
    es.essential_properties = vec![desc.clone()];
    es.supplemental_properties = vec![desc.clone()];
    es
}

fn every_field_producer_reference_time(
    id: u32,
    kind: ProducerReferenceKind,
) -> ProducerReferenceTime {
    let mut utc = UtcTiming::default();
    utc.scheme = UtcScheme::Direct;
    utc.scheme_id_uri = "urn:mpeg:dash:utc:direct:2014".to_string();
    utc.value = "2020-01-01T00:00:00Z".to_string();
    utc.id = Some("u".to_string());
    let mut prt = ProducerReferenceTime::default();
    prt.id = id;
    prt.inband = true;
    prt.kind = kind;
    prt.application_scheme = Some("urn:app".to_string());
    prt.wall_clock_time = "1970-01-01T00:00:01.5Z".to_string();
    prt.presentation_time = 1234;
    prt.utc_timing = Some(utc);
    prt
}

fn every_field_resync() -> Resync {
    let mut r = Resync::default();
    r.kind = 2;
    r.dt = Some(9);
    r.d_imax = Some(2.5);
    r.d_imin = Some(0.5);
    r.marker = true;
    r
}

fn every_field_preselection(desc: &Descriptor) -> Preselection {
    let mut common = CommonAttributes::default();
    common.profiles = Some("prof".to_string());
    common.width = Some(1280);
    common.height = Some(720);
    common.sar = Some("1:1".to_string());
    common.frame_rate = Some("25".to_string());
    common.audio_sampling_rate = Some("48000".to_string());
    common.mime_type = Some("audio/mp4".to_string());
    common.codecs = Some("ec-3".to_string());
    common.max_playout_rate = Some(2.0);
    common.coding_dependency = Some(true);
    let mut p = Preselection::default();
    p.id = Some("pre1".to_string());
    p.preselection_components = vec!["1".to_string(), "2".to_string()];
    p.lang = Some("de".to_string());
    p.order = Some(PreselectionOrder::FullyOrdered);
    p.common = common;
    p.audio_channel_configurations = vec![desc.clone()];
    p.essential_properties = vec![desc.clone()];
    p.supplemental_properties = vec![desc.clone()];
    p.accessibilities = vec![desc.clone()];
    p.roles = vec![desc.clone()];
    p.ratings = vec![desc.clone()];
    p.viewpoints = vec![desc.clone()];
    p
}

#[test]
fn event_streams_preselections_producer_times_and_resyncs_round_trip() {
    let mut desc = Descriptor::default();
    desc.scheme_id_uri = "urn:x".to_string();
    desc.value = Some("v".to_string());
    desc.default_kid = Some("kid".to_string());
    let mut rep = Representation::default();
    rep.id = "r".to_string();
    rep.producer_reference_times = vec![
        every_field_producer_reference_time(1, ProducerReferenceKind::Captured),
        every_field_producer_reference_time(2, ProducerReferenceKind::Application),
    ];
    rep.resyncs = vec![every_field_resync(), Resync::default()];
    let mut aset = AdaptationSet::default();
    aset.producer_reference_times = vec![ProducerReferenceTime::default()];
    aset.resyncs = vec![every_field_resync()];
    aset.representations = vec![rep];
    let mut period = Period::default();
    period.event_streams = vec![every_field_event_stream(&desc), EventStream::default()];
    period.adaptation_sets = vec![aset];
    period.preselections = vec![every_field_preselection(&desc), Preselection::default()];
    let mut mpd = Mpd::default();
    mpd.min_buffer_time = Some(Duration::from_secs(1));
    mpd.periods = vec![period];
    mpd.features.service_description = true;
    let xml = mpd.to_xml().unwrap();
    let again = Mpd::parse(xml.as_bytes()).unwrap();
    assert_eq!(again, mpd);
    assert!(mpd.has_event_streams() && mpd.has_preselections());
}

#[test]
fn an_event_with_an_xml_payload_is_flagged_and_refused_on_write() {
    let xml = br#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" minBufferTime="PT1S"><Period id="0"><EventStream schemeIdUri="urn:e"><Event presentationTime="1"><ReplacePresentation uri="http://x"/></Event></EventStream></Period></MPD>"#;
    let mpd = Mpd::parse(xml).unwrap();
    let event = &mpd.periods[0].event_streams[0].events[0];
    assert_eq!(event.unsupported, Some(Unsupported::EventXmlPayload));
    assert_eq!(
        mpd.to_xml(),
        Err(MpdError::Unsupported(Unsupported::EventXmlPayload))
    );
}

#[test]
fn text_events_parse_with_their_body() {
    let xml = br#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" minBufferTime="PT1S"><Period id="0"><EventStream schemeIdUri="urn:e" timescale="1000" presentationTimeOffset="4"><Event presentationTime="5" duration="2" id="9">  hello  </Event></EventStream></Period></MPD>"#;
    let mpd = Mpd::parse(xml).unwrap();
    let es = &mpd.periods[0].event_streams[0];
    assert_eq!(
        (es.timescale, es.presentation_time_offset),
        (Some(1000), Some(4))
    );
    let ev = &es.events[0];
    assert_eq!(
        (
            ev.presentation_time,
            ev.duration,
            ev.id,
            ev.text.as_str(),
            ev.unsupported
        ),
        (5, Some(2), Some(9), "hello", None)
    );
}

#[test]
fn to_xml_carries_declaration_and_namespace() {
    let mpd = Mpd::default();
    let xml = mpd.to_xml().unwrap();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
    assert!(xml.contains("xmlns=\"urn:mpeg:dash:schema:mpd:2011\""));
    let mut with_xlink = Mpd::default();
    let mut p = Period::default();
    let mut x = Xlink::default();
    x.href = "http://x".to_string();
    x.actuate = Actuate::OnLoad;
    p.xlink = Some(x);
    with_xlink.periods.push(p);
    let xml2 = with_xlink.to_xml().unwrap();
    assert!(xml2.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""));
}

#[test]
fn a_model_without_min_buffer_time_round_trips_without_one() {
    let xml = br#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT1S"><Period id="0"/></MPD>"#;
    let mpd = Mpd::parse_with(xml, &Limits::default()).unwrap();
    assert_eq!(mpd.min_buffer_time, None);
    let again = Mpd::parse_with(mpd.to_xml().unwrap().as_bytes(), &Limits::default()).unwrap();
    assert_eq!(again, mpd);
}

#[test]
fn infinite_availability_time_offset_round_trips_on_every_segment_element() {
    let mut mpd = Mpd::default();
    let mut period = Period::default();
    let mut template = SegmentTemplate::default();
    template.multi.base.availability_time_offset = Some(f64::INFINITY);
    let mut list = SegmentList::default();
    list.multi.base.availability_time_offset = Some(f64::NEG_INFINITY);
    let mut base = SegmentBase::default();
    base.info.availability_time_offset = Some(1.0e19);
    let mut aset = AdaptationSet::default();
    let mut rep = Representation::default();
    period.segment.template = Some(template);
    aset.segment.list = Some(list);
    rep.segment.base = Some(base);
    aset.representations = vec![rep];
    period.adaptation_sets = vec![aset];
    mpd.periods = vec![period];
    mpd.features.low_latency = true;
    let xml = mpd.to_xml().unwrap();
    assert!(xml.contains("availabilityTimeOffset=\"INF\""), "{xml}");
    assert!(xml.contains("availabilityTimeOffset=\"-INF\""), "{xml}");
    assert!(
        xml.contains("availabilityTimeOffset=\"10000000000000000000\""),
        "{xml}"
    );
    assert_eq!(Mpd::parse(xml.as_bytes()).unwrap(), mpd);
}
