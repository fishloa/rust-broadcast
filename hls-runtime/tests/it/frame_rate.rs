#![cfg(feature = "std")]
//! HLS FRAME-RATE derivation from track codec config (Apple 9.15).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use test_fixtures::process::tool_available;

use broadcast_hls::MasterPlaylist;
use bytes::Bytes;
use hls_runtime::server::{HlsOrigin, HlsOriginConfig};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::SegmentEntry;
use media_plane::trunk::{Trunk, TrunkConfig};
use transmux::Container;
use transmux::{CodecConfig, TrackSpec};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn origin() -> HlsOrigin {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(64)),
    );
    HlsOrigin::new(
        HlsOriginConfig::new(trunk, core::time::Duration::from_secs(4), nz(4))
            .with_instance(7)
            .with_declared_bandwidth_bps(5_000_000),
    )
    .expect("build")
}

fn render(origin: &HlsOrigin) -> String {
    match origin.master_playlist("media.m3u8") {
        Ok(s) => s,
        Err(e) => panic!("master_playlist: {e}"),
    }
}

fn specs_from_fixture(fixture: &str) -> Option<Vec<TrackSpec>> {
    let data = test_fixtures::require_bytes(fixture, "frame_rate")?;
    let media = test_harness::collect::media_from_ts(&data).ok()?;
    Some(media.tracks.into_iter().map(|t| t.spec).collect())
}

#[test]
fn h264_fixture_with_vui_timing_emits_frame_rate() {
    let Some(specs) = specs_from_fixture("ts/h264-aac.ts") else {
        return;
    };
    let origin = origin();
    origin.set_track_specs(&specs).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(
        parsed.variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(25_000))
    );
}

#[test]
fn audio_only_origin_has_no_frame_rate() {
    let origin = origin();
    origin.set_track_specs(&[]).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(parsed.variants[0].frame_rate, None);
}

#[test]
fn maximum_across_video_tracks_wins() {
    let specs25 = specs_from_fixture("ts/h264-aac.ts").unwrap_or_default();
    let specs30 =
        specs_from_fixture("scte35/ts/video-with-scte35-splice-insert.ts").unwrap_or_default();
    if specs25.is_empty() || specs30.is_empty() {
        return;
    }
    // Take the video spec from each fixture.
    let v25 = specs25
        .into_iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video spec");
    let v30 = specs30
        .into_iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video spec");
    let origin = origin();
    // unsorted input: track ids 2 then 1
    let mut a = v30;
    a.track_id = transmux::TrackId::new(2);
    let mut b = v25;
    b.track_id = transmux::TrackId::new(1);
    origin.set_track_specs(&[a, b]).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(
        parsed.variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(30_000))
    );
}

#[test]
fn set_init_reset_clears_frame_rate() {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(64)),
    );
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(trunk, core::time::Duration::from_secs(4), nz(4))
            .with_container(Container::Fmp4)
            .with_instance(7)
            .with_declared_bandwidth_bps(5_000_000),
    )
    .expect("build");
    let specs = specs_from_fixture("ts/h264-aac.ts").unwrap_or_default();
    if specs.is_empty() {
        return;
    }
    origin.set_track_specs(&specs).unwrap();
    assert_eq!(
        MasterPlaylist::parse(&render(&origin)).unwrap().variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(25_000))
    );
    origin.set_init(vec![0xAAu8; 16]);
    assert_eq!(
        MasterPlaylist::parse(&render(&origin)).unwrap().variants[0].frame_rate,
        None
    );
}

const IPHONE_30FPS: &str = "video/dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.ts";
const SEGMENT_TARGET_SECS: u32 = 1;
const NANOS_PER_SEC: u64 = 1_000_000_000;

