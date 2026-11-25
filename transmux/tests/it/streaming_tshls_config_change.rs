//! Issue #1425 (Apple HLS authoring 2.11 / 8.14): an audio configuration
//! change (ASC / channel layout / sample rate) or a video frame-rate /
//! display-parameter change mid-stream must open a new segment flagged
//! `#EXT-X-DISCONTINUITY`.

use crate::hub_fixtures::video_track;
use crate::ts_hls_stage;
use broadcast_common::Unpackage;
use broadcast_common::timescale::Timescale;
use broadcast_hls::MediaPlaylist;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;
use transmux::{Sample, StreamingTsHlsSegmenter, TsDemux, UpdateTrackOutcome};

fn load_ts() -> Vec<u8> {
    let path = test_fixtures::path("ts/h264-aac.ts");
    std::fs::read(path).expect("h264-aac.ts fixture must exist")
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
        ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    // First ASC change mid-stream.
    assert_eq!(
        seg.update_track(changed_audio.clone()),
        Ok(UpdateTrackOutcome::Changed)
    );

    for i in a..b {
        if i < video_samples.len() {
            ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        }
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    // Second ASC change mid-stream.
    assert_eq!(
        seg.update_track(changed_audio_2),
        Ok(UpdateTrackOutcome::Changed)
    );

    for i in b..n {
        if i < video_samples.len() {
            ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        }
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    ts_hls_stage::finish(&mut seg);
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
        ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    // Unchanged spec must not arm a discontinuity.
    assert_eq!(
        seg.update_track(audio_spec.clone()),
        Ok(UpdateTrackOutcome::Unchanged)
    );

    for i in m..n {
        if i < video_samples.len() {
            ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        }
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    ts_hls_stage::finish(&mut seg);
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
        ts_hls_stage::feed(&mut seg2, video_id, video_samples[i].clone());
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg2, audio_id, audio_samples[i].clone());
        }
    }
    ts_hls_stage::finish(&mut seg2);
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
        ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    assert_eq!(
        seg.update_track(changed_video),
        Ok(UpdateTrackOutcome::Changed)
    );

    for i in a..b {
        if i < video_samples.len() {
            ts_hls_stage::feed(&mut seg, video_id, video_samples[i].clone());
        }
        if i < audio_samples.len() {
            ts_hls_stage::feed(&mut seg, audio_id, audio_samples[i].clone());
        }
    }

    ts_hls_stage::finish(&mut seg);
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
        ts_hls_stage::feed(&mut seg, audio_id, sample);
    }
    ts_hls_stage::finish(&mut seg);
    let playlist = seg.playlist().expect("playlist");
    assert_eq!(
        count_discontinuities(&playlist),
        0,
        "a non-trigger change must not arm a discontinuity"
    );
    let reparsed = MediaPlaylist::parse(&playlist).expect("parse");
    assert_eq!(reparsed.to_m3u8().expect("reserialise"), playlist);
}

/// Flip one AudioSpecificConfig byte without touching the track's declared
/// sample rate or channel count, so only the AAC config arm can see the change.
fn with_changed_asc(mut spec: transmux::TrackSpec, byte: usize, mask: u8) -> transmux::TrackSpec {
    match &mut spec.config {
        CodecConfig::Aac { esds, .. } => {
            let dsi = esds
                .es_descriptor
                .decoder_config
                .as_mut()
                .and_then(|dc| dc.decoder_specific_info.as_mut())
                .expect("AAC track carries an AudioSpecificConfig");
            dsi.data[byte] ^= mask;
        }
        other => panic!("expected AAC, got {other:?}"),
    }
    spec
}

