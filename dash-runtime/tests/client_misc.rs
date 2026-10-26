//! Selection drops, URL policy (origin pinning, BaseURL alternates), the
//! manifest model, skipping, representation switching and protocol misuse of
//! `DashClient`. MPDs edited from the real ffmpeg file keep the real media.

mod common;

use common::loopback::{
    Drive, Fault, FaultKind, Origin, Sig, assert_matches_oracle, drive, mpd_text, oracle,
};
use dash_runtime::client::{
    Action, DashClient, DashConfig, DropCause, Error, FatalKind, Notice, Output, ResourceId,
};
use dash_runtime::selection::DropReason;

const DIR: &str = "timeline-ffmpeg-quirk";
const URL: &str = "http://dash.test/timeline-ffmpeg-quirk/manifest.mpd";

fn base_text() -> String {
    mpd_text("timeline-ffmpeg-quirk/manifest.mpd")
}

fn origin_with(text: String) -> Origin {
    Origin::new().doc(URL, text.into_bytes())
}

fn files(init: &str, stem: &str, n: usize) -> Vec<String> {
    let mut v = vec![format!("{DIR}/{init}")];
    for i in 1..=n {
        v.push(format!("{DIR}/{stem}{i:05}.m4s"));
    }
    v
}

fn oracle_of(files: &[String]) -> Vec<Sig> {
    oracle(&files.iter().map(String::as_str).collect::<Vec<_>>()).remove(0)
}

fn video_oracle() -> Vec<Sig> {
    oracle_of(&files("init-stream0.m4s", "chunk-stream0-", 3))
}

fn audio_oracle() -> Vec<Sig> {
    oracle_of(&files("init-stream1.m4s", "chunk-stream1-", 4))
}

fn dropped(run: &common::loopback::Run) -> Vec<(usize, Option<usize>, DropCause)> {
    run.outputs
        .iter()
        .filter_map(|o| match o {
            Output::TrackDropped {
                set, rep, cause, ..
            } => Some((*set, *rep, *cause)),
            _ => None,
        })
        .collect()
}

fn play_text(text: String, cfg: DashConfig) -> common::loopback::Run {
    drive(URL, cfg, &mut origin_with(text), &mut Drive::default())
}

const PROTECTION: &str =
    "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\" value=\"cenc\"/>";

#[test]
fn a_protected_video_set_is_dropped_with_a_notice_and_the_audio_plays_alone_as_track_one() {
    let text = base_text().replace(
        "<Representation id=\"0\"",
        &format!("{PROTECTION}<Representation id=\"0\""),
    );
    let run = play_text(text, DashConfig::default());
    assert!(run.ended, "{:?}", run.fatal);
    assert_matches_oracle(&run, &[audio_oracle()]);
    assert_eq!(
        dropped(&run),
        vec![(
            0,
            None,
            DropCause::Selection(DropReason::UnsupportedEncryption)
        )]
    );
    assert_eq!(
        run.count(|o| matches!(o, Output::Notice(Notice::UnsupportedEncryption))),
        1
    );
}

#[test]
fn protection_on_every_representation_drops_the_set() {
    let text = base_text().replacen("sar=\"1:1\">", &format!("sar=\"1:1\">{PROTECTION}"), 1);
    let run = play_text(text, DashConfig::default());
    assert!(run.ended, "{:?}", run.fatal);
    assert_matches_oracle(&run, &[audio_oracle()]);
    assert_eq!(
        dropped(&run),
        vec![(
            0,
            None,
            DropCause::Selection(DropReason::UnsupportedEncryption)
        )]
    );
}

