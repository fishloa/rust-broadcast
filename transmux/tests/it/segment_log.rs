//! `SegmentSummaryLog` gates: a DASH VOD rendered byte-for-byte from main's
//! MPD, and a live `Window(n)` log that keeps exactly its window and moves the
//! rendered MPD as new segments slide it.

use broadcast_common::timescale::Timescale;
use core::num::NonZeroUsize;
use test_harness::collect::{Collected as Media, CollectedTrack as Track};
use transmux::dash::DashPackager;
use transmux::ir::{CodecConfig, Sample, TrackId, TrackSpec};
use transmux::mux_stage::TrackPart;
use transmux::segment_log::{LogCapacity, SegmentSummaryLog};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

/// An AAC track spec at `timescale` (the same config the MPD golden was
/// rendered from on `origin/main`).
fn aac_spec(id: u32, timescale_value: u32) -> TrackSpec {
    TrackSpec::new(
        TrackId::new(id),
        timescale(timescale_value),
        CodecConfig::Aac {
            esds: isobmff::mp4esds::EsdsBox::new(isobmff::mp4esds::ESDescriptor::new(
                1,
                0,
                Some(isobmff::mp4esds::DecoderConfigDescriptor::new(
                    0x40,
                    0x05,
                    false,
                    0,
                    0,
                    0,
                    Some(isobmff::mp4esds::DecoderSpecificInfo::new(vec![0x12, 0x10])),
                )),
                Some(isobmff::mp4esds::SLConfigDescriptor::predefined_two()),
            )),
            channel_count: 2,
            sample_rate: timescale_value,
            sample_size: 16,
        },
    )
}

/// A track of `count` 100-tick samples of `len` bytes each, starting at 0.
fn track(id: u32, count: usize, len: usize) -> Track {
    let samples = (0..count)
        .map(|i| {
            let dts = i as i64 * 100;
            Sample::new(vec![0u8; len], Some(dts), Some(dts), Some(100), true).expect("sample")
        })
        .collect();
    Track::new(aac_spec(id, 1000), samples)
}

/// One long (10 samples, 4 bytes each) and one short (3 samples, 8 bytes each)
/// track: their durations (1000 vs 300 ticks) and bandwidths (320 vs 640
/// bit/s) can only come from the summaries.
fn long_short_media() -> Media {
    Media::new(vec![track(1, 10, 4), track(2, 3, 8)], timescale(1000))
}

/// The MPD `origin/main` rendered for [`long_short_media`] before manifests
/// were built from segment summaries, captured from a worktree at
/// `origin/main` before this change. Byte-identical is the gate.
const LONG_SHORT_MPD: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" minBufferTime="PT2S" mediaPresentationDuration="PT1S">
  <Period id="0" start="PT0S">
    <AdaptationSet id="0" contentType="audio" mimeType="audio/mp4" segmentAlignment="true" startWithSAP="1">
      <Role schemeIdUri="urn:mpeg:dash:role:2011" value="main"/>
      <Representation id="1" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="320" audioSamplingRate="44100">
        <AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="2"/>
        <SegmentTemplate timescale="1000" duration="1000" startNumber="1" initialization="init-stream$RepresentationID$.m4s" media="chunk-stream$RepresentationID$-$Number$.m4s"/>
      </Representation>
      <Representation id="2" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="640" audioSamplingRate="44100">
        <AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="2"/>
        <SegmentTemplate timescale="1000" duration="300" startNumber="1" initialization="init-stream$RepresentationID$.m4s" media="chunk-stream$RepresentationID$-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
"#;

#[test]
fn dash_vod_long_and_short_track_renders_main_mpd_bytes() {
    let media = long_short_media();
    let specs = crate::packagers::specs(&media);
    let log = crate::packagers::log(&media);
    let mpd = DashPackager::default()
        .render(&specs, &log)
        .expect("render the VOD MPD");
    assert_eq!(
        mpd, LONG_SHORT_MPD,
        "a long and a short track must render exactly main's MPD bytes"
    );
    assert!(
        mpd.contains("bandwidth=\"640\"") && mpd.contains("bandwidth=\"320\""),
        "both tracks' bandwidths come from the summaries"
    );
    assert!(
        mpd.contains("duration=\"300\"") && mpd.contains("duration=\"1000\""),
        "both tracks' durations come from the summaries"
    );
}

/// The live MPD a caller renders per window refresh.
fn live_mpd(specs: &[TrackSpec], log: &SegmentSummaryLog) -> String {
    DashPackager {
        dynamic: true,
        availability_start_time: Some("2024-01-01T00:00:00Z".to_string()),
        publish_time: Some("2024-01-01T00:00:00Z".to_string()),
        ..DashPackager::default()
    }
    .render(specs, log)
    .expect("render the live MPD")
}

/// One track's part of a segment.
fn part(track: TrackId, start: u64, duration: u64) -> TrackPart {
    TrackPart {
        track,
        start,
        duration,
        timescale: 1000,
        bytes: 10,
        samples: 1,
    }
}

#[test]
fn window_log_keeps_exactly_n_segments_and_the_live_mpd_slides() {
    let id = TrackId::new(1);
    let specs = vec![aac_spec(1, 1000)];
    let capacity = LogCapacity::Window(NonZeroUsize::new(3).expect("non-zero window"));
    let mut log = SegmentSummaryLog::with_capacity(capacity);

    // Segment 2 is the long one; once the window has slid past it, the
    // rendered first-run duration (and so the MPD bytes) changes.
    let durations = [100u64, 100, 200, 100, 100, 100];
    let mut start = 0u64;
    for &duration in &durations[..5] {
        log.push_part(&part(id, start, duration), true)
            .expect("push segment");
        start += duration;
    }

    let summary = log.track(id).expect("the track has segments");
    assert_eq!(
        summary.segments, 3,
        "the window keeps exactly its 3 segments"
    );
    assert_eq!(
        summary.segment_durations().count(),
        3,
        "the window's runs expand to exactly 3 segments"
    );
    assert_eq!(
        summary.runs().next().map(|run| run.start),
        Some(200),
        "the retained window starts at the third segment"
    );
    let before = live_mpd(&specs, &log);
    assert!(
        before.contains("duration=\"200\""),
        "the window's first retained segment duration is the template duration"
    );

    log.push_part(&part(id, start, durations[5]), true)
        .expect("push the sixth segment");
    let summary = log.track(id).expect("the track has segments");
    assert_eq!(summary.segments, 3, "the window still keeps 3 segments");
    assert_eq!(
        summary.runs().next().map(|run| run.start),
        Some(400),
        "the retained window slid to start at the fourth segment"
    );
    let after = live_mpd(&specs, &log);
    assert_ne!(before, after, "the live MPD slides with the window");
    assert!(
        after.contains("duration=\"100\"") && !after.contains("duration=\"200\""),
        "the slid window's first retained segment duration is now 100"
    );
}