fn ts_origin_fed_with(
    tracks: Vec<transmux::Track>,
    shape: impl Fn(usize, i64, &mut transmux::Sample) -> bool,
) -> (HlsOrigin, Vec<(Vec<u8>, Duration)>) {
    let specs: Vec<TrackSpec> = tracks.iter().map(|t| t.spec.clone()).collect();
    let mut segmenter = transmux::ts_hls::StreamingTsHlsSegmenter::new(
        specs.clone(),
        SEGMENT_TARGET_SECS,
        usize::MAX,
    )
    .expect("segmenter");
    let mut timeline: Vec<(f64, transmux::TrackId, transmux::Sample)> = Vec::new();
    for track in &tracks {
        let scale = f64::from(track.spec.timescale.get());
        for (i, sample) in track.samples.iter().enumerate() {
            let dts = sample
                .timing
                .and_then(|t| t.dts())
                .or(sample.timing.and_then(|t| t.pts()))
                .expect("timed sample");
            let mut sample = sample.clone();
            if shape(i, dts, &mut sample) {
                timeline.push((dts as f64 / scale, track.spec.track_id, sample));
            }
        }
    }
    timeline.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, id, sample) in timeline {
        feed_ts(&mut segmenter, id, sample);
    }
    broadcast_common::Stage::finish(&mut segmenter).expect("finish");

    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(64))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(64)),
    );
    let writer = trunk.segment_writer().expect("segment writer");
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(
            Arc::clone(&trunk),
            Duration::from_secs(u64::from(SEGMENT_TARGET_SECS) + 1),
            nz(64),
        )
        .with_container(Container::MpegTs)
        .with_instance(7)
        .with_declared_bandwidth_bps(5_000_000),
    )
    .expect("build");
    origin.set_track_specs(&specs).expect("specs");
    let mut start = 0u64;
    let mut published = Vec::new();
    for (n, segment) in ts_segments(&mut segmenter).into_iter().enumerate() {
        let duration = seg_duration(&segment);
        published.push((segment.bytes.to_vec(), duration));
        writer
            .publish_segment(SegmentEntry::new(
                segment.bytes,
                SegmentSeq::new(u32::try_from(n + 1).unwrap()),
                duration,
                broadcast_common::Timestamp::from_nanos(start),
                false,
            ))
            .expect("monotonic");
        start += u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    }
    (origin, published)
}

fn ts_fixture_tracks(fixture: &str, video_only: bool) -> Option<Vec<transmux::Track>> {
    let data = test_fixtures::require_bytes(fixture, "frame_rate")?;
    let media = test_harness::collect::media_from_ts(&data).ok()?;
    Some(
        media
            .tracks
            .into_iter()
            .filter(|t| !video_only || matches!(t.spec.config, CodecConfig::Hevc { .. }))
            .collect(),
    )
}

fn ts_master_frame_rate(origin: &HlsOrigin) -> Option<broadcast_hls::FrameRate> {
    MasterPlaylist::parse(&render(origin))
        .expect("parse master")
        .variants[0]
        .frame_rate
}

#[test]
fn ts_origin_without_vui_timing_measures_frame_rate_from_dts_deltas() {
    let Some(tracks) = ts_fixture_tracks(IPHONE_30FPS, false) else {
        return;
    };
    let signalled_none = {
        let origin = origin();
        let specs: Vec<TrackSpec> = tracks.iter().map(|t| t.spec.clone()).collect();
        origin.set_track_specs(&specs).unwrap();
        ts_master_frame_rate(&origin)
    };
    assert_eq!(signalled_none, None, "fixture must carry no VUI timing");
    let (origin, _segments) = ts_origin_fed_with(tracks, |_, _, _| true);
    assert_eq!(
        ts_master_frame_rate(&origin),
        Some(broadcast_hls::FrameRate::from_millihertz(30_000))
    );
}

#[test]
fn ts_origin_with_vui_timing_keeps_the_signalled_frame_rate() {
    let Some(tracks) = ts_fixture_tracks("video/hlg/sollevante-hlg-derived-1080p24-10s.ts", false)
    else {
        return;
    };
    let (origin, _segments) = ts_origin_fed_with(tracks, |_, _, _| true);
    assert_eq!(
        ts_master_frame_rate(&origin),
        Some(broadcast_hls::FrameRate::from_millihertz(24_000))
    );
}

#[test]
fn variable_frame_rate_reports_the_maximum_not_the_overall_mean() {
    let Some(tracks) = ts_fixture_tracks(IPHONE_30FPS, true) else {
        return;
    };
    let first = tracks[0]
        .samples
        .first()
        .and_then(|s| s.timing.and_then(|t| t.dts()))
        .unwrap();
    let last = tracks[0]
        .samples
        .last()
        .and_then(|s| s.timing.and_then(|t| t.dts()))
        .unwrap();
    let midpoint = first + (last - first) / 2;
    let (origin, _segments) = ts_origin_fed_with(tracks, |i, dts, sample| {
        if dts < midpoint {
            return true;
        }
        if let Some(t) = sample.timing.as_mut() {
            let doubled = t.duration().unwrap_or(2).saturating_mul(2);
            *t = t.with_duration(
                core::num::NonZeroU32::new(doubled).unwrap_or(core::num::NonZeroU32::MIN),
            )
        }
        i % 2 == 0
    });
    assert_eq!(
        ts_master_frame_rate(&origin),
        Some(broadcast_hls::FrameRate::from_millihertz(30_000))
    );
}