#[test]
fn every_set_kind_the_client_cannot_play_is_reported_not_misplayed() {
    let cases: Vec<(&str, String, DropReason)> = vec![
        (
            "mp2t",
            base_text().replace(
                "<Representation id=\"0\" mimeType=\"video/mp4\"",
                "<Representation id=\"0\" mimeType=\"video/mp2t\"",
            ),
            DropReason::Mp2tSegments,
        ),
        (
            "webm",
            base_text().replace(
                "<Representation id=\"0\" mimeType=\"video/mp4\"",
                "<Representation id=\"0\" mimeType=\"video/webm\"",
            ),
            DropReason::WebM,
        ),
        (
            "trick",
            base_text().replacen(
                "<Representation id=\"0\"",
                "<EssentialProperty schemeIdUri=\"http://dashif.org/guidelines/trickmode\" value=\"1\"/><Representation id=\"0\"",
                1,
            ),
            DropReason::TrickMode,
        ),
        (
            "unknown-essential",
            base_text().replacen(
                "<Representation id=\"0\"",
                "<EssentialProperty schemeIdUri=\"urn:example:unknown\" value=\"1\"/><Representation id=\"0\"",
                1,
            ),
            DropReason::UnknownEssentialProperty,
        ),
    ];
    for (name, text, reason) in cases {
        let run = play_text(text, DashConfig::default());
        let d = dropped(&run);
        assert!(
            d.iter().any(|(_, _, c)| *c == DropCause::Selection(reason)),
            "{name}: {d:?}"
        );
        assert!(run.ended, "{name}: {:?}", run.fatal);
        assert_eq!(run.samples().len(), 1, "{name}: only audio plays");
    }
}

#[test]
fn a_subtitle_set_is_reported_and_ignored() {
    let text = base_text().replace(
        "</Period>",
        "<AdaptationSet id=\"9\" contentType=\"text\" mimeType=\"application/mp4\" codecs=\"stpp\"><Representation id=\"9\" bandwidth=\"1000\"><BaseURL>sub.mp4</BaseURL></Representation></AdaptationSet></Period>",
    );
    let run = play_text(text, DashConfig::default());
    assert!(run.ended);
    assert_matches_oracle(&run, &[video_oracle(), audio_oracle()]);
    assert!(dropped(&run).contains(&(2, None, DropCause::Selection(DropReason::Text))));
    assert_eq!(run.requested("sub.mp4"), 0);
}

#[test]
fn nothing_playable_is_a_typed_fatal() {
    let text = base_text()
        .replace("video/mp4", "video/webm")
        .replace("audio/mp4", "audio/webm");
    let run = play_text(text, DashConfig::default());
    assert_eq!(
        run.fatal,
        Some(FatalKind::MpdInvalid {
            reason: "no_playable_representation"
        })
    );
    assert!(!run.ended);
}

#[test]
fn a_dynamic_mpd_is_refused_cleanly_and_nothing_else_is_requested() {
    let run = drive(
        "http://dash.test/live-number/live.mpd",
        DashConfig::default(),
        &mut Origin::new(),
        &mut Drive::default(),
    );
    assert_eq!(run.fatal, Some(FatalKind::LiveNotYetSupported));
    assert!(run.errors.contains(&Error::LiveNotYetSupported));
    assert_eq!(run.requests.len(), 1);
    assert!(run.samples().is_empty());
}

fn absolute_base(text: String, base: &str) -> String {
    text.replacen(
        "<Period id=\"0\"",
        &format!("<BaseURL>{base}</BaseURL><Period id=\"0\""),
        1,
    )
}

