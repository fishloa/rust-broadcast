//! `SegmentCutPolicy` gate (issue #1425, Apple HLS Authoring Spec 7.4: every
//! segment starts on an IDR). Real captures only: `france2.ts` (IDR and
//! non-IDR sync samples), `gulli-opengop.ts` (open-GOP, zero IDR), the HEVC
//! and H.264 conformance fixtures. Streams are lengthened by looping the real
//! samples with rebased timestamps; HEVC NAL types are patched in real sample
//! bytes only where a fixture cannot supply the type (CRA, IDR_N_LP/W_RADL).

use crate::ts_hls_stage;
use broadcast_bitstream::{NalCodec, access_unit_is_idr};
use test_harness::collect::Collected as Media;
use transmux::mux_stage::SegmentOut;
use transmux::{CodecConfig, Sample, SegmentCutPolicy, StreamingTsHlsSegmenter, TrackSpec};

const TARGET_SECS: u32 = 1;
const LOOP_SECONDS: i64 = 12;
const LENGTH_PREFIX: usize = 4;
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;
const HEVC_IDR_W_RADL: u8 = 19;
const HEVC_IDR_N_LP: u8 = 20;
const HEVC_CRA: u8 = 21;
const HEVC_BLA_W_LP: u8 = 16;
const HEVC_BLA_W_RADL: u8 = 17;
const HEVC_BLA_N_LP: u8 = 18;
const AVC_NAL_TYPE_MASK: u8 = 0x1F;
const AVC_NAL_SEI: u8 = 6;
const AVC_NAL_SPS: u8 = 7;
const AVC_NAL_AUD: u8 = 9;

struct Video {
    spec: TrackSpec,
    codec: NalCodec,
    samples: Vec<Sample>,
}

fn video(rel: &str) -> Option<Video> {
    video_at(&test_fixtures::path(rel))
}

fn video_at(path: &std::path::Path) -> Option<Video> {
    let bytes = std::fs::read(path).ok()?;
    let ir: Media = test_harness::collect::media_from_ts(&bytes).expect("demux fixture");
    ir.tracks.into_iter().find_map(|t| {
        let codec = match t.spec.config {
            CodecConfig::Avc { .. } => NalCodec::Avc,
            CodecConfig::Hevc { .. } => NalCodec::Hevc,
            _ => return None,
        };
        Some(Video {
            spec: t.spec,
            codec,
            samples: t.samples,
        })
    })
}

fn require_video(rel: &str) -> Video {
    video(rel).unwrap_or_else(|| panic!("public fixture {rel} must exist and carry AVC/HEVC video"))
}

fn span_ticks(samples: &[Sample]) -> i64 {
    let first = samples
        .first()
        .and_then(|s| s.timing.and_then(|t| t.dts()))
        .expect("dts");
    let last = samples.last().expect("sample");
    last.timing.and_then(|t| t.dts()).expect("dts")
        + i64::from(last.timing.and_then(|t| t.duration()).expect("duration"))
        - first
}

fn looped(samples: &[Sample], reps: usize) -> Vec<Sample> {
    let span = span_ticks(samples);
    let mut out = Vec::with_capacity(samples.len() * reps);
    for k in 0..reps as i64 {
        for s in samples {
            out.push(
                Sample::new(
                    s.data.to_vec(),
                    s.timing.and_then(|t| t.dts()).map(|d| d + span * k),
                    s.timing.and_then(|t| t.pts()).map(|p| p + span * k),
                    s.timing.and_then(|t| t.duration()),
                    s.flags.is_sync,
                )
                .expect("valid timing"),
            );
        }
    }
    out
}

fn looped_to_seconds(v: &Video) -> Vec<Sample> {
    let span = span_ticks(&v.samples);
    let want = LOOP_SECONDS * i64::from(v.spec.timescale.get());
    let reps = (want / span).max(1) as usize + 1;
    looped(&v.samples, reps)
}

struct Run {
    cuts: Vec<usize>,
    segments: Vec<SegmentOut>,
    dropped: u64,
}

