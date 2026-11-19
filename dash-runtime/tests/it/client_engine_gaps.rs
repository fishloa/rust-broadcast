//! Engine decisions the fault matrix leaves open: the rounded
//! `@maxSegmentDuration`, the bookkeeping accessors, the MPD retry gate, the
//! addressing causes surfaced as a fatal, the `XlinkSkipped` notice, a
//! single-segment plan's last segment, and BaseURL candidate rotation.

use crate::common::loopback::{Drive, Fault, FaultKind, Origin, drive, mpd_text};
use broadcast_common::stage::Timestamp;
use dash_runtime::client::{
    Action, DashClient, DashClientConfig, DashConfig, FailureClass, FatalKind, Notice, Output,
    PullLimits, ResourceId, StateSize,
};
use url::Url;

const DIR: &str = "timeline-ffmpeg-quirk";
const URL: &str = "http://dash.test/timeline-ffmpeg-quirk/manifest.mpd";

fn now(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

fn base_text() -> String {
    mpd_text("timeline-ffmpeg-quirk/manifest.mpd")
}

fn client(url: &str, dash: DashConfig) -> DashClient {
    DashClient::new(
        DashClientConfig::new(Url::parse(url).expect("URL"))
            .with_dash(dash)
            .with_limits(PullLimits::default()),
    )
}

#[test]
fn the_max_segment_duration_is_rounded_up_to_whole_seconds() {
    let mut c = client(URL, DashConfig::default());
    let _ = c.poll(now(0));
    let text = base_text().replace("PT1.0S", "PT2.5S");
    c.on_mpd(now(0), text.as_bytes()).unwrap();
    assert_eq!(c.max_segment_duration_secs(), Some(3));
    let mut c = client(URL, DashConfig::default());
    let _ = c.poll(now(0));
    let text = base_text().replace("PT1.0S", "PT4S");
    c.on_mpd(now(0), text.as_bytes()).unwrap();
    assert_eq!(c.max_segment_duration_secs(), Some(4));
}

#[test]
fn the_bookkeeping_accessors_report_the_planned_presentation() {
    let mut c = client(URL, DashConfig::default());
    assert_eq!(c.state_size().periods, 0);
    assert_eq!(c.state_size().reps, 0);
    let _ = c.poll(now(0));
    c.on_mpd(now(0), base_text().as_bytes()).unwrap();
    let size = c.state_size();
    assert_eq!(size.periods, 1);
    assert_eq!(size.reps, 2, "one video and one audio Representation");
    assert_ne!(size, StateSize::default());
}

#[test]
fn awaits_follows_the_in_flight_fetch() {
    let mut c = client(URL, DashConfig::default());
    let _ = c.poll(now(0));
    c.on_mpd(now(0), base_text().as_bytes()).unwrap();
    let Some(Action::FetchInit { id, .. }) = c.poll(now(0)) else {
        panic!("no init fetch");
    };
    assert!(c.awaits(id));
    let other = ResourceId::Segment {
        rep: id.rep(),
        number: 99,
    };
    assert!(!c.awaits(other));
    c.on_failure(now(2_000), id, dash_runtime::client::Failure::Transport)
        .unwrap();
    assert!(!c.awaits(id), "a failed fetch is no longer awaited");
}

#[test]
fn retry_mpd_does_not_request_a_second_mpd_while_one_is_in_flight() {
    let mut c = client(URL, DashConfig::default());
    let Some(Action::FetchMpd { .. }) = c.poll(now(0)) else {
        panic!("the first poll asks for the MPD");
    };
    c.retry_mpd(now(1_000));
    assert!(c.poll(now(2_000)).is_none());
}

fn two_entry_list(second: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT4S" minBufferTime="PT2S">
<Period id="0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
<Representation id="0" bandwidth="1" codecs="avc1.4d400d">
<SegmentList timescale="90000" duration="180000">
<Initialization sourceURL="init-stream0.m4s"/>
<SegmentURL media="chunk-stream0-00001.m4s"/>
<SegmentURL media="{second}"/>
</SegmentList>
</Representation></AdaptationSet></Period></MPD>"#
    )
}

#[test]
fn a_cross_origin_segment_url_is_a_fatal_cross_origin() {
    let mut dash = DashConfig::default();
    dash.cross_origin_urls = false;
    let text = two_entry_list("http://other.test/x.m4s");
    let mut origin = Origin::new().doc(URL, text.into_bytes());
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive {
            dash,
            ..Drive::default()
        },
    );
    assert_eq!(
        run.fatal,
        Some(FatalKind::UnsupportedAddressing {
            reason: "cross_origin"
        }),
        "{:#?}",
        run.outputs
    );
}

