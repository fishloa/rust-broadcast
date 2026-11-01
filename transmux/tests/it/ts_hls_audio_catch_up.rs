//! A streaming TS-HLS segment's audio must span the same media as its video
//! (round-9 F2): Apple's `mediastreamvalidator` flags "Playlist vs segment
//! duration mismatch" when audio, which a live multiplex delivers after the
//! video it accompanies, is cut short at the boundary. Samples are pushed in
//! the order `StreamingTsDemux` emits them, exactly as a live ingest does.

use std::collections::HashMap;

use broadcast_common::Unpackage;
use transmux::media::Media;
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
        demux.feed(chunk);
        drain(&mut demux);
    }
    demux.finish();
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
        let first = t.samples[0].dts.expect("dts");
        let last = t.samples.last().expect("sample");
        let span = (last.dts.expect("dts") + i64::from(last.duration.expect("duration")) - first)
            as f64
            / f64::from(t.spec.timescale);
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
    for rel in ["ts/h264_aac_40s.ts", "ts/gulli-opengop.ts"] {
        let bytes = std::fs::read(test_fixtures::path(rel)).expect("fixture");
        total += check(rel, &bytes);
    }
    assert!(total > 0);
}

#[test]
fn zenith_capture_audio_spans_match_video() {
    for rel in [
        "zenith-http/tf1-live.ts",
        "zenith-live/rai-gulp-8MB.ts",
        "zenith-live/rai-news-24-8MB.ts",
    ] {
        let Some(path) = test_fixtures::require(rel, "ts_hls_audio_catch_up") else {
            continue;
        };
        let bytes = std::fs::read(path).expect("fixture");
        assert!(check(rel, &bytes) > 0, "{rel}: no audio segment checked");
    }
}