#[test]
fn every_video_variant_on_a_ts_origin_carries_frame_rate() {
    for (fixture, expected) in [
        (
            "video/dolby-vision/sollevante-dv-p8.1-1080p24-10s.ts",
            24_000,
        ),
        ("video/dolby-vision/sollevante-dv-p5-1080p24-10s.ts", 24_000),
        (IPHONE_30FPS, 30_000),
        ("video/hlg/sollevante-hlg-derived-1080p24-10s.ts", 24_000),
    ] {
        let Some(tracks) = ts_fixture_tracks(fixture, false) else {
            continue;
        };
        let (origin, _segments) = ts_origin_fed_with(tracks, |_, _, _| true);
        assert_eq!(
            ts_master_frame_rate(&origin),
            Some(broadcast_hls::FrameRate::from_millihertz(expected)),
            "{fixture}"
        );
    }
}

#[test]
fn mediastreamvalidator_reports_no_missing_frame_rate_on_the_measured_ts_variant() {
    if !tool_available("mediastreamvalidator", "--version") {
        eprintln!(
            "SKIP mediastreamvalidator_reports_no_missing_frame_rate_on_the_measured_ts_variant: mediastreamvalidator not on PATH"
        );
        return;
    }
    let Some(tracks) = ts_fixture_tracks(IPHONE_30FPS, false) else {
        return;
    };
    let (origin, segments) = ts_origin_fed_with(tracks, |_, _, _| true);
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join("ts-frame-rate-validator");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("master.m3u8"), render(&origin)).unwrap();
    let mut media = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n",
    );
    for (n, (bytes, duration)) in segments.iter().enumerate() {
        media.push_str(&format!(
            "#EXTINF:{:.3},\nseg{n}.ts\n",
            duration.as_secs_f64()
        ));
        std::fs::write(dir.join(format!("seg{n}.ts")), bytes).unwrap();
    }
    media.push_str("#EXT-X-ENDLIST\n");
    std::fs::write(dir.join("media.m3u8"), media).unwrap();
    let json = test_fixtures::process::msv::MsvRun::new(&dir, "master.m3u8")
        .timeout(Duration::from_secs(5))
        .deadline(Duration::from_secs(35))
        .run();
    let text = json.to_string();
    eprintln!(
        "VALIDATOR mentions FRAME-RATE: {}",
        text.contains("FRAME-RATE")
    );
    assert!(!text.contains("FRAME-RATE"), "{text}");
}

const NORMAL_SAMPLES: usize = 30;
const NORMAL_TICKS: u32 = 3000;
const GLITCH_TICKS: u32 = 60;
const SINGLE_SAMPLE_TICKS: u32 = 6000;

fn video_piece(
    tracks: &[transmux::Track],
    samples: usize,
    ticks: u32,
) -> (Vec<TrackSpec>, Vec<(Vec<u8>, Duration)>) {
    let track = &tracks[0];
    let specs = vec![track.spec.clone()];
    let mut segmenter = transmux::ts_hls::StreamingTsHlsSegmenter::new(
        specs.clone(),
        SEGMENT_TARGET_SECS,
        usize::MAX,
    )
    .expect("segmenter");
    for sample in track.samples.iter().take(samples) {
        let mut sample = sample.clone();
        if let Some(t) = sample.timing.as_mut() {
            *t = t.with_duration(
                core::num::NonZeroU32::new(ticks).unwrap_or(core::num::NonZeroU32::MIN),
            )
        } else {
            sample.timing = Some(transmux::ir::SampleTiming::duration_only(ticks).unwrap_or(
                transmux::ir::SampleTiming::DurationOnly {
                    duration: core::num::NonZeroU32::new(1).unwrap(),
                },
            ));
        }
        feed_ts(&mut segmenter, track.spec.track_id, sample);
    }
    broadcast_common::Stage::finish(&mut segmenter).expect("finish");
    let cut: Vec<_> = ts_segments(&mut segmenter)
        .into_iter()
        .map(|segment| (segment.bytes.to_vec(), seg_duration(&segment)))
        .collect();
    (specs, cut)
}

struct Feed {
    origin: HlsOrigin,
    writer: media_plane::trunk::SegmentWriter,
    next: u32,
}