#[test]
fn aac_asc_change_at_the_same_rate_and_channels_arms_discontinuity() {
    let ir = demux(&load_ts());
    let audio_pos = ir
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();
    let audio_spec = ir.tracks[audio_pos].spec.clone();
    let audio_samples = ir.tracks[audio_pos].samples.clone();
    let audio_id = audio_spec.track_id;
    assert!(audio_samples.len() > 4, "fixture must carry AAC samples");

    let changed = with_changed_asc(audio_spec.clone(), 1, 0x08);

    let mut seg =
        StreamingTsHlsSegmenter::new(vec![audio_spec.clone()], 1, usize::MAX).expect("construct");

    // Cut one segment so the change lands mid-stream.
    let cut_at = audio_samples.len() / 2;
    for sample in &audio_samples[..cut_at] {
        ts_hls_stage::feed(&mut seg, audio_id, sample.clone());
    }
    assert!(
        !ts_hls_stage::drain(&mut seg).is_empty(),
        "the fixture must cut at least one segment before the change"
    );

    assert_eq!(
        seg.update_track(changed),
        Ok(UpdateTrackOutcome::Changed),
        "a different AudioSpecificConfig at the same rate/channels is a change"
    );

    for sample in &audio_samples[cut_at..] {
        ts_hls_stage::feed(&mut seg, audio_id, sample.clone());
    }
    ts_hls_stage::finish(&mut seg);
    let playlist = seg.playlist().expect("playlist");
    assert!(
        playlist.contains("#EXT-X-DISCONTINUITY"),
        "the change must flag the next segment discontinuous:\n{playlist}"
    );

    // Control: the identical ASC is Unchanged.
    let mut control =
        StreamingTsHlsSegmenter::new(vec![audio_spec.clone()], 1, usize::MAX).expect("construct");
    for sample in &audio_samples[..cut_at] {
        ts_hls_stage::feed(&mut control, audio_id, sample.clone());
    }
    assert_eq!(
        control.update_track(audio_spec),
        Ok(UpdateTrackOutcome::Unchanged)
    );
}

fn push_one_second_frames(
    seg: &mut StreamingTsHlsSegmenter,
    id: transmux::TrackId,
    frame_ticks: u32,
    frames_per_second: usize,
    seconds: usize,
) {
    let gop = frames_per_second * 2;
    for i in 0..frames_per_second * seconds {
        let sync = i % gop == 0;
        let sample =
            Sample::new(vec![0u8; 4], None, None, Some(frame_ticks), sync).expect("valid timing");
        ts_hls_stage::feed(seg, id, sample);
    }
}

#[test]
fn anchor_timescale_change_recomputes_the_target_ticks() {
    let spec = video_track();
    let id = spec.track_id;
    let mut changed = spec.clone();
    changed.timescale = Timescale::try_new(30_000).expect("non-zero");

    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 2, usize::MAX).expect("construct");
    assert_eq!(
        seg.update_track(changed),
        Ok(UpdateTrackOutcome::Changed),
        "a timescale change is a discontinuity"
    );
    // 4 s of 0.1 s frames at 30 kHz, keyframe every 2 s.
    push_one_second_frames(&mut seg, id, 3_000, 10, 4);
    let cuts = ts_hls_stage::drain(&mut seg);
    assert!(
        !cuts.is_empty(),
        "the 2 s target must cut at the 2 s keyframe under the new timescale"
    );
    assert_eq!(
        ts_hls_stage::duration(&cuts[0]),
        core::time::Duration::from_secs(2),
        "target_ticks must be re-derived at 30 kHz, not keep 90 kHz ticks"
    );
}

#[test]
fn a_display_only_change_keeps_the_cut_cadence() {
    let spec = video_track();
    let id = spec.track_id;
    let mut changed = spec.clone();
    changed.display = Some(transmux::DisplayGeometry::new(1920, 1080, None));

    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 2, usize::MAX).expect("construct");
    assert_eq!(seg.update_track(changed), Ok(UpdateTrackOutcome::Changed));
    // 4 s of 1 s frames at 90 kHz, keyframe every 2 s.
    push_one_second_frames(&mut seg, id, 9_000, 10, 4);
    let cuts = ts_hls_stage::drain(&mut seg);
    assert!(!cuts.is_empty());
    assert_eq!(
        ts_hls_stage::duration(&cuts[0]),
        core::time::Duration::from_secs(2)
    );
}