#[test]
fn a_cross_origin_base_url_is_followed_by_default() {
    let text = absolute_base(base_text(), "http://other.test/");
    let mut origin = origin_with(text).mount("other.test", DIR);
    let run = drive(
        URL,
        DashConfig::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert!(run.ended, "{:?}", run.fatal);
    assert_matches_oracle(&run, &[video_oracle(), audio_oracle()]);
    assert!(
        run.requests
            .iter()
            .all(|r| r.url.starts_with("http://other.test/") || r.url == URL)
    );
}

#[test]
fn cross_origin_urls_are_refused_when_pinned_and_never_requested() {
    let mut cfg = DashConfig::default();
    cfg.cross_origin_urls = false;
    for base in [
        "http://other.test/m/",
        "https://dash.test/m/",
        "http://dash.test:8080/m/",
    ] {
        let text = absolute_base(base_text(), base);
        let mut origin = origin_with(text).mount("other.test", DIR);
        let run = drive(URL, cfg.clone(), &mut origin, &mut Drive::default());
        assert_eq!(
            run.fatal,
            Some(FatalKind::MpdInvalid {
                reason: "no_playable_representation"
            }),
            "{base}"
        );
        assert!(
            dropped(&run)
                .iter()
                .all(|(_, _, c)| *c == DropCause::CrossOrigin),
            "{base}"
        );
        assert_eq!(run.requests.len(), 1, "{base}: only the MPD was fetched");
    }
}

#[test]
fn a_same_origin_absolute_url_passes_the_pin() {
    let mut cfg = DashConfig::default();
    cfg.cross_origin_urls = false;
    let text = absolute_base(base_text(), "http://dash.test/timeline-ffmpeg-quirk/");
    let run = drive(URL, cfg, &mut origin_with(text), &mut Drive::default());
    assert!(run.ended, "{:?}", run.fatal);
}

#[test]
fn an_absolute_segment_template_cannot_escape_the_pin() {
    let mut cfg = DashConfig::default();
    cfg.cross_origin_urls = false;
    let text = base_text().replace(
        "media=\"chunk-stream$RepresentationID$-",
        "media=\"http://169.254.169.254/chunk-stream$RepresentationID$-",
    );
    let mut origin = origin_with(text);
    let run = drive(URL, cfg, &mut origin, &mut Drive::default());
    assert!(run.requests.iter().all(|r| !r.url.contains("169.254")));
    assert!(run.fatal.is_some());
}

#[test]
fn the_pin_follows_the_base_set_after_a_redirect() {
    let mut cfg = DashConfig::default();
    cfg.cross_origin_urls = false;
    let mut client = DashClient::new(URL, cfg);
    client.set_mpd_base("http://cdn.test/timeline-ffmpeg-quirk/manifest.mpd");
    let Some(Action::FetchMpd { .. }) = client.poll(0) else {
        panic!("mpd")
    };
    client.on_mpd(0, base_text().as_bytes()).unwrap();
    let urls: Vec<String> = std::iter::from_fn(|| client.poll(0))
        .map(|a| a.url().to_owned())
        .collect();
    assert!(!urls.is_empty());
    assert!(
        urls.iter().all(|u| u.starts_with("http://cdn.test/")),
        "{urls:?}"
    );
}

fn alternates_text(first_priority: u32, second_priority: u32) -> String {
    let t = base_text().replace(
        "<MPD ",
        "<MPD xmlns:dvb=\"urn:dvb:dash:dash-extensions:2014-1\" ",
    );
    t.replacen(
        "<Period id=\"0\"",
        &format!(
            "<BaseURL dvb:priority=\"{first_priority}\" serviceLocation=\"a\">http://a.test/</BaseURL><BaseURL dvb:priority=\"{second_priority}\" serviceLocation=\"b\">http://b.test/</BaseURL><Period id=\"0\""
        ),
        1,
    )
}

fn two_hosts(text: String) -> Origin {
    origin_with(text).mount("a.test", DIR).mount("b.test", DIR)
}

#[test]
fn base_url_alternates_are_used_in_priority_order() {
    let mut origin = two_hosts(alternates_text(2, 1));
    let run = drive(
        URL,
        DashConfig::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert!(run.ended);
    assert_eq!(
        run.requests
            .iter()
            .filter(|r| r.url.contains("a.test"))
            .count(),
        0
    );
    assert!(run.requests.iter().any(|r| r.url.contains("b.test")));
}

#[test]
fn a_failing_alternate_is_rotated_away_after_two_failures() {
    let mut origin = two_hosts(alternates_text(1, 2)).fault(Fault::always(
        "a.test/chunk-stream0-00002",
        FaultKind::Status(503),
    ));
    let run = drive(
        URL,
        DashConfig::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert!(run.ended, "{:?}", run.fatal);
    assert_matches_oracle(&run, &[video_oracle(), audio_oracle()]);
    assert_eq!(run.requested("a.test/chunk-stream0-00002"), 2);
    assert_eq!(run.requested("b.test/chunk-stream0-00002"), 1);
}

#[test]
fn alternates_that_all_fail_end_at_the_cap_having_tried_both() {
    let mut cfg = DashConfig::default();
    cfg.max_segment_failures = 6;
    let mut origin = two_hosts(alternates_text(1, 2))
        .fault(Fault::always(
            "a.test/chunk-stream0-00002",
            FaultKind::Status(503),
        ))
        .fault(Fault::always(
            "b.test/chunk-stream0-00002",
            FaultKind::Status(503),
        ));
    let run = drive(URL, cfg, &mut origin, &mut Drive::default());
    assert!(matches!(
        run.fatal,
        Some(FatalKind::SegmentFetchFailed { .. })
    ));
    let (a, b) = (
        run.requested("a.test/chunk-stream0-00002"),
        run.requested("b.test/chunk-stream0-00002"),
    );
    assert_eq!(a + b, 6, "every failure was one request");
    assert!(a >= 2 && b >= 2, "both locations were tried: {a} {b}");
}

#[test]
fn the_manifest_describes_the_selection() {
    let run = drive(
        "http://dash.test/manifest.mpd",
        DashConfig::default(),
        &mut Origin::new(),
        &mut Drive::default(),
    );
    let Some(Output::Manifest(m)) = run.outputs.first() else {
        panic!("Manifest first")
    };
    assert_eq!(m.period, 0);
    assert_eq!(m.variants.len(), 1);
    assert_eq!(m.chosen, Some(0));
    let v = &m.variants[0];
    assert_eq!(v.resolution, Some((320, 240)));
    assert_eq!(v.codecs.as_deref(), Some("avc1.4d400d"));
    assert_eq!(v.bandwidth, 58_141);
    assert_eq!(
        v.init_url.as_deref(),
        Some("http://dash.test/init-stream0.m4s")
    );
    assert_eq!(v.audio_group.as_deref(), Some("1"));
    assert_eq!(m.audio.len(), 1);
    assert_eq!(m.chosen_audio, Some(0));
    let a = &m.audio[0];
    assert_eq!(a.name, "und");
    assert_eq!(a.language.as_deref(), Some("und"));
    assert!(a.autoselect && !a.default);
    assert_eq!(
        a.init_url.as_deref(),
        Some("http://dash.test/init-stream1.m4s")
    );
}

#[test]
fn a_skipped_segment_is_one_discontinuity_and_the_rest_plays() {
    let mut origin = Origin::new().fault(Fault::times(
        "chunk-stream0-00002.m4s",
        FaultKind::Status(503),
        1,
    ));
    let run = drive(
        URL,
        DashConfig::default(),
        &mut origin,
        &mut Drive {
            skip_instead: true,
            ..Drive::default()
        },
    );
    assert!(run.ended, "{:?}", run.fatal);
    assert_eq!(run.count(|o| matches!(o, Output::Discontinuity)), 1);
    let mut video = files("init-stream0.m4s", "chunk-stream0-", 1);
    video.push(format!("{DIR}/chunk-stream0-00003.m4s"));
    let samples = run.samples();
    assert_eq!(common::loopback::sigs(&samples[&1]), oracle_of(&video));
    assert_eq!(common::loopback::sigs(&samples[&2]), audio_oracle());
}

fn switch_mpd() -> String {
    r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT3S" minBufferTime="PT1S">
 <Period id="p" start="PT0S">
  <AdaptationSet id="1" contentType="video" mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
   <Representation id="gpac" bandwidth="99692" codecs="avc1.4D400D" width="320" height="240">
    <BaseURL>http://dash.test/timeline-gpac/</BaseURL>
    <SegmentTemplate media="seg-1-$Number$.m4s" initialization="seg-1-.mp4" timescale="12800" startNumber="1" duration="25600"/>
   </Representation>
   <Representation id="ffmpeg" bandwidth="63434" codecs="avc1.4d400d" width="320" height="240">
    <BaseURL>http://dash.test/timeline-ffmpeg-quirk/</BaseURL>
    <SegmentTemplate media="chunk-stream0-$Number%05d$.m4s" initialization="init-stream0.m4s" timescale="90000" duration="90000" startNumber="1"/>
   </Representation>
  </AdaptationSet>
 </Period>
</MPD>"#
        .to_owned()
}

#[test]
fn a_representation_switch_continues_on_the_new_representation_and_reannounces() {
    const SWITCH_URL: &str = "http://dash.test/switch.mpd";
    let mut cfg = DashConfig::default();
    cfg.max_inflight_segments = 1;
    let mut origin = Origin::new().doc(SWITCH_URL, switch_mpd().into_bytes());
    let mut switched = false;
    let run = drive(
        SWITCH_URL,
        cfg,
        &mut origin,
        &mut Drive {
            hook: Some(Box::new(move |c, _| {
                if !switched && c.select_representation(1).is_ok() {
                    switched = true;
                }
            })),
            ..Drive::default()
        },
    );
    assert!(run.ended, "{:?} {:?}", run.fatal, run.errors);
    assert_eq!(
        run.count(|o| matches!(o, Output::Discontinuity)),
        0,
        "aligned set"
    );
    assert!(run.count(|o| matches!(o, Output::Init(_))) == 2);
    let segs: Vec<&str> = run
        .requests
        .iter()
        .map(|r| r.url.rsplit('/').next().unwrap())
        .filter(|n| n.ends_with(".m4s"))
        .collect();
    let first = segs.first().copied().unwrap();
    assert!(first.starts_with("seg-1-"), "{segs:?}");
    assert!(
        segs.iter().any(|n| n.starts_with("chunk-stream0-")),
        "{segs:?}"
    );
    let ids: std::collections::BTreeSet<u32> = run.samples().keys().copied().collect();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from([1]),
        "the track id survives"
    );
}

#[test]
fn switching_to_an_unknown_variant_or_before_the_start_is_an_error() {
    let mut c = DashClient::new(URL, DashConfig::default());
    assert_eq!(c.select_representation(0), Err(Error::NotSwitchable));
    let Some(Action::FetchMpd { .. }) = c.poll(0) else {
        panic!()
    };
    c.on_mpd(0, base_text().as_bytes()).unwrap();
    assert_eq!(c.select_representation(0), Err(Error::NotSwitchable));
}

#[test]
fn protocol_misuse_is_a_typed_error_and_changes_nothing() {
    let mut c = DashClient::new(URL, DashConfig::default());
    let mpd = base_text();
    assert_eq!(c.on_mpd(0, mpd.as_bytes()), Err(Error::MpdNotAwaited));
    let Some(Action::FetchMpd { url }) = c.poll(0) else {
        panic!()
    };
    assert_eq!(url, URL);
    assert!(
        c.poll(0).is_none(),
        "the MPD is requested once until retried"
    );
    c.on_mpd(0, mpd.as_bytes()).unwrap();
    assert_eq!(c.on_mpd(0, mpd.as_bytes()), Err(Error::MpdNotAwaited));
    let init = loop {
        match c.poll(0) {
            Some(Action::FetchInit { id, .. }) => break id,
            Some(_) => {}
            None => panic!("no init"),
        }
    };
    let seg = ResourceId::Segment {
        rep: init.rep(),
        number: 999,
    };
    assert_eq!(
        c.on_segment(0, seg, b"x"),
        Err(Error::UnrequestedResource { id: seg })
    );
    let data = test_fixtures::read("dash/timeline-ffmpeg-quirk/init-stream0.m4s");
    c.on_init(0, init, &data).unwrap();
    assert_eq!(
        c.on_init(0, init, &data),
        Err(Error::DuplicateResource { id: init })
    );
    assert!(!c.retry(0, init));
    assert!(!c.skip(0, init));
}

#[test]
fn after_a_fatal_everything_is_ignored() {
    let mut cfg = DashConfig::default();
    cfg.max_init_failures = 0;
    let mut c = DashClient::new(URL, cfg);
    let _ = c.poll(0);
    c.on_mpd(0, base_text().as_bytes()).unwrap();
    let Some(Action::FetchInit { id, .. }) = c.poll(0) else {
        panic!()
    };
    let class = c
        .on_failure(0, id, dash_runtime::client::Failure::Transport)
        .unwrap();
    assert_eq!(class, dash_runtime::client::FailureClass::GiveUp);
    assert!(c.poll(0).is_none());
    assert_eq!(c.on_init(0, id, b"x"), Err(Error::Stopped));
    assert_eq!(
        c.on_failure(0, id, dash_runtime::client::Failure::Transport),
        Err(Error::Stopped)
    );
    assert!(!c.retry(0, id));
}

#[test]
fn accessors_report_the_presentation() {
    let mut c = DashClient::new(URL, DashConfig::default());
    assert_eq!(c.mpd_url(), URL);
    assert_eq!(c.is_static(), None);
    assert_eq!(c.max_segment_duration_secs(), None);
    let _ = c.poll(0);
    c.on_mpd(0, base_text().as_bytes()).unwrap();
    assert_eq!(c.is_static(), Some(true));
    assert_eq!(c.max_segment_duration_secs(), Some(1));
    assert!(!c.presentation_ended());
}

#[test]
fn config_validation_names_the_first_bad_field() {
    assert!(DashConfig::default().validate().is_ok());
    let mut c = DashConfig::default();
    c.max_segment_bytes = 0;
    assert_eq!(c.validate().unwrap_err().field, "max_segment_bytes");
    let mut c = DashConfig::default();
    c.limits.max_bytes = 0;
    assert_eq!(c.validate().unwrap_err().field, "limits.max_bytes");
}