fn run_with(spec: &TrackSpec, samples: &[Sample], seg: StreamingTsHlsSegmenter) -> Run {
    let mut seg = seg;
    let mut cuts = Vec::new();
    let mut segments = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        ts_hls_stage::feed(&mut seg, spec.track_id, s.clone());
        let ready = ts_hls_stage::drain(&mut seg);
        if !ready.is_empty() {
            cuts.push(i);
            segments.extend(ready);
        }
    }
    let dropped = seg.dropped_leading_aus();
    ts_hls_stage::finish(&mut seg);
    segments.extend(ts_hls_stage::drain(&mut seg));
    Run {
        cuts,
        segments,
        dropped,
    }
}

fn run(spec: &TrackSpec, samples: &[Sample], policy: SegmentCutPolicy) -> Run {
    let seg = StreamingTsHlsSegmenter::new(vec![spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(policy);
    run_with(spec, samples, seg)
}

/// Verbatim copy of the cut predicate as it was before `SegmentCutPolicy`
/// existed (`ts_hls::is_cut_point`), kept here as the independent oracle.
fn old_is_cut_point(
    has_pending: bool,
    is_sync: bool,
    buffered_ticks: u64,
    target_ticks: u64,
) -> bool {
    has_pending && is_sync && buffered_ticks >= target_ticks
}

fn oracle_cuts(samples: &[Sample], timescale: u32) -> Vec<usize> {
    let target_ticks = u64::from(TARGET_SECS) * u64::from(timescale);
    let mut buffered = 0u64;
    let mut prev_dts: Option<i64> = None;
    let mut cuts = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        if old_is_cut_point(i > 0, s.flags.is_sync, buffered, target_ticks) {
            cuts.push(i);
            buffered = 0;
        }
        let tick = match (
            s.timing.and_then(|t| t.duration()),
            s.timing.and_then(|t| t.dts()),
            prev_dts,
        ) {
            (Some(d), _, _) if d > 0 => u64::from(d),
            (_, Some(now), Some(prev)) => (now - prev).max(0) as u64,
            _ => 0,
        };
        buffered += tick;
        prev_dts = s.timing.and_then(|t| t.dts());
    }
    cuts
}

fn first_video_sample_is_idr(segment: &SegmentOut, codec: NalCodec) -> bool {
    let ir = test_harness::collect::media_from_ts(&segment.bytes).expect("demux segment");
    let track = ir
        .tracks
        .iter()
        .find(|t| {
            matches!(
                t.spec.config,
                CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
            )
        })
        .expect("segment carries a video track");
    access_unit_is_idr(codec, &track.samples[0].data)
}

fn nal_ranges(data: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + LENGTH_PREFIX <= data.len() {
        let len = u32::from_be_bytes(data[off..off + LENGTH_PREFIX].try_into().unwrap()) as usize;
        let start = off + LENGTH_PREFIX;
        out.push((start, start + len));
        off = start + len;
    }
    out
}

fn hevc_patch_irap(sample: &Sample, nal_type: u8) -> Sample {
    let mut data = sample.data.to_vec();
    let mut patched = false;
    for (start, _) in nal_ranges(&sample.data) {
        let t = (data[start] >> HEVC_NAL_TYPE_SHIFT) & HEVC_NAL_TYPE_MASK;
        if (HEVC_BLA_W_LP..=HEVC_CRA).contains(&t) {
            data[start] = (data[start] & !(HEVC_NAL_TYPE_MASK << HEVC_NAL_TYPE_SHIFT))
                | (nal_type << HEVC_NAL_TYPE_SHIFT);
            patched = true;
        }
    }
    assert!(patched, "fixture sample must carry an IRAP NAL to patch");
    Sample::new(
        data,
        sample.timing.and_then(|t| t.dts()),
        sample.timing.and_then(|t| t.pts()),
        sample.timing.and_then(|t| t.duration()),
        sample.flags.is_sync,
    )
    .expect("valid timing")
}

fn hevc_stream_with_irap(nal_type: u8) -> (Video, Vec<Sample>) {
    let v = require_video("ts/hevc/main.ts");
    let patched: Vec<Sample> = v
        .samples
        .iter()
        .map(|s| {
            if s.flags.is_sync {
                hevc_patch_irap(s, nal_type)
            } else {
                s.clone()
            }
        })
        .collect();
    let stream = looped(&patched, 8);
    (v, stream)
}

const PUBLIC_FIXTURES: &[&str] = &[
    "ts/h264-aac.ts",
    "ts/h264-aac-40s.ts",
    "ts/gulli-opengop.ts",
    "ts/france2.ts",
    "ts/h264/baseline.ts",
    "ts/h264/high.ts",
    "ts/h264/interlaced.ts",
    "ts/h264/main.ts",
    "ts/hevc/main.ts",
    "ts/hevc/main10.ts",
];

#[test]
fn idr_only_cuts_only_on_idr() {
    let v = require_video("ts/france2.ts");
    let stream = looped_to_seconds(&v);

    let strict = run(&v.spec, &stream, SegmentCutPolicy::IdrOnly);
    assert!(
        strict.cuts.len() >= 3,
        "france2 looped must produce several IdrOnly cuts, got {:?}",
        strict.cuts
    );
    for &i in &strict.cuts {
        assert!(
            access_unit_is_idr(v.codec, &stream[i].data),
            "IdrOnly cut at sample {i} which is not an IDR"
        );
    }
    for seg in &strict.segments {
        assert!(
            first_video_sample_is_idr(seg, v.codec),
            "segment {} does not start on an IDR",
            seg.summary.sequence
        );
    }

    let loose = run(&v.spec, &stream, SegmentCutPolicy::AnyRap);
    assert!(
        loose
            .cuts
            .iter()
            .any(|&i| !access_unit_is_idr(v.codec, &stream[i].data)),
        "contrast: AnyRap must cut on a non-IDR sync sample of this stream"
    );
}

#[test]
fn any_rap_cut_points_equal_the_pre_change_predicate() {
    let mut fixtures: Vec<(String, Video)> = PUBLIC_FIXTURES
        .iter()
        .map(|rel| ((*rel).to_owned(), require_video(rel)))
        .collect();
    for rel in [
        "ts/france-tnt-uhf32-video-head-6-mb.ts",
        "ts/hotbird-mhp-video-head-6-mb.ts",
    ] {
        if let Some(v) = test_fixtures::private(rel).and_then(|p| video_at(&p)) {
            fixtures.push((format!("private:{rel}"), v));
        }
    }
    let mut total = 0usize;
    let mut non_idr_cuts = 0usize;
    for (rel, v) in &fixtures {
        let stream = looped_to_seconds(v);
        let want = oracle_cuts(&stream, v.spec.timescale.get());
        assert!(
            !want.is_empty(),
            "{rel}: oracle found no cut; fixture too weak"
        );
        let got = run(&v.spec, &stream, SegmentCutPolicy::AnyRap);
        assert_eq!(
            got.cuts, want,
            "{rel}: AnyRap cut points differ from the old predicate"
        );
        assert_eq!(got.dropped, 0, "{rel}: AnyRap must never drop leading AUs");
        total += want.len();
        non_idr_cuts += want
            .iter()
            .filter(|&&i| !access_unit_is_idr(v.codec, &stream[i].data))
            .count();
    }
    assert!(total >= PUBLIC_FIXTURES.len());
    assert!(non_idr_cuts > 0, "no fixture exercised a non-IDR cut");
}

#[test]
fn idr_only_first_segment_starts_at_first_idr_dropping_leading_aus() {
    let v = require_video("ts/france2.ts");
    let first_non_idr_sync = 20;
    let first_idr_after = 48;
    assert!(!access_unit_is_idr(
        v.codec,
        &v.samples[first_non_idr_sync].data
    ));
    assert!(v.samples[first_non_idr_sync].flags.is_sync);
    assert!(access_unit_is_idr(
        v.codec,
        &v.samples[first_idr_after].data
    ));

    let stream = looped(&v.samples[first_non_idr_sync..], 4);
    let strict = run(&v.spec, &stream, SegmentCutPolicy::IdrOnly);
    assert!(!strict.segments.is_empty());
    assert!(
        first_video_sample_is_idr(&strict.segments[0], v.codec),
        "first segment must open on the first IDR"
    );
    let first_seg_samples = test_harness::collect::media_from_ts(&strict.segments[0].bytes)
        .expect("demux")
        .tracks
        .iter()
        .map(|t| t.samples.len())
        .max()
        .expect("track");
    assert!(first_seg_samples > 0);

    let loose = run(&v.spec, &stream, SegmentCutPolicy::AnyRap);
    assert!(
        !first_video_sample_is_idr(&loose.segments[0], v.codec),
        "contrast: AnyRap opens its first segment on the leading non-IDR sample"
    );

    let emitted: usize = strict
        .segments
        .iter()
        .map(|s| {
            test_harness::collect::media_from_ts(&s.bytes)
                .expect("demux")
                .tracks
                .iter()
                .map(|t| t.samples.len())
                .sum::<usize>()
        })
        .sum();
    assert_eq!(
        emitted + strict.dropped as usize,
        stream.len(),
        "every access unit is either emitted or counted as dropped"
    );
}

#[test]
fn leading_drops_counted() {
    let v = require_video("ts/france2.ts");
    let leading = 48 - 20;
    let stream = looped(&v.samples[20..], 3);

    let strict = run(&v.spec, &stream, SegmentCutPolicy::IdrOnly);
    assert_eq!(strict.dropped, leading as u64);

    let loose = run(&v.spec, &stream, SegmentCutPolicy::AnyRap);
    assert_eq!(loose.dropped, 0);

    let g = require_video("ts/gulli-opengop.ts");
    let open_gop = looped(&g.samples, 4);
    let none = run(&g.spec, &open_gop, SegmentCutPolicy::IdrOnly);
    assert_eq!(
        none.dropped,
        open_gop.len() as u64,
        "zero IDR: every AU dropped"
    );
    assert!(
        none.segments.is_empty(),
        "no IDR, no segment may be emitted"
    );
}

#[test]
fn hevc_idr_w_radl_and_idr_n_lp_are_cuts_cra_is_not() {
    for t in [HEVC_IDR_W_RADL, HEVC_IDR_N_LP] {
        let (v, stream) = hevc_stream_with_irap(t);
        let strict = run(&v.spec, &stream, SegmentCutPolicy::IdrOnly);
        assert!(
            !strict.cuts.is_empty(),
            "HEVC NAL type {t} must be a cut point"
        );
        assert_eq!(strict.dropped, 0, "stream starts on an IDR of type {t}");
        for &i in &strict.cuts {
            assert!(access_unit_is_idr(NalCodec::Hevc, &stream[i].data));
        }
    }
    for t in [HEVC_CRA, HEVC_BLA_W_LP, HEVC_BLA_W_RADL, HEVC_BLA_N_LP] {
        let (v, stream) = hevc_stream_with_irap(t);
        let strict = run(&v.spec, &stream, SegmentCutPolicy::IdrOnly);
        assert!(
            strict.cuts.is_empty(),
            "HEVC NAL type {t} must not be a cut point"
        );
        assert!(strict.segments.is_empty());
        assert_eq!(strict.dropped, stream.len() as u64);
        let loose = run(&v.spec, &stream, SegmentCutPolicy::AnyRap);
        assert!(
            !loose.cuts.is_empty(),
            "contrast: AnyRap cuts on HEVC NAL type {t}"
        );
    }
}

#[test]
fn recovery_point_sei_is_not_a_cut_under_idr_only() {
    let g = require_video("ts/gulli-opengop.ts");
    let sei_rap = g
        .samples
        .iter()
        .filter(|s| s.flags.is_sync && !access_unit_is_idr(g.codec, &s.data))
        .filter(|s| {
            nal_ranges(&s.data)
                .iter()
                .any(|&(a, _)| s.data[a] & AVC_NAL_TYPE_MASK == AVC_NAL_SEI)
        })
        .count();
    assert!(sei_rap >= 5, "fixture carries recovery-point RAPs");
    let stream = looped_to_seconds(&g);

    let strict = run(&g.spec, &stream, SegmentCutPolicy::IdrOnly);
    assert!(strict.cuts.is_empty(), "recovery-point RAPs are not IDRs");
    assert!(strict.segments.is_empty());

    let loose = run(&g.spec, &stream, SegmentCutPolicy::AnyRap);
    assert!(
        !loose.cuts.is_empty(),
        "contrast: AnyRap cuts on recovery points"
    );
}

fn with_sps_prefix(idr: &Sample, p_frame: &Sample, dts: i64) -> Sample {
    let sps = nal_ranges(&idr.data)
        .into_iter()
        .find(|&(a, _)| idr.data[a] & AVC_NAL_TYPE_MASK == AVC_NAL_SPS)
        .expect("IDR sample carries an SPS");
    let mut data = Vec::new();
    data.extend_from_slice(&((sps.1 - sps.0) as u32).to_be_bytes());
    data.extend_from_slice(&idr.data[sps.0..sps.1]);
    for (a, b) in nal_ranges(&p_frame.data) {
        if p_frame.data[a] & AVC_NAL_TYPE_MASK != AVC_NAL_AUD {
            data.extend_from_slice(&((b - a) as u32).to_be_bytes());
            data.extend_from_slice(&p_frame.data[a..b]);
        }
    }
    Sample::new(
        data,
        Some(dts),
        Some(dts),
        p_frame.timing.and_then(|t| t.duration()),
        true,
    )
    .expect("valid timing")
}

#[test]
fn sps_only_picture_is_not_a_cut_under_idr_only() {
    let v = require_video("ts/france2.ts");
    let idr = &v.samples[0];
    let p_frames: Vec<&Sample> = v.samples[1..20].iter().collect();
    assert!(
        p_frames
            .iter()
            .all(|s| !access_unit_is_idr(v.codec, &s.data))
    );

    let mut stream = vec![idr.clone()];
    let step = i64::from(
        p_frames[0]
            .timing
            .and_then(|t| t.duration())
            .expect("duration"),
    );
    let mut dts = idr.timing.and_then(|t| t.dts()).expect("dts") + step;
    for k in 0..(LOOP_SECONDS * 25) as usize {
        let p = p_frames[k % p_frames.len()];
        stream.push(with_sps_prefix(idr, p, dts));
        dts += step;
    }

    let strict = run(&v.spec, &stream, SegmentCutPolicy::IdrOnly);
    assert!(
        strict.cuts.is_empty(),
        "SPS-led non-IDR pictures are not cut points"
    );
    assert_eq!(strict.dropped, 0, "stream opens on a real IDR");

    let loose = run(&v.spec, &stream, SegmentCutPolicy::AnyRap);
    assert!(
        !loose.cuts.is_empty(),
        "contrast: AnyRap cuts on the SPS-flagged samples"
    );
}

#[test]
fn gap_restarts_at_next_idr() {
    let v = require_video("ts/france2.ts");
    let stream = looped(&v.samples, 6);
    let per_rep = v.samples.len();
    let gap_at = 2 * per_rep + 20;
    assert!(stream[gap_at].flags.is_sync);
    assert!(!access_unit_is_idr(v.codec, &stream[gap_at].data));

    let mut seg = StreamingTsHlsSegmenter::new(vec![v.spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::IdrOnly);
    let mut before_gap = Vec::new();
    let mut after_gap = Vec::new();
    for (i, s) in stream.iter().enumerate() {
        if i == gap_at {
            before_gap.extend(ts_hls_stage::drain(&mut seg));
            assert_eq!(seg.dropped_leading_aus(), 0);
            seg.mark_discontinuity();
        }
        ts_hls_stage::feed(&mut seg, v.spec.track_id, s.clone());
        if i < gap_at {
            before_gap.extend(ts_hls_stage::drain(&mut seg));
        } else {
            after_gap.extend(ts_hls_stage::drain(&mut seg));
        }
    }
    ts_hls_stage::finish(&mut seg);
    after_gap.extend(ts_hls_stage::drain(&mut seg));

    let next_idr = (gap_at..stream.len())
        .find(|&i| access_unit_is_idr(v.codec, &stream[i].data))
        .expect("an IDR follows the gap");
    assert_eq!(
        seg.dropped_leading_aus(),
        (next_idr - gap_at) as u64,
        "exactly the AUs between the gap and the next IDR are dropped"
    );
    assert!(
        before_gap.iter().all(|s| !s.summary.discontinuous),
        "segments closed before the gap are continuous"
    );
    assert!(
        !after_gap[0].summary.discontinuous,
        "the pre-gap tail closed at the resync IDR is continuous"
    );
    assert!(
        after_gap[1].summary.discontinuous,
        "the first segment opened after the gap carries the discontinuity"
    );
    assert!(first_video_sample_is_idr(&after_gap[1], v.codec));
    assert!(after_gap.iter().skip(2).all(|s| !s.summary.discontinuous));
    assert!(
        seg.playlist()
            .expect("playlist")
            .contains("#EXT-X-DISCONTINUITY")
    );
}

#[test]
fn default_policy_is_any_rap() {
    assert_eq!(SegmentCutPolicy::default(), SegmentCutPolicy::AnyRap);
    let v = require_video("ts/gulli-opengop.ts");
    let seg = StreamingTsHlsSegmenter::new(vec![v.spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter");
    assert_eq!(seg.cut_policy(), SegmentCutPolicy::AnyRap);
    let stream = looped_to_seconds(&v);
    let want = oracle_cuts(&stream, v.spec.timescale.get());
    assert!(!want.is_empty());
    let got = run_with(&v.spec, &stream, seg);
    assert_eq!(
        got.cuts, want,
        "an unconfigured segmenter keeps the old cut rule"
    );
    assert_eq!(got.dropped, 0);
}

#[test]
fn name_and_display_labels() {
    assert_eq!(SegmentCutPolicy::AnyRap.name(), "any_rap");
    assert_eq!(SegmentCutPolicy::IdrOnly.name(), "idr_only");
    assert_eq!(SegmentCutPolicy::AnyRap.to_string(), "any_rap");
    assert_eq!(SegmentCutPolicy::IdrOnly.to_string(), "idr_only");
}

// ---------------------------------------------------------------------------
// audit 22 — IdrWait::tripped is a real state transition, not a constant
// ---------------------------------------------------------------------------

#[test]
fn idr_wait_trips_at_its_bound_not_before() {
    use core::time::Duration;
    use transmux::IdrWait;

    let mut wait = IdrWait::new(Duration::from_secs(1));
    assert!(!wait.tripped(), "a fresh wait has not tripped");
    let sample = transmux::Sample::new(vec![0u8; 4], Some(0), Some(0), Some(1000), true)
        .expect("valid timing");
    let rate = broadcast_core::timescale!(1000);
    assert!(
        wait.observe_dropped(&sample, rate),
        "1000 ticks at 1 kHz reaches the 1 s bound"
    );
    assert!(wait.tripped(), "the bound is now reached");
    assert!(
        !wait.observe_dropped(&sample, rate),
        "the trip is reported exactly once"
    );
}

#[test]
fn with_cut_policy_returns_the_configured_policy() {
    let v = require_video("ts/france2.ts");
    let seg = StreamingTsHlsSegmenter::new(vec![v.spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::IdrOnly);
    assert_eq!(seg.cut_policy(), SegmentCutPolicy::IdrOnly);
}

#[test]
fn fall_back_to_any_rap_opens_the_next_sync_sample() {
    // gulli-opengop carries RAPs but zero IDRs, so without the fall-back the
    // IdrOnly wait never opens a segment.
    let g = require_video("ts/gulli-opengop.ts");
    let stream = looped_to_seconds(&g);
    let mut seg = StreamingTsHlsSegmenter::new(vec![g.spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::IdrOnly);

    let mut idx = 0;
    while idx < stream.len() && seg.dropped_leading_aus() == 0 {
        ts_hls_stage::feed(&mut seg, g.spec.track_id, stream[idx].clone());
        idx += 1;
    }
    assert!(
        seg.dropped_leading_aus() > 0,
        "the opengop stream must start without an IDR"
    );
    seg.fall_back_to_any_rap();
    assert!(seg.idr_fallback_active(), "fall-back must be reported");

    let mut cuts = ts_hls_stage::drain(&mut seg);
    for sample in &stream[idx..] {
        ts_hls_stage::feed(&mut seg, g.spec.track_id, sample.clone());
        cuts.extend(ts_hls_stage::drain(&mut seg));
    }
    ts_hls_stage::finish(&mut seg);
    cuts.extend(ts_hls_stage::drain(&mut seg));
    assert!(
        !cuts.is_empty(),
        "a sync sample must open a segment once the fall-back accepts any RAP"
    );
}

#[test]
fn with_cut_policy_does_not_rearm_after_samples_are_buffered() {
    let v = require_video("ts/france2.ts");
    // A non-sync sample only: nothing buffered can be cut, but the buffer is
    // no longer untouched, so switching to IdrOnly must not arm the wait.
    let non_sync = v
        .samples
        .iter()
        .find(|s| !s.flags.is_sync)
        .expect("france2 carries a non-sync sample")
        .clone();

    let mut seg = StreamingTsHlsSegmenter::new(vec![v.spec.clone()], u32::MAX, usize::MAX)
        .expect("segmenter");
    ts_hls_stage::feed(&mut seg, v.spec.track_id, non_sync);
    let mut seg = seg.with_cut_policy(SegmentCutPolicy::IdrOnly);
    assert_eq!(seg.cut_policy(), SegmentCutPolicy::IdrOnly);

    // Sample 20 is a sync sample that is not an IDR; it must not be dropped.
    ts_hls_stage::feed(&mut seg, v.spec.track_id, v.samples[20].clone());
    assert_eq!(
        seg.dropped_leading_aus(),
        0,
        "re-arming the IDR wait after buffering would drop this sample"
    );
}

fn vvc_spec() -> Option<TrackSpec> {
    let bytes = test_fixtures::read("mp4/frag/vvc.frag.mp4");
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .expect("demux vvc fixture");
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Vvc { .. }))
        .map(|t| t.spec)
}

fn vvc_au(nal_type: u8) -> Vec<u8> {
    let mut au = 2u32.to_be_bytes().to_vec();
    au.push(0x00);
    au.push(nal_type << 3);
    au
}

#[test]
fn vvc_idr_opens_the_first_segment_under_idr_only() {
    let Some(spec) = vvc_spec() else {
        eprintln!("SKIP: mp4/frag/vvc.frag.mp4 missing");
        return;
    };
    let id = spec.track_id;
    let cra = vvc_au(9);
    let idr = vvc_au(7);
    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::IdrOnly);

    ts_hls_stage::feed(
        &mut seg,
        id,
        Sample::new(cra.clone(), None, None, Some(90_000), true).expect("valid timing"),
    );
    assert_eq!(
        seg.dropped_leading_aus(),
        1,
        "a VVC CRA is a RAP but not an IDR"
    );

    ts_hls_stage::feed(
        &mut seg,
        id,
        Sample::new(idr.clone(), None, None, Some(90_000), true).expect("valid timing"),
    );
    assert_eq!(
        seg.dropped_leading_aus(),
        1,
        "the IDR opens the stream; the CRA stays dropped"
    );
    // Already open: a following non-IDR sync sample is buffered, not dropped.
    ts_hls_stage::feed(
        &mut seg,
        id,
        Sample::new(cra, None, None, Some(90_000), true).expect("valid timing"),
    );
    assert_eq!(
        seg.dropped_leading_aus(),
        1,
        "once the IDR opens the stream, no further access unit is dropped"
    );
}

#[test]
fn a_marked_discontinuity_before_any_cut_does_not_flag_the_first_segment() {
    let v = require_video("ts/france2.ts");
    let stream = looped(&v.samples, 4);
    let mut seg = StreamingTsHlsSegmenter::new(vec![v.spec.clone()], TARGET_SECS, usize::MAX)
        .expect("segmenter")
        .with_cut_policy(SegmentCutPolicy::IdrOnly);
    seg.mark_discontinuity();

    let mut cuts = Vec::new();
    let mut cut_idx = 0;
    for (i, sample) in stream.iter().enumerate() {
        ts_hls_stage::feed(&mut seg, v.spec.track_id, sample.clone());
        cuts.extend(ts_hls_stage::drain(&mut seg));
        if !cuts.is_empty() {
            cut_idx = i;
            break;
        }
    }
    assert!(!cuts.is_empty());
    assert!(
        !cuts[0].summary.discontinuous,
        "a discontinuity signalled before any output is not a timeline break at the start"
    );
    assert!(
        !seg.playlist()
            .expect("playlist")
            .contains("#EXT-X-DISCONTINUITY"),
        "the first segment must not render the tag"
    );

    seg.mark_discontinuity();
    let mut second = Vec::new();
    for sample in &stream[cut_idx + 1..] {
        ts_hls_stage::feed(&mut seg, v.spec.track_id, sample.clone());
        second.extend(ts_hls_stage::drain(&mut seg));
        if second.len() >= 2 {
            break;
        }
    }
    assert!(
        second.len() >= 2,
        "a pre-gap tail and then a fresh segment must be cut"
    );
    assert!(
        !second[0].summary.discontinuous,
        "the pre-gap tail closed at the resync IDR is continuous"
    );
    assert!(
        second[1].summary.discontinuous,
        "after one segment, a marked discontinuity flags the next"
    );
}
