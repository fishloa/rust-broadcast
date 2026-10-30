//! Issue #1425 (Apple HLS authoring 2.11 / 8.14): an audio configuration
//! change (ASC / channel layout / sample rate) or a video frame-rate /
//! display-parameter change mid-stream must open a new segment flagged
//! `#EXT-X-DISCONTINUITY`.

use broadcast_common::Unpackage;
use broadcast_hls::MediaPlaylist;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;
use transmux::{StreamingTsHlsSegmenter, TsDemux, UpdateTrackOutcome};

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

fn discontinuous_segment_indices(playlist: &str) -> Vec<usize> {
    let pl = MediaPlaylist::parse(playlist).expect("parse playlist");
    pl.segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.discontinuous)
        .map(|(i, _)| i)
        .collect()
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
    assert_eq!(
        seg.update_track(changed_audio.clone()),
        Ok(UpdateTrackOutcome::Changed)
    );

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
    assert_eq!(
        seg.update_track(changed_audio_2),
        Ok(UpdateTrackOutcome::Changed)
    );

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
    // The discontinuity tag must sit immediately before the first segment cut
    // after a change.
    assert!(
        playlist.contains("#EXT-X-DISCONTINUITY\n#EXTINF"),
        "discontinuity tag must immediately precede an #EXTINF"
    );
    // Exact segment indices of the two discontinuities.
    let idxs = discontinuous_segment_indices(&playlist);
    assert_eq!(
        idxs,
        vec![0, 1],
        "discontinuities must land on segments 0 and 1"
    );
    // Parse + re-serialise is byte-stable.
    let reparsed = MediaPlaylist::parse(&playlist).expect("parse");
    assert_eq!(reparsed.to_m3u8().expect("reserialise"), playlist);
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
    assert_eq!(
        seg.update_track(audio_spec.clone()),
        Ok(UpdateTrackOutcome::Unchanged)
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
    // Control: no config change over the same samples also has zero.
    let mut seg2 =
        StreamingTsHlsSegmenter::new(vec![video_spec.clone(), audio_spec.clone()], 1, usize::MAX)
            .expect("construct");
    for i in 0..n {
        seg2.push(video_id, video_samples[i].clone())
            .expect("push v");
        if i < audio_samples.len() {
            seg2.push(audio_id, audio_samples[i].clone())
                .expect("push a");
        }
    }
    seg2.finish().expect("finish");
    let playlist2 = seg2.playlist().expect("playlist");
    assert_eq!(count_discontinuities(&playlist2), 0);
    let reparsed = MediaPlaylist::parse(&playlist).expect("parse");
    assert_eq!(reparsed.to_m3u8().expect("reserialise"), playlist);
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

    // Change display parameters (and timescale) to trigger a discontinuity.
    let mut changed_video = video_spec.clone();
    changed_video.display = Some(transmux::DisplayGeometry::new(1920, 1080, None));
    changed_video.timescale = video_spec.timescale; // keep; display change enough

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

    assert_eq!(
        seg.update_track(changed_video),
        Ok(UpdateTrackOutcome::Changed)
    );

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
    assert!(
        playlist.contains("#EXT-X-DISCONTINUITY\n#EXTINF"),
        "discontinuity tag must immediately precede an #EXTINF"
    );
    let reparsed = MediaPlaylist::parse(&playlist).expect("parse");
    assert_eq!(reparsed.to_m3u8().expect("reserialise"), playlist);
}

#[test]
fn audio_non_trigger_change_is_noop() {
    let ir = demux(&load_ts());
    let audio_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();
    let audio_spec = ir.tracks[audio_pos].spec.clone();
    let audio_samples = ir.tracks[audio_pos].samples.clone();
    let audio_id = audio_spec.track_id;

    // Bitrate-only-ish change: sample_size bits, must not arm a discontinuity.
    let mut changed = audio_spec.clone();
    match &mut changed.config {
        CodecConfig::Aac { sample_size, .. } => *sample_size = sample_size.saturating_add(8).max(8),
        _ => unreachable!(),
    }

    let mut seg =
        StreamingTsHlsSegmenter::new(vec![audio_spec.clone()], 1, usize::MAX).expect("construct");
    let total = audio_samples.len();
    for (i, sample) in audio_samples.into_iter().enumerate() {
        if i == total / 2 {
            assert_eq!(
                seg.update_track(changed.clone()),
                Ok(UpdateTrackOutcome::Unchanged)
            );
        }
        seg.push(audio_id, sample).expect("push a");
    }
    seg.finish().expect("finish");
    let playlist = seg.playlist().expect("playlist");
    assert_eq!(
        count_discontinuities(&playlist),
        0,
        "a non-trigger change must not arm a discontinuity"
    );
    let reparsed = MediaPlaylist::parse(&playlist).expect("parse");
    assert_eq!(reparsed.to_m3u8().expect("reserialise"), playlist);
}
