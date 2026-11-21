//! A streaming TS-HLS segment's audio must span the same media as its video
//! (round-9 F2): Apple's `mediastreamvalidator` flags "Playlist vs segment
//! duration mismatch" when audio, which a live multiplex delivers after the
//! video it accompanies, is cut short at the boundary. Samples are pushed in
//! the order `StreamingTsDemux` emits them, exactly as a live ingest does.

use std::collections::HashMap;

use broadcast_common::Unpackage;
use broadcast_common::timescale::Timescale;
use transmux::media::Media;
use transmux::pipeline::{CodecConfig, DataCarriage};
use transmux::{
    DemuxEvent, Sample, SegmentCutPolicy, StreamingTsDemux, StreamingTsHlsSegmenter, TrackId,
    TrackSpec, TsDemux,
};

const TARGET_SECS: u32 = 1;
const AUDIO_TOLERANCE_SECS: f64 = 0.07;
const MIN_SEGMENTS: usize = 3;
const CHUNK: usize = 7 * 188;

struct Streamed {
    specs: Vec<TrackSpec>,
    events: Vec<(TrackId, Sample)>,
}

fn stream(bytes: &[u8]) -> Streamed {
    let mut demux = StreamingTsDemux::new();
    let mut specs: Vec<TrackSpec> = Vec::new();
    let mut events = Vec::new();
    let mut drain = |demux: &mut StreamingTsDemux| {
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) => specs.push(spec),
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => events.push((track_id, sample)),
                _ => {}
            }
        }
    };
    for chunk in bytes.chunks(CHUNK) {
        demux.feed(chunk).expect("valid timing");
        drain(&mut demux);
    }
    demux.finish().expect("valid timing");
    drain(&mut demux);
    Streamed { specs, events }
}

fn segments(streamed: &Streamed) -> Vec<Vec<u8>> {
    let mut seg = StreamingTsHlsSegmenter::new(streamed.specs.clone(), TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::AnyRap);
    let known: Vec<TrackId> = streamed.specs.iter().map(|s| s.track_id).collect();
    let mut out = Vec::new();
    for (track_id, sample) in &streamed.events {
        if known.contains(track_id) {
            seg.push(*track_id, sample.clone()).expect("push");
            out.extend(seg.take_ready().into_iter().map(|s| s.bytes));
        }
    }
    out
}

fn spans(segment: &[u8]) -> (f64, Vec<f64>) {
    let ir: Media = TsDemux::new().unpackage(segment).expect("demux segment");
    let mut video = 0.0;
    let mut audio = Vec::new();
    for t in &ir.tracks {
        let is_audio = t.spec.config.audio_sample_rate().is_some();
        if !is_audio && !t.spec.config.is_video() {
            continue;
        }
        let first = t.samples[0].timing.and_then(|t| t.dts()).expect("dts");
        let last = t.samples.last().expect("sample");
        let span = (last.timing.and_then(|t| t.dts()).expect("dts")
            + i64::from(last.timing.and_then(|t| t.duration()).expect("duration"))
            - first) as f64
            / f64::from(t.spec.timescale.get());
        match t.spec.config {
            _ if is_audio => audio.push(span),
            _ => video = span,
        }
    }
    (video, audio)
}

fn check(rel: &str, bytes: &[u8]) -> usize {
    let streamed = stream(bytes);
    let cut = segments(&streamed);
    assert!(
        cut.len() >= MIN_SEGMENTS,
        "{rel}: only {} segments",
        cut.len()
    );
    let mut checked = 0;
    let mut worst: HashMap<&str, f64> = HashMap::new();
    for (i, segment) in cut.iter().enumerate().skip(1) {
        let (video, audio) = spans(segment);
        for a in audio {
            let off = (a - video).abs();
            let slot = worst.entry("worst").or_default();
            *slot = slot.max(off);
            assert!(
                off <= AUDIO_TOLERANCE_SECS,
                "{rel}: segment {i} audio spans {a:.3}s against {video:.3}s of video"
            );
            checked += 1;
        }
    }
    checked
}

#[test]
fn live_order_audio_spans_match_video_on_real_captures() {
    let mut total = 0;
    for rel in ["ts/h264-aac-40s.ts", "ts/gulli-opengop.ts"] {
        let bytes = std::fs::read(test_fixtures::path(rel)).expect("fixture");
        total += check(rel, &bytes);
    }
    assert!(total > 0);
}

#[test]
fn zenith_capture_audio_spans_match_video() {
    for rel in [
        "http/live-channels/tf1-live.ts",
        "ts/live-channels/rai-gulp-8-mb.ts",
        "ts/live-channels/rai-news-24-8-mb.ts",
    ] {
        let Some(path) = test_fixtures::require(rel, "ts_hls_audio_catch_up") else {
            continue;
        };
        let bytes = std::fs::read(path).expect("fixture");
        assert!(check(rel, &bytes) > 0, "{rel}: no audio segment checked");
    }
}

// ---------------------------------------------------------------------------
// Synthetic deferred-cut / audio-lag boundaries (audit 23 finding 168)
// ---------------------------------------------------------------------------

fn video_sample(duration: u32, sync: bool) -> Sample {
    Sample::new(vec![0u8; 4], None, None, Some(duration), sync).expect("valid timing")
}