impl Feed {
    fn new(specs: &[TrackSpec]) -> Feed {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(64))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(64))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(64)),
        );
        let writer = trunk.segment_writer().expect("segment writer");
        let origin = HlsOrigin::new(
            HlsOriginConfig::new(trunk, core::time::Duration::from_secs(2), nz(3))
                .with_container(Container::MpegTs)
                .with_instance(7)
                .with_declared_bandwidth_bps(5_000_000),
        )
        .expect("build");
        origin.set_track_specs(specs).expect("specs");
        Feed {
            origin,
            writer,
            next: 1,
        }
    }

    fn publish(&mut self, piece: &[(Vec<u8>, Duration)]) {
        for (bytes, duration) in piece {
            self.writer
                .publish_segment(SegmentEntry::new(
                    Bytes::from(bytes.clone()),
                    SegmentSeq::new(self.next),
                    *duration,
                    broadcast_common::Timestamp::from_nanos(u64::from(self.next) * NANOS_PER_SEC),
                    false,
                ))
                .expect("monotonic");
            self.next += 1;
        }
    }

    fn rate(&self) -> Option<broadcast_hls::FrameRate> {
        ts_master_frame_rate(&self.origin)
    }
}

#[test]
fn implausible_glitch_segment_is_never_reported() {
    let Some(tracks) = ts_fixture_tracks(IPHONE_30FPS, true) else {
        return;
    };
    let (specs, normal) = video_piece(&tracks, NORMAL_SAMPLES, NORMAL_TICKS);
    let (_, glitch) = video_piece(&tracks, NORMAL_SAMPLES, GLITCH_TICKS);
    assert_eq!(normal.len(), 1);
    assert_eq!(glitch.len(), 1);
    let mut feed = Feed::new(&specs);
    feed.publish(&normal);
    feed.publish(&glitch);
    feed.publish(&normal);
    let thirty = Some(broadcast_hls::FrameRate::from_millihertz(30_000));
    assert_eq!(feed.rate(), thirty);
    feed.publish(&normal);
    assert_eq!(feed.rate(), thirty);
    feed.publish(&normal);
    assert_eq!(feed.rate(), thirty);
}

#[test]
fn single_sample_segment_does_not_set_the_frame_rate() {
    let Some(tracks) = ts_fixture_tracks(IPHONE_30FPS, true) else {
        return;
    };
    let (specs, normal) = video_piece(&tracks, NORMAL_SAMPLES, NORMAL_TICKS);
    let (_, single) = video_piece(&tracks, 1, SINGLE_SAMPLE_TICKS);
    assert_eq!(single.len(), 1);
    let mut feed = Feed::new(&specs);
    feed.publish(&single);
    assert_eq!(feed.rate(), None);
    feed.publish(&normal);
    assert_eq!(
        feed.rate(),
        Some(broadcast_hls::FrameRate::from_millihertz(30_000))
    );
}

#[test]
fn the_maximum_survives_a_faster_track_listed_first() {
    let specs25 = specs_from_fixture("ts/h264-aac.ts").unwrap_or_default();
    let specs30 =
        specs_from_fixture("scte35/ts/video-with-scte35-splice-insert.ts").unwrap_or_default();
    if specs25.is_empty() || specs30.is_empty() {
        return;
    }
    let mut fast = specs30
        .into_iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video spec");
    let mut slow = specs25
        .into_iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video spec");
    fast.track_id = transmux::TrackId::new(1);
    slow.track_id = transmux::TrackId::new(2);
    let origin = origin();
    origin.set_track_specs(&[slow, fast]).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(
        parsed.variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(30_000)),
        "the maximum wins even when the slower track sorts last"
    );
}

/// Feed one sample to the segmenter's muxer vocabulary.
fn feed_ts(
    seg: &mut transmux::ts_hls::StreamingTsHlsSegmenter,
    track: transmux::TrackId,
    sample: transmux::Sample,
) {
    broadcast_common::Stage::feed(
        seg,
        transmux::mux_stage::MuxInput::Sample { track, sample },
        broadcast_common::Timestamp::ZERO,
    )
    .expect("feed sample");
}

/// Drain every segment the segmenter has cut so far.
fn ts_segments(
    seg: &mut transmux::ts_hls::StreamingTsHlsSegmenter,
) -> Vec<transmux::mux_stage::SegmentOut> {
    let mut out = Vec::new();
    while let Some(item) = broadcast_common::Stage::poll(seg) {
        if let transmux::mux_stage::MuxOutput::Segment(segment) = item {
            out.push(*segment);
        }
    }
    out
}

/// A segment summary's duration, in `Duration`.
fn seg_duration(segment: &transmux::mux_stage::SegmentOut) -> Duration {
    const NANOS_PER_SEC: u128 = 1_000_000_000;
    let nanos = u128::from(segment.summary.duration).saturating_mul(NANOS_PER_SEC)
        / u128::from(segment.summary.timescale.max(1));
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}
