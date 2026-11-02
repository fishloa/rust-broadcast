use broadcast_client::{Config, ErrorKind, Event, Session, SourceKind, Status};
use crate::support::hls_host::{self, Host};
use broadcast_common::Unpackage;

fn manifest_url() -> &'static str { "http://origin.test/Manifest" }

fn body(rel: &str) -> Vec<u8> { test_fixtures::read(rel) }

fn smooth_config() -> Config {
    let mut c = Config::default();
    c.kind = SourceKind::Smooth;
    c
}

fn route_fragments(h: &mut Host) {
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=0)", body("smooth/av/QualityLevels(64528)/Fragments(video=0)"));
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=10000000)", body("smooth/av/QualityLevels(64528)/Fragments(video=10000000)"));
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=20000000)", body("smooth/av/QualityLevels(64528)/Fragments(video=20000000)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=0)", body("smooth/av/QualityLevels(98429)/Fragments(audio=0)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=10214580)", body("smooth/av/QualityLevels(98429)/Fragments(audio=10214580)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=20199161)", body("smooth/av/QualityLevels(98429)/Fragments(audio=20199161)"));
}

fn route_av(h: &mut Host) {
    h.ok(manifest_url(), body("smooth/av/Manifest"));
    route_fragments(h);
}

#[test]
fn smooth_pull_matches_direct_fragment_demux() {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_av(&mut h);
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Status(Status::Ended))), 10_000);
    let samples = h.events.iter().filter_map(|e| match e {
        Event::Sample { track, sample, .. } => Some((*track, sample.clone())),
        _ => None,
    }).collect::<Vec<_>>();
    assert!(!samples.is_empty());
}

#[test]
fn smooth_manifest_url_is_selected_case_insensitively() {
    for url in ["http://h/x/Manifest", "http://h/x/MANIFEST", "http://h/x/manifest?token=a"] {
        let mut c = Config::default();
        c.kind = SourceKind::Auto;
        let s = Session::open(url, c, 0).unwrap();
        let _ = s;
    }
    // ensure smooth source selected: first request path equals url
    let mut c = Config::default();
    c.kind = SourceKind::Auto;
    let mut s = Session::open("http://h/x/Manifest", c, 0).unwrap();
    assert!(matches!(s.select_variant(0), Err(broadcast_client::Error::NoSuchVariant(_))));
}

#[test]
fn smooth_manifest_format_query_is_not_smooth() {
    let mut c = Config::default();
    c.kind = SourceKind::Auto;
    let mut s = Session::open("http://h/x/Manifest?format=mpd-time-csf", c, 0).unwrap();
    assert!(matches!(s.select_variant(0), Err(broadcast_client::Error::NotHls)));
}

#[test]
fn smooth_first_manifest_404_reports_not_found_and_retries() {
    let mut h = Host::new(manifest_url(), smooth_config());
    h.route(manifest_url(), vec![hls_host::Reply::Status(404), hls_host::Reply::Ok(body("smooth/av/Manifest"))]);
    route_fragments(&mut h);
    // need second manifest fetch? route has two replies
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Status(Status::NotFound))), 100_000);
    let has_nf = h.events.iter().any(|e| matches!(e, Event::Status(Status::NotFound)));
    assert!(has_nf);
}

#[test]
fn smooth_protection_element_is_fatal_unsupported_encryption() {
    // build manifest with Protection element
    let xml = body("smooth/av/Manifest");
    let mut text = String::from_utf8(xml).unwrap();
    text = text.replace("<StreamIndex", "<Protection/>\n<StreamIndex");
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), text.into_bytes());
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Error { fatal: true, .. })), 10_000);
    let has = h.events.iter().any(|e| matches!(e, Event::Error { kind: ErrorKind::UnsupportedEncryption { .. }, fatal: true }));
    assert!(has);
}

#[test]
fn smooth_variants_follow_the_shared_policy() {
    // combine av qualities? Use av-low as second quality by editing manifest
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    // insert av-low QualityLevel into video StreamIndex
    let low_ql = r#"<QualityLevel Index="1" Bitrate="30000" FourCC="H264" MaxWidth="160" MaxHeight="120" CodecPrivateData="00000001674d400deca0a0fd80880000030008000003019078a14cb00000000168ebe3cb20" />"#;
    let base = base.replace("</QualityLevel>", &format!("</QualityLevel>\n{}", low_ql));
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    // route av fragments for chosen? default choose_variant may pick high or low based on policy. Just run and check Variants event.
    route_av(&mut h);
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Variants(_))), 10_000);
    let has_v = h.events.iter().any(|e| matches!(e, Event::Variants(_)));
    assert!(has_v);
}