fn audio_sample(duration: u32) -> Sample {
    Sample::new(vec![0u8; 4], None, None, Some(duration), true).expect("valid timing")
}

fn catch_up_segmenter() -> (StreamingTsHlsSegmenter, TrackId, TrackId) {
    let video = crate::hub_fixtures::video_track();
    let audio = crate::hub_fixtures::audio_track();
    let (vid, aud) = (video.track_id, audio.track_id);
    let seg = StreamingTsHlsSegmenter::new(vec![video, audio], TARGET_SECS, usize::MAX)
        .expect("construct");
    (seg, vid, aud)
}

/// 0.1 s video frames at 90 kHz, keyframe every 10 (a 1 s GOP).
fn push_video_frames(seg: &mut StreamingTsHlsSegmenter, id: TrackId, frames: usize, start: usize) {
    for i in start..start + frames {
        seg.push(id, video_sample(9_000, i % 10 == 0))
            .expect("push video");
    }
}

#[test]
fn a_lagging_audio_track_defers_the_cut_to_the_boundary() {
    let (mut seg, vid, aud) = catch_up_segmenter();
    for _ in 0..42 {
        seg.push(aud, audio_sample(1024)).expect("push audio");
    }
    push_video_frames(&mut seg, vid, 11, 0);
    assert!(
        seg.take_ready().is_empty(),
        "audio 0.9 s behind the 1.0 s boundary must defer the cut"
    );

    for _ in 42..48 {
        seg.push(aud, audio_sample(1024)).expect("push audio");
    }
    let cuts = seg.take_ready();
    assert_eq!(cuts.len(), 1, "audio reaching 1.0 s must close the cut");
    let media = TsDemux::new()
        .unpackage(&cuts[0].bytes)
        .expect("demux segment");
    let audio = media
        .tracks
        .iter()
        .find(|t| t.spec.config.audio_sample_rate().is_some())
        .expect("segment carries audio");
    assert_eq!(
        audio.samples.len(),
        47,
        "the closed segment holds every audio sample before the 1.0 s boundary"
    );
}

#[test]
fn a_cut_is_waited_out_after_catch_up_max_secs_of_anchor_media() {
    let (mut seg, vid, aud) = catch_up_segmenter();
    for _ in 0..42 {
        seg.push(aud, audio_sample(1024)).expect("push audio");
    }
    push_video_frames(&mut seg, vid, 11, 0);
    assert!(seg.take_ready().is_empty(), "the cut defers first");

    for _ in 0..9 {
        seg.push(vid, video_sample(9_000, false))
            .expect("push video");
    }
    assert!(
        !seg.take_ready().is_empty(),
        "1 s of further anchor media must wait out the lagging audio"
    );
}

#[test]
fn a_track_that_has_delivered_no_media_never_defers_the_cut() {
    let video = crate::hub_fixtures::video_track();
    let audio = crate::hub_fixtures::audio_track();
    let vid = video.track_id;
    let data = TrackSpec::new(
        TrackId::new(3),
        Timescale::try_new(90_000).expect("non-zero"),
        CodecConfig::Data {
            stream_type: 0x86,
            descriptors: Vec::new(),
            carriage: DataCarriage::Sections,
        },
    );
    let mut seg = StreamingTsHlsSegmenter::new(vec![video, audio, data], TARGET_SECS, usize::MAX)
        .expect("construct");

    push_video_frames(&mut seg, vid, 11, 0);
    assert!(
        !seg.take_ready().is_empty(),
        "an audio track with no samples and a section track must not defer"
    );
}

#[test]
fn audio_already_past_the_boundary_cuts_immediately() {
    let (mut seg, vid, aud) = catch_up_segmenter();
    for _ in 0..50 {
        seg.push(aud, audio_sample(1024)).expect("push audio");
    }
    push_video_frames(&mut seg, vid, 11, 0);
    assert!(
        !seg.take_ready().is_empty(),
        "audio past the boundary must not defer the cut"
    );
}

#[test]
fn a_non_anchor_section_track_with_samples_never_defers_the_cut() {
    let audio = crate::hub_fixtures::audio_track();
    let aid = audio.track_id;
    let data_id = TrackId::new(3);
    let data = TrackSpec::new(
        data_id,
        Timescale::try_new(90_000).expect("non-zero"),
        CodecConfig::Data {
            stream_type: 0x86,
            descriptors: Vec::new(),
            carriage: DataCarriage::Sections,
        },
    );
    let mut seg = StreamingTsHlsSegmenter::new(vec![audio, data], TARGET_SECS, usize::MAX)
        .expect("construct");

    // A section sample has no timing and no duration; the track has therefore
    // "delivered media" but can never carry a decode time of its own.
    seg.push(
        data_id,
        Sample::new(vec![0xFC, 0x30, 0x11], None, None, None, true).expect("valid timing"),
    )
    .expect("push section");
    // 0.1 s audio frames: the cut-triggering frame adds less than the
    // CATCH_UP_MAX_SECS bound, so a false lag cannot be waited out on the
    // same push.
    for _ in 0..11 {
        seg.push(aid, audio_sample(4_800)).expect("push audio");
    }

    assert!(
        !seg.take_ready().is_empty(),
        "a non-audio track must never be treated as lagging audio"
    );
}
