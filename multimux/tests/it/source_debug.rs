//! Audit 23 finding 229: every ingest route reports its configured stream
//! name (which feeds route lookup, log fields and metrics labels) and a
//! non-empty `Debug` body that names both the route type and its stream.
//!
//! Each route is built through its real constructor, so a `stream_name`
//! that returns `""`/a placeholder, or a `Debug` that writes nothing, is
//! observable here.

use multimux::source::PullKind;
use multimux::source::Source;
use multimux::source::dash_pull::DashPullRoute;
use multimux::source::dashif_ingest::DashifIngestRoute;
use multimux::source::hls_pull::HlsPullRoute;
use multimux::source::rtmp::RtmpRoute;
use multimux::source::rtp_udp::RtpUdpRoute;
use multimux::source::rtsp::RtspRoute;
use multimux::source::smooth_pull::SmoothPullRoute;
use multimux::source::srt::SrtRoute;
use multimux::source::ts_http::TsHttpRoute;
use multimux::source::ts_udp::TsUdpRoute;

/// A distinctive name, so a hard-coded placeholder cannot pass.
const NAME: &str = "cam1";

fn assert_route<S: Source + std::fmt::Debug>(route: &S, type_name: &str) {
    assert_eq!(route.stream_name(), NAME, "{type_name} stream name");
    let shown = format!("{route:?}");
    assert!(!shown.is_empty(), "{type_name} Debug must not be empty");
    assert!(
        shown.contains(type_name),
        "{type_name} Debug must name the type: {shown}"
    );
    assert!(
        shown.contains(NAME),
        "{type_name} Debug must carry the stream name: {shown}"
    );
}

#[test]
fn every_source_route_reports_its_name_and_debug_type() {
    assert_route(&RtspRoute::new(NAME, "rtsp://example/live"), "RtspRoute");
    assert_route(
        &TsHttpRoute::new(NAME, "http://example/live.ts"),
        "TsHttpRoute",
    );
    assert_route(
        &HlsPullRoute::new(NAME, "http://example/live.m3u8"),
        "HlsPullRoute",
    );
    assert_route(
        &SmoothPullRoute::new(NAME, "http://example/live.ism/Manifest"),
        "SmoothPullRoute",
    );
    assert_route(
        &DashPullRoute::new(NAME, "http://example/live.mpd"),
        "DashPullRoute",
    );
    assert_route(
        &RtpUdpRoute::new(NAME, "127.0.0.1:0", "v=0\r\n"),
        "RtpUdpRoute",
    );
    assert_route(&TsUdpRoute::new(NAME, "127.0.0.1:0"), "TsUdpRoute");
    assert_route(&RtmpRoute::new(NAME, "127.0.0.1:0"), "RtmpRoute");
    assert_route(&SrtRoute::new_listener(NAME, "127.0.0.1:0"), "SrtRoute");
    #[cfg(feature = "whip")]
    assert_route(&WhipRoute::new(NAME, "127.0.0.1:0"), "WhipRoute");

    let route = DashifIngestRoute::new(NAME, "127.0.0.1:0", "live");
    let shown = format!("{route:?}");
    assert!(
        !shown.is_empty(),
        "DashifIngestRoute Debug must not be empty"
    );
    assert!(shown.contains("DashifIngestRoute"), "{shown}");
    assert!(shown.contains(NAME), "{shown}");
}

/// DASH-pull contributes its documented `allow_cross_origin` field; a kind
/// without an extra contributes none.
#[test]
fn pull_kinds_contribute_their_documented_debug_fields() {
    let dash = DashPullRoute::new(NAME, "http://example/live.mpd");
    let shown = format!("{dash:?}");
    assert!(
        shown.contains("allow_cross_origin"),
        "DASH pull must report its cross-origin setting: {shown}"
    );

    let rtsp = RtspRoute::new(NAME, "rtsp://example/live");
    let shown = format!("{rtsp:?}");
    assert!(
        !shown.contains("allow_cross_origin"),
        "an extra-free kind must not invent a field: {shown}"
    );

    assert!(
        <multimux::source::dash_pull::DashPullKind as PullKind>::debug_extra(&true)
            .iter()
            .any(|(key, _)| *key == "allow_cross_origin"),
        "DashPullKind's extra must be the documented key"
    );
}

#[cfg(feature = "whip")]
use multimux::source::whip::WhipRoute;
