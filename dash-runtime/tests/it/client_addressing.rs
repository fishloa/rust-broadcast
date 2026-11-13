//! A segment-address error after the initial plan drops the track with its
//! typed cause instead of stalling.

use dash_runtime::client::{
    Action, DashClient, DashConfig, DropCause, FatalKind, Output, PullLimits, ResponseMeta,
};

const DOC: &str = "http://dash.test/live/manifest.mpd";

#[test]
fn a_segment_address_error_after_the_initial_plan_drops_the_track() {
    let mpd = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic"
 profiles="urn:mpeg:dash:profile:isoff-live:2011"
 availabilityStartTime="1970-01-01T00:00:00Z"
 publishTime="2026-01-01T00:01:00Z" minimumUpdatePeriod="PT2S" minBufferTime="PT2S"
 timeShiftBufferDepth="PT30S">
 <Period id="0" start="PT0S">
  <AdaptationSet mimeType="video/mp4" contentType="video">
   <SegmentTemplate timescale="17179869184" duration="34359738368"
    initialization="init.mp4" media="seg-$Number$.m4s"/>
   <Representation id="V" bandwidth="1000" codecs="avc1.4d400d" width="320" height="240"/>
  </AdaptationSet>
 </Period>
</MPD>"#;

    let mut client = DashClient::new(
        dash_runtime::client::DashClientConfig::new(DOC)
            .with_dash(DashConfig::default())
            .with_limits(PullLimits::default()),
    );
    let now_unix_us: u64 = 1_767_225_600 * 1_000_000;
    client.set_wall_clock(0, now_unix_us);
    let Some(Action::FetchMpd { url, .. }) = client.poll(0) else {
        panic!("first action is the MPD")
    };
    assert_eq!(url, DOC);
    let meta = ResponseMeta::at(0);
    client.on_mpd_with(0, mpd.as_bytes(), &meta).unwrap();

    let mut dropped = false;
    let mut fatal = false;
    for _ in 0..20 {
        while let Some(_a) = client.poll(0) {}
        while let Some(o) = client.next_output() {
            eprintln!("OUT: {:?}", o);
            match o {
                Output::TrackDropped {
                    cause: DropCause::Addressing(_),
                    ..
                } => dropped = true,
                Output::Fatal(_) => fatal = true,
                _ => {}
            }
        }
        if dropped || fatal {
            break;
        }
    }
    assert!(
        dropped || fatal,
        "expected typed addressing drop/fatal, not a stall"
    );
}

#[test]
fn an_init_lookup_plan_error_drops_the_track() {
    // A dynamic MPD with no availabilityStartTime makes plan_representation
    // fail with MissingAvailabilityStartTime while the init segment is being
    // looked up, so the representation is dropped with a typed addressing
    // cause instead of being played without an init segment.
    let mpd = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic"
 profiles="urn:mpeg:dash:profile:isoff-live:2011"
 availabilityStartTime="1970-01-01T00:00:00Z"
 publishTime="2026-01-01T00:01:00Z" minimumUpdatePeriod="PT2S" minBufferTime="PT2S"
 timeShiftBufferDepth="PT30S">
 <Period id="0" start="PT0S">
  <AdaptationSet mimeType="video/mp4" contentType="video">
   <SegmentTemplate timescale="17179869184" duration="34359738368"
    initialization="init.mp4" media="seg-$Number$.m4s"/>
   <Representation id="V" bandwidth="1000" codecs="avc1.4d400d" width="320" height="240"/>
  </AdaptationSet>
 </Period>
</MPD>"#;

    let mut client = DashClient::new(
        dash_runtime::client::DashClientConfig::new(DOC)
            .with_dash(DashConfig::default())
            .with_limits(PullLimits::default()),
    );
    let now_unix_us: u64 = 1_767_225_600 * 1_000_000;
    client.set_wall_clock(0, now_unix_us);
    let Some(Action::FetchMpd { url, .. }) = client.poll(0) else {
        panic!("first action is the MPD")
    };
    assert_eq!(url, DOC);
    let meta = ResponseMeta::at(0);
    client.on_mpd_with(0, mpd.as_bytes(), &meta).unwrap();

    let mut dropped = false;
    let mut fatal = false;
    for _ in 0..20 {
        while let Some(_a) = client.poll(0) {}
        while let Some(o) = client.next_output() {
            match o {
                Output::TrackDropped {
                    cause: DropCause::Addressing(_),
                    ..
                } => dropped = true,
                Output::Fatal(FatalKind::UnsupportedAddressing { .. }) => fatal = true,
                _ => {}
            }
        }
        if dropped || fatal {
            break;
        }
    }
    assert!(
        dropped || fatal,
        "expected typed addressing drop/fatal, not a stall"
    );
}
