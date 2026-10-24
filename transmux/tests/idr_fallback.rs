//! Bounded fallback of `SegmentCutPolicy::IdrOnly` to any sync sample (round-9
//! F1). `gulli-opengop.ts` is real open-GOP H.264: sync samples, zero IDR
//! pictures. `france2.ts` carries real IDRs.

use std::time::Duration;

use broadcast_common::Unpackage;
use transmux::media::Media;
use transmux::{
    CodecConfig, NalCodec, Sample, SegmentCutPolicy, StreamingTsHlsSegmenter, TrackSpec, TsDemux,
    access_unit_is_idr,
};

const TARGET_SECS: u32 = 1;
const LOOP_SECONDS: i64 = 24;
const FALLBACK_BOUND: Duration = Duration::from_secs(2);
const GENEROUS_BOUND: Duration = Duration::from_secs(60);

struct Video {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

fn require_video(rel: &str) -> Video {
    let bytes = std::fs::read(test_fixtures::path(rel)).expect("public fixture");
    let ir: Media = TsDemux::new().unpackage(&bytes).expect("demux");
    ir.tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .map(|t| Video {
            spec: t.spec,
            samples: t.samples,
        })
        .expect("AVC track")
}

fn looped(v: &Video) -> Vec<Sample> {
    let first = v.samples[0].dts.expect("dts");
    let last = v.samples.last().expect("sample");
    let span = last.dts.expect("dts") + i64::from(last.duration.expect("duration")) - first;
    let want = LOOP_SECONDS * i64::from(v.spec.timescale);
    let reps = (want / span).max(1) + 1;
    let mut out = Vec::new();
    for k in 0..reps {
        for s in &v.samples {
            out.push(Sample::new(
                s.data.to_vec(),
                s.dts.map(|d| d + span * k),
                s.pts.map(|p| p + span * k),
                s.duration,
                s.flags.is_sync,
            ));
        }
    }
    out
}

struct Outcome {
    cuts: Vec<usize>,
    fell_back: bool,
}

fn run(v: &Video, stream: &[Sample], bound: Option<Duration>) -> Outcome {
    let mut seg = StreamingTsHlsSegmenter::new(vec![v.spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::IdrOnly);
    if let Some(bound) = bound {
        seg = seg.with_idr_fallback(bound);
    }
    let mut cuts = Vec::new();
    for (i, s) in stream.iter().enumerate() {
        seg.push(v.spec.track_id, s.clone()).expect("push");
        if !seg.take_ready().is_empty() {
            cuts.push(i);
        }
    }
    Outcome {
        cuts,
        fell_back: seg.idr_fallback_active(),
    }
}

#[test]
fn open_gop_stream_falls_back_after_the_bound() {
    let v = require_video("ts/gulli-opengop.ts");
    assert!(
        !v.samples
            .iter()
            .any(|s| access_unit_is_idr(NalCodec::Avc, &s.data)),
        "fixture must carry no IDR for this test to mean anything"
    );
    let stream = looped(&v);
    let out = run(&v, &stream, Some(FALLBACK_BOUND));
    assert!(out.fell_back);
    let first = *out.cuts.first().expect("a segment after the fallback");
    let first_dts = stream[0].dts.expect("dts");
    let cut_dts = stream[first].dts.expect("dts");
    let ticks_per_sec = i64::from(v.spec.timescale);
    assert!(
        cut_dts - first_dts >= 2 * ticks_per_sec,
        "first segment cut before the {FALLBACK_BOUND:?} bound"
    );
    assert!(
        cut_dts - first_dts <= (2 + 1 + i64::from(TARGET_SECS) + 4) * ticks_per_sec,
        "first segment cut far later than bound + one GOP + target"
    );
}

#[test]
fn strict_mode_on_an_open_gop_stream_never_cuts() {
    let v = require_video("ts/gulli-opengop.ts");
    let out = run(&v, &looped(&v), None);
    assert!(out.cuts.is_empty());
    assert!(!out.fell_back);
}

#[test]
fn idr_stream_never_falls_back() {
    let v = require_video("ts/france2.ts");
    let out = run(&v, &looped(&v), Some(GENEROUS_BOUND));
    assert!(!out.fell_back);
    assert!(!out.cuts.is_empty());
}
