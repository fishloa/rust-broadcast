//! `StreamingTsHlsSegmenter::add_track` gate (issue #624): proves a track can
//! join a live segmenter *after* construction and *after* segments have
//! already been cut, mirroring the real bug this issue fixes — a live
//! `StreamingTsDemux` resolves audio's `DemuxEvent::TrackAdded` after video's
//! (the audio PID's first frame commonly parses after the first video
//! keyframe), so a consumer building `StreamingTsHlsSegmenter` at the first
//! video keyframe used to get a permanently video-only segmenter with no way
//! to add audio later.
//!
//! Real fixture: `fixtures/ts/h264_aac.ts` (2-track H.264 + AAC MPEG-2 TS).
//! We deliberately SIMULATE the late-audio-resolution scenario: build the
//! segmenter with only the video `TrackSpec`, push enough video samples to
//! force at least one video-only segment cut, call `add_track` with the audio
//! `TrackSpec`, then push the remaining video and all of the audio samples
//! interleaved by decode time. Segments cut *before* `add_track` must have no
//! audio elementary stream at all (PMT or PES); segments cut *after* must
//! declare the audio track in the PMT and actually carry its PES data.

use broadcast_common::Unpackage;
use transmux::media::Media;
use transmux::pipeline::{CodecConfig, Sample};
use transmux::{StreamingTsHlsSegmenter, TsDemux, TsSegment};

fn load_ts() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
}

fn demux(ts: &[u8]) -> Media {
    TsDemux::new().unpackage(ts).expect("demux TS")
}

/// Demux one produced `.ts` segment back through the batch demuxer so we can
/// inspect what elementary streams (PMT-declared *and* sample-bearing) it
/// actually carries — the only way to independently verify `add_track`
/// genuinely changed the muxed output, not just the in-memory track list.
fn demux_segment(bytes: &[u8]) -> Media {
    TsDemux::new()
        .unpackage(bytes)
        .expect("demux produced segment")
}

fn is_avc(c: &CodecConfig) -> bool {
    matches!(c, CodecConfig::Avc { .. })
}

fn is_aac(c: &CodecConfig) -> bool {
    matches!(c, CodecConfig::Aac { .. })
}