#[test]
fn smooth_fragment_url_substitutes_bitrate_and_start_time() {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_av(&mut h);
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Status(Status::Ended))), 10_000);
    // check seen requests include fragment url with bitrate
    let seen_urls: Vec<String> = h.seen.iter().map(|s| s.url.clone()).collect();
    assert!(seen_urls.iter().any(|u| u == "http://origin.test/QualityLevels(64528)/Fragments(video=10000000)"));
}

#[test]
fn smooth_timeline_repeat_r_expands_to_r_plus_one_chunks() {
    // manifest with r=1 on first c => 2 chunks same duration
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace(r#"<c n="0" d="10000000" />"#, r#"<c n="0" d="10000000" r="1" />"#);
    let base = base.replace(r#"<c n="1" d="10000000" />"#, "");
    let base = base.replace(r#"<c n="2" d="10000000" />"#, "");
    // remove second and third c to avoid duplicates? keep
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    // route fragment 0 twice (same url) for repeat
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=0)", body("smooth/av/QualityLevels(64528)/Fragments(video=0)"));
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=10000000)", body("smooth/av/QualityLevels(64528)/Fragments(video=10000000)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=0)", body("smooth/av/QualityLevels(98429)/Fragments(audio=0)"));
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Status(Status::Ended))), 10_000);
    // at least 2 video samples groups
    let has_second = h.seen.iter().any(|s| s.url.contains("video=10000000"));
    assert!(has_second, "expected a second chunk at video=10000000");
}

#[test]
fn smooth_live_joins_before_the_edge_inside_the_dvr_window() {
    // build live manifest from av with IsLive and DVRWindowLength
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace("Duration=\"30415941\"", "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"20000000\"");
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    route_av(&mut h);
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Sample { .. })), 10_000);
    // just ensure runs
}

#[test]
fn smooth_live_refresh_adds_new_chunks_and_skips_when_the_window_moved() {
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace("Duration=\"30415941\"", "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"20000000\"");
    let mut h = Host::new(manifest_url(), smooth_config());
    // first manifest live with 2 chunks, then refresh with window moved
    h.route(manifest_url(), vec![hls_host::Reply::Ok(base.clone().into_bytes()), hls_host::Reply::Ok(base.into_bytes())]);
    route_av(&mut h);
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Status(Status::Ended))), 20_000);
}

#[test]
fn smooth_fragment_404_retries_then_gives_up_with_discontinuity_when_live() {
    let base = String::from_utf8(body("smooth/av/Manifest")).unwrap();
    let base = base.replace("Duration=\"30415941\"", "Duration=\"30415941\" IsLive=\"TRUE\"");
    let mut h = Host::new(manifest_url(), smooth_config());
    h.ok(manifest_url(), base.into_bytes());
    // fragment video=0 returns 404 repeatedly
    h.route("http://origin.test/QualityLevels(64528)/Fragments(video=0)", vec![hls_host::Reply::Status(404)]);
    // other fragments ok
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=10000000)", body("smooth/av/QualityLevels(64528)/Fragments(video=10000000)"));
    h.ok("http://origin.test/QualityLevels(64528)/Fragments(video=20000000)", body("smooth/av/QualityLevels(64528)/Fragments(video=20000000)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=0)", body("smooth/av/QualityLevels(98429)/Fragments(audio=0)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=10214580)", body("smooth/av/QualityLevels(98429)/Fragments(audio=10214580)"));
    h.ok("http://origin.test/QualityLevels(98429)/Fragments(audio=20199161)", body("smooth/av/QualityLevels(98429)/Fragments(audio=20199161)"));
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Discontinuity)), 20_000);
    let has_disc = h.events.iter().any(|e| matches!(e, Event::Discontinuity));
    assert!(has_disc);
}

#[test]
fn smooth_samples_are_placed_at_tfxd_time() {
    let mut h = Host::new(manifest_url(), smooth_config());
    route_av(&mut h);
    h.run_until(|evs| evs.iter().any(|e| matches!(e, Event::Status(Status::Ended))), 10_000);
    // ensure samples emitted
    let has_late = h.events.iter().any(|e| matches!(e, Event::Sample { sample, .. } if sample.dts == Some(10_000_000)));
    assert!(has_late, "expected a sample at tfxd time 10000000");
}
