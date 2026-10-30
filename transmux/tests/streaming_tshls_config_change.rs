//! Issue #1425 (Apple HLS authoring 2.11 / 8.14): an audio configuration
//! change (ASC / channel layout / sample rate) or a video frame-rate /
//! display-parameter change mid-stream must open a new segment flagged
//! `#EXT-X-DISCONTINUITY`.

use broadcast_common::Unpackage;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;
use transmux::{StreamingTsHlsSegmenter, TsDemux};

fn load_ts() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
}

fn demux(ts: &[u8]) -> Media {
    TsDemux::new().unpackage(ts).expect("demux TS")
}

fn count_discontinuities(playlist: &str) -> usize {
    playlist
        .lines()
        .filter(|l| l.trim() == "#EXT-X-DISCONTINUITY")
        .count()
}

#[test]
fn audio_config_change_arms_discontinuity() {
    let ir = demux(&load_ts());
    let video_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    let audio_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();
    let video_spec = ir.tracks[video_pos].spec.clone();
    let audio_spec = ir.tracks[audio_pos].spec.clone();
    let video_samples = ir.tracks[video_pos].samples.clone();
    let audio_samples = ir.tracks[audio_pos].samples.clone();
    let video_id = video_spec.track_id;
    let audio_id = audio_spec.track_id;

    // First config change.
    let mut changed_audio = audio_spec.clone();
    match &mut changed_audio.config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate *= 2,
        _ => unreachable!(),
    }

    // Second config change (different field).
    let mut changed_audio_2 = changed_audio.clone();
    match &mut changed_audio_2.config {
        CodecConfig::Aac { channel_count, .. } => {
            *channel_count = channel_count.saturating_add(1).max(1)
        }
        _ => unreachable!(),
    }

    let mut seg =
        StreamingTsHlsSegmenter::new(vec![video_spec.clone(), audio_spec.clone()], 1, usize::MAX)
            .expect("construct");

    // Push a few video + audio samples.
    let n = video_samples.len();
    let a = n / 3;
    let b = 2 * n / 3;
    for i in 0..a {
        seg.push(video_id, video_samples[i].clone())
            .expect("push v");
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a");
        }
    }

    // First ASC change mid-stream.
    assert!(seg.update_track(changed_audio.clone()).expect("update 1"));

    for i in a..b {
        if i < video_samples.len() {
            seg.push(video_id, video_samples[i].clone())
                .expect("push v2");
        }
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a2");
        }
    }

    // Second ASC change mid-stream.
    assert!(seg.update_track(changed_audio_2).expect("update 2"));

    for i in b..n {
        if i < video_samples.len() {
            seg.push(video_id, video_samples[i].clone())
                .expect("push v3");
        }
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a3");
        }
    }

    seg.finish().expect("finish");
    let playlist = seg.playlist().expect("playlist");
    assert_eq!(
        count_discontinuities(&playlist),
        2,
        "two distinct audio config changes must produce two discontinuities"
    );
}

#[test]
fn audio_config_unchanged_is_noop() {
    let ir = demux(&load_ts());
    let video_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    let audio_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();
    let video_spec = ir.tracks[video_pos].spec.clone();
    let audio_spec = ir.tracks[audio_pos].spec.clone();
    let video_samples = ir.tracks[video_pos].samples.clone();
    let audio_samples = ir.tracks[audio_pos].samples.clone();
    let video_id = video_spec.track_id;
    let audio_id = audio_spec.track_id;

    let mut seg =
        StreamingTsHlsSegmenter::new(vec![video_spec.clone(), audio_spec.clone()], 1, usize::MAX)
            .expect("construct");

    let n = video_samples.len();
    let m = n / 2;
    for i in 0..m {
        seg.push(video_id, video_samples[i].clone())
            .expect("push v");
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a");
        }
    }

    // Unchanged spec must not arm a discontinuity.
    assert!(
        !seg.update_track(audio_spec.clone())
            .expect("update unchanged")
    );

    for i in m..n {
        if i < video_samples.len() {
            seg.push(video_id, video_samples[i].clone())
                .expect("push v2");
        }
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a2");
        }
    }

    seg.finish().expect("finish");
    let playlist = seg.playlist().expect("playlist");
    assert_eq!(
        count_discontinuities(&playlist),
        0,
        "an unchanged update_track must produce no discontinuity"
    );
}

#[test]
fn video_frame_rate_change_arms_discontinuity() {
    let ir = demux(&load_ts());
    let video_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    let audio_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();
    let video_spec = ir.tracks[video_pos].spec.clone();
    let audio_spec = ir.tracks[audio_pos].spec.clone();
    let video_samples = ir.tracks[video_pos].samples.clone();
    let audio_samples = ir.tracks[audio_pos].samples.clone();
    let video_id = video_spec.track_id;
    let audio_id = audio_spec.track_id;

    let mut changed_video = video_spec.clone();
    match &mut changed_video.config {
        CodecConfig::Avc { width, .. } => *width = width.saturating_add(640),
        _ => unreachable!(),
    }

    let mut seg =
        StreamingTsHlsSegmenter::new(vec![video_spec.clone(), audio_spec.clone()], 1, usize::MAX)
            .expect("construct");

    let n = video_samples.len();
    let a = n / 3;
    let b = 2 * n / 3;
    for i in 0..a {
        seg.push(video_id, video_samples[i].clone())
            .expect("push v");
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a");
        }
    }

    assert!(seg.update_track(changed_video).expect("update video"));

    for i in a..b {
        if i < video_samples.len() {
            seg.push(video_id, video_samples[i].clone())
                .expect("push v2");
        }
        if i < audio_samples.len() {
            seg.push(audio_id, audio_samples[i].clone())
                .expect("push a2");
        }
    }

    seg.finish().expect("finish");
    let playlist = seg.playlist().expect("playlist");
    assert_eq!(
        count_discontinuities(&playlist),
        1,
        "a video display/param change must produce one discontinuity"
    );
}