#[test]
fn add_track_registers_audio_after_video_only_segments_were_already_cut() {
    let ir = demux(&load_ts());
    assert_eq!(ir.tracks.len(), 2, "h264_aac.ts must be a 2-track fixture");

    let video_pos = ir
        .tracks
        .iter()
        .position(|t| is_avc(&t.spec.config))
        .expect("AVC video track");
    let audio_pos = ir
        .tracks
        .iter()
        .position(|t| is_aac(&t.spec.config))
        .expect("AAC audio track");

    let video_spec = ir.tracks[video_pos].spec.clone();
    let audio_spec = ir.tracks[audio_pos].spec.clone();
    let video_samples: Vec<Sample> = ir.tracks[video_pos].samples.clone();
    let audio_samples: Vec<Sample> = ir.tracks[audio_pos].samples.clone();
    assert!(!audio_samples.is_empty(), "fixture must carry AAC samples");

    let video_track_id = video_spec.track_id;
    let audio_track_id = audio_spec.track_id;
    let video_scale = video_spec.timescale.max(1) as u64;
    let audio_scale = audio_spec.timescale.max(1) as u64;

    // ── Phase 1: build video-only (as if audio hadn't resolved yet) ─────────
    let mut seg = StreamingTsHlsSegmenter::new(vec![video_spec], 1, usize::MAX)
        .expect("construct video-only streaming segmenter");

    let mut before_segments: Vec<TsSegment> = Vec::new();
    let mut split_idx: Option<usize> = None;
    for (i, s) in video_samples.iter().enumerate() {
        if let Some(cut) = seg.push(video_track_id, s.clone()).expect("push video") {
            before_segments.push(cut);
            split_idx = Some(i);
            break;
        }
    }
    let split_idx = split_idx.expect(
        "fixture must be long enough to force at least one video-only segment cut \
         before its samples are exhausted",
    );
    assert!(
        !before_segments.is_empty(),
        "at least one segment must have been cut before add_track"
    );
    assert!(
        split_idx + 1 < video_samples.len(),
        "fixture must have video samples remaining after the forced cut, to push \
         alongside audio in phase 2"
    );

    // ── Phase 2: audio "resolves" late — register it now ────────────────────
    seg.add_track(audio_spec).expect("add_track: audio");

    // Duplicate track_id must be rejected.
    let dup = transmux::pipeline::TrackSpec::new(video_track_id, video_scale as u32, {
        // Reuse the already-registered video codec config via the IR clone.
        let re_demux = demux(&load_ts());
        re_demux.tracks[video_pos].spec.config.clone()
    });
    assert!(
        seg.add_track(dup).is_err(),
        "add_track must reject a track_id collision"
    );

    // ── Phase 3: push the remaining video + all audio, interleaved by decode time ─
    let mut acc: u64 = video_samples[..=split_idx]
        .iter()
        .map(|s| s.duration as u64)
        .sum();
    let mut items: Vec<(f64, bool, usize)> = Vec::new();
    for (i, s) in video_samples.iter().enumerate().skip(split_idx + 1) {
        items.push((acc as f64 / video_scale as f64, true, i));
        acc += s.duration as u64;
    }
    let mut aacc: u64 = 0;
    for (i, s) in audio_samples.iter().enumerate() {
        items.push((aacc as f64 / audio_scale as f64, false, i));
        aacc += s.duration as u64;
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));

    let mut after_segments: Vec<TsSegment> = Vec::new();
    for (_, is_video, idx) in items {
        let (track_id, sample) = if is_video {
            (video_track_id, video_samples[idx].clone())
        } else {
            (audio_track_id, audio_samples[idx].clone())
        };
        if let Some(cut) = seg.push(track_id, sample).expect("push") {
            after_segments.push(cut);
        }
    }
    if let Some(cut) = seg.finish().expect("finish") {
        after_segments.push(cut);
    }
    assert!(
        !after_segments.is_empty(),
        "at least one segment must be cut after add_track"
    );

    // ── Assertions: segments before add_track are video-only ────────────────
    for (i, s) in before_segments.iter().enumerate() {
        let media = demux_segment(&s.bytes);
        assert_eq!(
            media.tracks.len(),
            1,
            "before-add_track segment {i} must declare exactly one elementary stream"
        );
        assert!(
            is_avc(&media.tracks[0].spec.config),
            "before-add_track segment {i}'s only track must be the video track"
        );
    }

    // ── Assertions: segments after add_track carry both tracks in the PMT,
    //    and audio genuinely has PES data by the end of the stream ─────────
    let mut audio_samples_seen = 0usize;
    let mut video_samples_seen_after = 0usize;
    for (i, s) in after_segments.iter().enumerate() {
        let media = demux_segment(&s.bytes);
        assert!(
            media.tracks.iter().any(|t| is_avc(&t.spec.config)),
            "after-add_track segment {i} must still carry the video track"
        );
        let audio_track = media.tracks.iter().find(|t| is_aac(&t.spec.config));
        assert!(
            audio_track.is_some(),
            "after-add_track segment {i} must declare the audio track in its PMT"
        );
        audio_samples_seen += audio_track.map(|t| t.samples.len()).unwrap_or(0);
        video_samples_seen_after += media
            .tracks
            .iter()
            .find(|t| is_avc(&t.spec.config))
            .map(|t| t.samples.len())
            .unwrap_or(0);
    }
    assert_eq!(
        audio_samples_seen,
        audio_samples.len(),
        "every pushed audio sample must land in some after-add_track segment's PES \
         (no sample lost)"
    );
    // `split_idx` itself was already pushed in phase 1 (it's the sample that
    // *triggered* the cut, so it opens the new pending segment rather than
    // flushing into `before_segments`) — so the after-segments carry that one
    // plus every sample pushed in phase 3, i.e. every sample from `split_idx`
    // to the end.
    assert_eq!(
        video_samples_seen_after,
        video_samples.len() - split_idx,
        "every remaining video sample (including the cut-triggering one carried \
         over from phase 1) must be accounted for after add_track"
    );
}
