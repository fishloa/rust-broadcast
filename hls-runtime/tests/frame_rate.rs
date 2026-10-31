#![cfg(feature = "std")]
//! HLS FRAME-RATE derivation from track codec config (Apple 9.15).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_hls::MasterPlaylist;
use bytes::Bytes;
use hls_runtime::server::{Container, HlsOrigin};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::SegmentEntry;
use media_plane::trunk::{Trunk, TrunkConfig};
use transmux::ts_demux::TsDemux;
use transmux::{CodecConfig, TrackSpec};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn origin() -> HlsOrigin {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
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
    let media = TsDemux::new().demux(&data).ok()?;
    Some(media.tracks.into_iter().map(|t| t.spec).collect())
}

#[test]
fn h264_fixture_with_vui_timing_emits_frame_rate() {
    let Some(specs) = specs_from_fixture("ts/h264_aac.ts") else {
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
    let specs25 = specs_from_fixture("ts/h264_aac.ts").unwrap_or_default();
    let specs30 =
        specs_from_fixture("scte35-ssai/ts/video_with_scte35_splice_insert.ts").unwrap_or_default();
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
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .container(Container::Fmp4)
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    let specs = specs_from_fixture("ts/h264_aac.ts").unwrap_or_default();
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

const IPHONE_30FPS: &str = "dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.ts";
const SEGMENT_TARGET_SECS: u32 = 1;
const NANOS_PER_SEC: u64 = 1_000_000_000;

fn ts_origin_fed_with(
    tracks: Vec<transmux::Track>,
    shape: impl Fn(usize, i64, &mut transmux::Sample) -> bool,
) -> (HlsOrigin, Vec<(Vec<u8>, f64)>) {
    let specs: Vec<TrackSpec> = tracks.iter().map(|t| t.spec.clone()).collect();
    let mut segmenter = transmux::ts_hls::StreamingTsHlsSegmenter::new(
        specs.clone(),
        SEGMENT_TARGET_SECS,
        usize::MAX,
    )
    .expect("segmenter");
    let mut timeline: Vec<(f64, transmux::TrackId, transmux::Sample)> = Vec::new();
    for track in &tracks {
        let scale = f64::from(track.spec.timescale);
        for (i, sample) in track.samples.iter().enumerate() {
            let dts = sample.dts.or(sample.pts).expect("timed sample");
            let mut sample = sample.clone();
            if shape(i, dts, &mut sample) {
                timeline.push((dts as f64 / scale, track.spec.track_id, sample));
            }
        }
    }
    timeline.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, id, sample) in timeline {
        segmenter.push(id, sample).expect("push");
    }
    segmenter.finish().expect("finish");

    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(64), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("segment writer");
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .container(Container::MpegTs)
        .target_duration_secs(f64::from(SEGMENT_TARGET_SECS) + 1.0)
        .window_segments(nz(64))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    origin.set_track_specs(&specs).expect("specs");
    let mut start = 0u64;
    let mut published = Vec::new();
    for (n, segment) in segmenter.take_ready().into_iter().enumerate() {
        published.push((segment.bytes.clone(), segment.duration));
        let duration = Duration::from_secs_f64(segment.duration);
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(segment.bytes),
                SegmentSeq::new(u32::try_from(n + 1).unwrap()),
                duration,
                broadcast_common::Timestamp::from_nanos(start),
                transmux::SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("monotonic");
        start += (segment.duration * NANOS_PER_SEC as f64) as u64;
    }
    (origin, published)
}

fn ts_fixture_tracks(fixture: &str, video_only: bool) -> Option<Vec<transmux::Track>> {
    let data = test_fixtures::require_bytes(fixture, "frame_rate")?;
    let media = TsDemux::new().demux(&data).ok()?;
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
    let Some(tracks) = ts_fixture_tracks("hlg/sollevante-hlg-derived-1080p24-10s.ts", false) else {
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
    let first = tracks[0].samples.first().and_then(|s| s.dts).unwrap();
    let last = tracks[0].samples.last().and_then(|s| s.dts).unwrap();
    let midpoint = first + (last - first) / 2;
    let (origin, _segments) = ts_origin_fed_with(tracks, |i, dts, sample| {
        if dts < midpoint {
            return true;
        }
        sample.duration = sample.duration.map(|d| d * 2);
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
        ("dolby-vision/sollevante-dv-p8.1-1080p24-10s.ts", 24_000),
        ("dolby-vision/sollevante-dv-p5-1080p24-10s.ts", 24_000),
        (IPHONE_30FPS, 30_000),
        ("hlg/sollevante-hlg-derived-1080p24-10s.ts", 24_000),
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

fn tool_available(tool: &str, arg: &str) -> bool {
    std::process::Command::new(tool)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
        media.push_str(&format!("#EXTINF:{duration:.3},\nseg{n}.ts\n"));
        std::fs::write(dir.join(format!("seg{n}.ts")), bytes).unwrap();
    }
    media.push_str("#EXT-X-ENDLIST\n");
    std::fs::write(dir.join("media.m3u8"), media).unwrap();
    let report = dir.join("report.json");
    let mut cmd = std::process::Command::new("mediastreamvalidator");
    cmd.current_dir(&dir)
        .args(["--quiet", "-t", "5", "-O"])
        .arg(&report)
        .arg("master.m3u8");
    test_bounded::output_bounded(&mut cmd, Duration::from_secs(35)).expect("run validator");
    let text = std::fs::read_to_string(&report).expect("validator report");
    eprintln!(
        "VALIDATOR mentions FRAME-RATE: {}",
        text.contains("FRAME-RATE")
    );
    assert!(!text.contains("FRAME-RATE"), "{text}");
}