#[test]
fn a_data_url_segment_is_a_fatal_unsupported_data_url() {
    let text = two_entry_list("data:text/plain,x");
    let mut origin = Origin::new().doc(URL, text.into_bytes());
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert_eq!(
        run.fatal,
        Some(FatalKind::UnsupportedAddressing { reason: "data_url" }),
        "{:#?}",
        run.outputs
    );
}

#[test]
fn an_xlink_is_noticed_and_a_plain_drop_is_not() {
    let with_xlink = base_text().replace(
        r#"<AdaptationSet id="1" contentType="audio""#,
        r#"<AdaptationSet xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="http://x/set.xml" id="2" contentType="audio""#,
    );
    let mut origin = Origin::new().doc(URL, with_xlink.into_bytes());
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert_eq!(
        run.count(|o| matches!(o, Output::Notice(Notice::XlinkSkipped))),
        1
    );

    let with_text = base_text().replace(
        r#"<AdaptationSet id="1" contentType="audio""#,
        r#"<AdaptationSet id="2" contentType="text" mimeType="text/vtt"/><AdaptationSet id="1" contentType="audio""#,
    );
    let mut origin = Origin::new().doc(URL, with_text.into_bytes());
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert_eq!(
        run.count(|o| matches!(o, Output::Notice(Notice::XlinkSkipped))),
        0,
        "a text set dropped for its class is not an xlink"
    );
}

fn single_segment_mpd() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="utf-8"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT4S" minBufferTime="PT2S">
<Period id="0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
<Representation id="0" bandwidth="1" codecs="avc1.4d400d">
<SegmentTemplate timescale="90000" initialization="init-stream0.m4s" media="whole-$Number$.m4s"/>
</Representation></AdaptationSet></Period></MPD>"#,
    )
}

#[test]
fn a_single_segment_plan_that_404s_is_a_failure_not_a_missing_tail() {
    let mut origin = Origin::new().doc(URL, single_segment_mpd().into_bytes());
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert!(
        !run.classes
            .iter()
            .any(|c| matches!(c, FailureClass::EndTail)),
        "{:?}",
        run.classes
    );
    assert!(
        matches!(run.fatal, Some(FatalKind::SegmentFetchFailed { .. })),
        "{:?}",
        run.fatal
    );
}

#[test]
fn a_base_url_candidate_rotates_after_two_consecutive_failures() {
    let text = single_segment_mpd()
        .replace("whole-$Number$.m4s", "chunk-stream0-0000$Number$.m4s")
        .replace(
        "<Period id=\"0\" start=\"PT0S\">",
        "<BaseURL>http://dash.test/broken/</BaseURL><BaseURL>http://dash.test/timeline-ffmpeg-quirk/</BaseURL><Period id=\"0\" start=\"PT0S\">",
    );
    let mut origin = Origin::new()
        .doc(URL, text.into_bytes())
        .fault(Fault::always("/broken/", FaultKind::Status(500)));
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert_eq!(
        run.requests
            .iter()
            .map(|r| r.url.as_str())
            .collect::<Vec<_>>(),
        vec![
            "http://dash.test/timeline-ffmpeg-quirk/manifest.mpd",
            "http://dash.test/broken/init-stream0.m4s",
            "http://dash.test/broken/chunk-stream0-00001.m4s",
            "http://dash.test/broken/init-stream0.m4s",
            "http://dash.test/timeline-ffmpeg-quirk/chunk-stream0-00001.m4s",
            "http://dash.test/timeline-ffmpeg-quirk/init-stream0.m4s",
        ],
        "the second init failure is what rotates the candidate"
    );
    assert!(
        run.requests
            .iter()
            .any(|r| r.url.ends_with("/timeline-ffmpeg-quirk/init-stream0.m4s")),
        "the second candidate serves the init: {:?}",
        run.requests
            .iter()
            .map(|r| r.url.as_str())
            .collect::<Vec<_>>()
    );
    let _ = DIR;
}

#[test]
fn a_segment_byte_range_that_cannot_be_a_span_names_its_cause() {
    let text = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT4S" minBufferTime="PT2S">
<Period id="0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
<Representation id="0" bandwidth="1" codecs="avc1.4d400d">
<SegmentList timescale="90000" duration="180000">
<Initialization sourceURL="init-stream0.m4s"/>
<SegmentURL media="chunk-stream0-00001.m4s"/>
<SegmentURL media="chunk-stream0-00001.m4s" mediaRange="5-"/>
</SegmentList>
</Representation></AdaptationSet></Period></MPD>"#,
    );
    let mut origin = Origin::new().doc(URL, text.into_bytes());
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert_eq!(
        run.fatal,
        Some(FatalKind::UnsupportedAddressing {
            reason: "open_ended_range"
        }),
        "{:#?}",
        run.outputs
    );
}
