//! Follower-cut mode of `LlHlsSegmenter` (#1428 slice 2a): an audio-only
//! segmenter cuts segments and parts where the video leader cut.
//!
//! Boundary rule under test: a sample belongs to the segment (and part) in which
//! it starts, so a sample whose decode time equals a boundary opens the next one.
//! Expected sample sets are derived here by exact cross-multiplication
//! (`d / follower_ts < boundary / leader_ts`), independent of the library's
//! ceiling rescale, and compared against the emitted `mdat` bytes.

#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::ll_hls::{CutPlan, LlHlsSegmenter, PartCut, PartInfo, SegmentInfo};
use transmux::media::{Media, Track};
use transmux::segmenter::MAX_PENDING_SAMPLES_PER_TRACK;
use transmux::{
    CodecConfig, Error, Sample, Severity, TsDemux, validate_init_segment, validate_media_segment,
};

const LEADER_TS: u32 = 90_000;
const AAC_FRAME_TICKS: i64 = 1024;
const AAC_RATE: u32 = 48_000;
const LEADER_TICKS_PER_8_FOLLOWER_TICKS: i64 = 15;
const PART_TARGET_MS: u32 = 334;
const SEGMENT_TARGET_SECS: f64 = 2.0;

fn leader_ticks(follower_ticks: i64) -> i64 {
    assert_eq!(follower_ticks % 8, 0, "not exactly representable at 90 kHz");
    follower_ticks / 8 * LEADER_TICKS_PER_8_FOLLOWER_TICKS
}

struct BoxView<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
}

fn boxes(bytes: &[u8]) -> Vec<BoxView<'_>> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && off + size <= bytes.len(), "malformed box");
        out.push(BoxView {
            kind: bytes[off + 4..off + 8].try_into().unwrap(),
            payload: &bytes[off + 8..off + size],
        });
        off += size;
    }
    assert_eq!(off, bytes.len(), "trailing bytes after last box");
    out
}

fn mdat_payload(bytes: &[u8]) -> Vec<u8> {
    let mdats: Vec<_> = boxes(bytes)
        .into_iter()
        .filter(|b| &b.kind == b"mdat")
        .collect();
    assert_eq!(mdats.len(), 1, "exactly one mdat per fragment");
    mdats[0].payload.to_vec()
}

fn trak_count(init: &[u8]) -> usize {
    let top = boxes(init);
    let moov = top.iter().find(|b| &b.kind == b"moov").expect("moov");
    boxes(moov.payload)
        .iter()
        .filter(|b| &b.kind == b"trak")
        .count()
}

fn assert_no_errors(what: &str, issues: &[transmux::ConformanceIssue]) {
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:?}");
}

fn concat(samples: &[&Sample]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.data.to_vec()).collect()
}

fn opens_next(dts: i64, track_ts: u32, start: i64, start_ts: u32) -> bool {
    i128::from(dts) * i128::from(start_ts) >= i128::from(start) * i128::from(track_ts)
}

/// Index of the plan whose segment the sample starts in: the count of later
/// plan starts at or before it. Samples before the first start stay in plan 0.
fn segment_of(dts: i64, track_ts: u32, plans: &[CutPlan]) -> usize {
    plans[1..]
        .iter()
        .take_while(|p| opens_next(dts, track_ts, p.segment_start, p.timescale))
        .count()
}

fn part_slot_of(dts: i64, track_ts: u32, plan: &CutPlan) -> usize {
    plan.parts
        .get(1..)
        .unwrap_or_default()
        .iter()
        .take_while(|p| {
            let ts = if p.timescale == 0 {
                plan.timescale
            } else {
                p.timescale
            };
            opens_next(dts, track_ts, p.start, ts)
        })
        .count()
}

fn run_follower(
    audio: &Track,
    movie_ts: u32,
    plans: &[CutPlan],
) -> (Vec<u8>, Vec<PartInfo>, Vec<SegmentInfo>) {
    let mut f = LlHlsSegmenter::with_part_target(
        vec![audio.spec.clone()],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap();
    let init = f.init_segment().unwrap();
    f.follow(plans[0].clone()).unwrap();
    for s in &audio.samples {
        f.push(audio.spec.track_id, s.clone()).unwrap();
    }
    for p in &plans[1..] {
        f.follow(p.clone()).unwrap();
    }
    f.flush().unwrap();
    (init, f.take_ready_parts(), f.take_ready_segments())
}

fn leader_plans(video: &Track, movie_ts: u32) -> Vec<CutPlan> {
    let mut l = LlHlsSegmenter::with_part_target(
        vec![video.spec.clone()],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap();
    l.record_cuts();
    for s in &video.samples {
        l.push(video.spec.track_id, s.clone()).unwrap();
    }
    l.flush().unwrap();
    let parts = l.take_ready_parts();
    let segs = l.take_ready_segments();
    let plans = l.take_ready_cut_plans();
    assert_eq!(plans.len(), segs.len());
    for (plan, seg) in plans.iter().zip(&segs) {
        assert_eq!(plan.segment_seq, seg.segment_seq);
        let indices: Vec<u32> = parts
            .iter()
            .filter(|p| p.segment_seq == seg.segment_seq)
            .map(|p| p.part_index)
            .collect();
        assert_eq!(
            plan.parts.iter().map(|c| c.part_index).collect::<Vec<_>>(),
            indices,
            "recorded part cuts match the emitted parts"
        );
        assert_eq!(plan.timescale, video.spec.timescale);
    }
    plans
}

fn demux(name: &str) -> Media {
    let bytes = test_fixtures::read(name);
    TsDemux::new().unpackage(&bytes).unwrap()
}

/// Full alignment contract of one follower against one leader.
fn check_follower(audio: &Track, movie_ts: u32, plans: &[CutPlan]) {
    assert!(
        plans.len() >= 2,
        "fixture must yield several leader segments"
    );
    let (init, parts, segs) = run_follower(audio, movie_ts, plans);
    let ts = audio.spec.timescale;

    assert_eq!(trak_count(&init), 1, "follower init carries only its trak");
    assert_no_errors("follower init", &validate_init_segment(&init));

    let mut expected_segment: Vec<Vec<&Sample>> = vec![Vec::new(); plans.len()];
    let mut expected_part: Vec<Vec<Vec<&Sample>>> = plans
        .iter()
        .map(|p| vec![Vec::new(); p.parts.len().max(1)])
        .collect();
    for s in &audio.samples {
        let d = s.dts.unwrap();
        let si = segment_of(d, ts, plans);
        expected_segment[si].push(s);
        expected_part[si][part_slot_of(d, ts, &plans[si])].push(s);
    }

    let nonempty_segments: Vec<u32> = plans
        .iter()
        .zip(&expected_segment)
        .filter(|(_, e)| !e.is_empty())
        .map(|(p, _)| p.segment_seq)
        .collect();
    assert_eq!(
        segs.iter().map(|s| s.segment_seq).collect::<Vec<_>>(),
        nonempty_segments,
        "follower segment sequence numbers are the leader's"
    );

    let mut emitted_total = 0usize;
    for seg in &segs {
        let si = plans
            .iter()
            .position(|p| p.segment_seq == seg.segment_seq)
            .unwrap();
        let expected = &expected_segment[si];
        assert_eq!(
            mdat_payload(&seg.bytes),
            concat(expected),
            "segment {} sample payloads, in order",
            seg.segment_seq
        );
        assert_no_errors(
            &format!("segment {}", seg.segment_seq),
            &validate_media_segment(&seg.bytes),
        );
        if si > 0 {
            let first = expected[0].dts.unwrap();
            assert!(
                opens_next(first, ts, plans[si].segment_start, plans[si].timescale),
                "first sample of segment {} starts at or after the leader start",
                seg.segment_seq
            );
        }
        if let Some(next) = plans.get(si + 1) {
            let last = expected.last().unwrap().dts.unwrap();
            assert!(
                !opens_next(last, ts, next.segment_start, next.timescale),
                "last sample of segment {} starts before the next leader start",
                seg.segment_seq
            );
        }
        let seg_parts: Vec<&PartInfo> = parts
            .iter()
            .filter(|p| p.segment_seq == seg.segment_seq)
            .collect();
        let expected_parts: Vec<(usize, &Vec<&Sample>)> = expected_part[si]
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_empty())
            .collect();
        assert_eq!(
            seg_parts.len(),
            expected_parts.len(),
            "segment {} part count",
            seg.segment_seq
        );
        assert_eq!(seg.part_count as usize, seg_parts.len());
        let mut joined = Vec::new();
        for (part, (slot, exp)) in seg_parts.iter().zip(expected_parts) {
            let cut = plans[si].parts.get(slot);
            let want_index = cut.map_or(0, |c| c.part_index);
            assert_eq!(part.part_index, want_index, "part index follows the leader");
            assert_eq!(mdat_payload(&part.bytes), concat(exp), "part payloads");
            joined.extend(mdat_payload(&part.bytes));
        }
        assert_eq!(
            joined,
            mdat_payload(&seg.bytes),
            "parts of segment {} concatenate to the segment",
            seg.segment_seq
        );
        emitted_total += expected.len();
    }
    assert_eq!(emitted_total, audio.samples.len(), "no loss, no duplicate");
    let all: Vec<u8> = segs.iter().flat_map(|s| mdat_payload(&s.bytes)).collect();
    assert_eq!(
        all,
        concat(&audio.samples.iter().collect::<Vec<_>>()),
        "union of segments is the demuxed audio, in order"
    );
}

fn check_fixture(name: &str) {
    let ir = demux(name);
    let video = ir
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let plans = leader_plans(video, ir.movie_timescale);
    assert_eq!(
        plans[0].segment_start,
        video.samples[0].dts.unwrap(),
        "leader cuts are absolute decode times"
    );
    for pair in plans.windows(2) {
        let boundary = pair[1].segment_start;
        assert!(
            video
                .samples
                .iter()
                .any(|s| s.dts == Some(boundary) && s.flags.is_sync),
            "segment {} starts on a video sync sample",
            pair[1].segment_seq
        );
    }
    let audio: Vec<&Track> = ir
        .tracks
        .iter()
        .filter(|t| t.spec.config.audio_sample_rate().is_some())
        .collect();
    assert!(!audio.is_empty());
    for a in audio {
        check_follower(a, ir.movie_timescale, &plans);
    }
}

#[test]
fn h264_aac_follower_aligns_with_leader() {
    check_fixture("ts/h264_aac.ts");
}

#[test]
fn h264_aac_40s_follower_aligns_with_leader() {
    check_fixture("ts/h264_aac_40s.ts");
}

#[test]
fn france2_each_eac3_track_aligns_with_leader() {
    let ir = demux("ts/france2.ts");
    let audio = ir
        .tracks
        .iter()
        .filter(|t| t.spec.config.audio_sample_rate().is_some())
        .count();
    assert_eq!(audio, 3, "france2 carries three E-AC-3 audio tracks");
    check_fixture("ts/france2.ts");
}

fn audio_template() -> (Track, u32) {
    let ir = demux("ts/h264_aac.ts");
    let movie_ts = ir.movie_timescale;
    let mut track = ir
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();
    track.spec.timescale = AAC_RATE;
    (track, movie_ts)
}

fn frame(n: i64) -> Sample {
    let tag = (n as u16).to_be_bytes();
    Sample::new(
        vec![tag[0], tag[1], 0xAA, 0xBB],
        Some(n * AAC_FRAME_TICKS),
        Some(n * AAC_FRAME_TICKS),
        Some(AAC_FRAME_TICKS as u32),
        true,
    )
}

fn frames(range: impl Iterator<Item = i64>) -> Vec<Sample> {
    range.map(frame).collect()
}

fn payload_of(range: impl Iterator<Item = i64>) -> Vec<u8> {
    frames(range).iter().flat_map(|s| s.data.to_vec()).collect()
}

fn synthetic(samples: Vec<Sample>, plans: &[CutPlan]) -> (Vec<PartInfo>, Vec<SegmentInfo>) {
    let (mut audio, movie_ts) = audio_template();
    audio.samples = samples;
    let (_, parts, segs) = run_follower(&audio, movie_ts, plans);
    (parts, segs)
}

fn plan(seq: u32, start_follower_ticks: i64) -> CutPlan {
    CutPlan::new(
        seq,
        leader_ticks(start_follower_ticks),
        LEADER_TS,
        Vec::new(),
    )
}

#[test]
fn sample_starting_exactly_on_segment_boundary_opens_next_segment() {
    // Frame 10 starts at 10240 ticks = 19200 leader ticks, exactly boundary 2.
    let plans = [plan(1, 0), plan(2, 10 * 1024), plan(3, 20 * 1024)];
    let (_, segs) = synthetic(frames(0..30), &plans);
    assert_eq!(segs.len(), 3);
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(0..10));
    assert_eq!(mdat_payload(&segs[1].bytes), payload_of(10..20));
    assert_eq!(mdat_payload(&segs[2].bytes), payload_of(20..30));
}

#[test]
fn frame_straddling_boundary_stays_in_segment_where_it_starts() {
    // Boundary 2 is 1 leader tick after frame 10's start: frame 10 starts
    // before it and spills over, so it stays in segment 1 (a floor rescale
    // would move it). Boundary 3 is 1 tick before frame 20's start: frame 20
    // starts after it, so it opens segment 3.
    let b2 = leader_ticks(10 * 1024) + 1;
    let b3 = leader_ticks(20 * 1024) - 1;
    let plans = [
        CutPlan::new(1, 0, LEADER_TS, Vec::new()),
        CutPlan::new(2, b2, LEADER_TS, Vec::new()),
        CutPlan::new(3, b3, LEADER_TS, Vec::new()),
    ];
    let (_, segs) = synthetic(frames(0..30), &plans);
    assert_eq!(segs.len(), 3);
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(0..11));
    assert_eq!(mdat_payload(&segs[1].bytes), payload_of(11..20));
    assert_eq!(mdat_payload(&segs[2].bytes), payload_of(20..30));
}

#[test]
fn sample_starting_exactly_on_part_boundary_opens_next_part() {
    let cuts = vec![
        PartCut::new(0, 0, LEADER_TS),
        PartCut::new(1, leader_ticks(5 * 1024), LEADER_TS),
        PartCut::new(2, leader_ticks(10 * 1024), LEADER_TS),
    ];
    let plans = [CutPlan::new(1, 0, LEADER_TS, cuts), plan(2, 15 * 1024)];
    let (parts, segs) = synthetic(frames(0..20), &plans);
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].part_count, 3);
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(0..15));
    let seg1: Vec<_> = parts.iter().filter(|p| p.segment_seq == 1).collect();
    assert_eq!(
        seg1.iter().map(|p| p.part_index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(mdat_payload(&seg1[0].bytes), payload_of(0..5));
    assert_eq!(mdat_payload(&seg1[1].bytes), payload_of(5..10));
    assert_eq!(mdat_payload(&seg1[2].bytes), payload_of(10..15));
}

#[test]
fn frame_straddling_part_boundary_stays_in_part_where_it_starts() {
    let cuts = vec![
        PartCut::new(0, 0, LEADER_TS),
        PartCut::new(1, leader_ticks(5 * 1024) + 1, LEADER_TS),
    ];
    let plans = [CutPlan::new(1, 0, LEADER_TS, cuts)];
    let (parts, _) = synthetic(frames(0..10), &plans);
    assert_eq!(parts.len(), 2);
    assert_eq!(mdat_payload(&parts[0].bytes), payload_of(0..6));
    assert_eq!(mdat_payload(&parts[1].bytes), payload_of(6..10));
}

#[test]
fn leader_segment_without_follower_samples_is_not_emitted() {
    // Leader segment 2 spans frames 10..15's time but no follower sample
    // starts in it; segments 1 and 3 are emitted with their own sequence
    // numbers and nothing is lost.
    let samples: Vec<Sample> = frames((0..10).chain(15..25));
    let plans = [plan(1, 0), plan(2, 10 * 1024), plan(3, 15 * 1024)];
    let (parts, segs) = synthetic(samples, &plans);
    assert_eq!(
        segs.iter().map(|s| s.segment_seq).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(
        parts.iter().map(|p| p.segment_seq).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(0..10));
    assert_eq!(mdat_payload(&segs[1].bytes), payload_of(15..25));
}

#[test]
fn boundary_before_any_follower_sample_leaves_first_segment_empty() {
    // Samples begin at frame 4; segment 2 opens exactly at frame 4's start, so
    // segment 1 has no sample starting in it and only segment 2 is emitted.
    let plans = [plan(1, 0), plan(2, 4 * 1024)];
    let (parts, segs) = synthetic(frames(4..14), &plans);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].segment_seq, 2);
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(4..14));
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].segment_seq, 2);
}

#[test]
fn samples_before_the_first_plan_join_the_first_segment() {
    let plans = [plan(7, 5 * 1024), plan(8, 10 * 1024)];
    let (_, segs) = synthetic(frames(0..15), &plans);
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].segment_seq, 7);
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(0..10));
    assert_eq!(segs[1].segment_seq, 8);
    assert_eq!(mdat_payload(&segs[1].bytes), payload_of(10..15));
}

#[test]
fn pending_buffer_cap_is_a_typed_error_at_the_exact_limit() {
    let (audio, movie_ts) = audio_template();
    let mut f = LlHlsSegmenter::with_part_target(
        vec![audio.spec.clone()],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap();
    f.follow(plan(1, 0)).unwrap();
    for n in 0..MAX_PENDING_SAMPLES_PER_TRACK as i64 {
        f.push(audio.spec.track_id, frame(n)).unwrap();
    }
    let err = f
        .push(
            audio.spec.track_id,
            frame(MAX_PENDING_SAMPLES_PER_TRACK as i64),
        )
        .unwrap_err();
    assert_eq!(
        err,
        Error::InvalidInput(
            "follower segmenter pending buffer full: no boundary received within \
             MAX_PENDING_SAMPLES_PER_TRACK samples"
        )
    );
}

#[test]
fn follow_rejects_invalid_plans_with_typed_errors() {
    let (audio, movie_ts) = audio_template();
    let new = || {
        LlHlsSegmenter::with_part_target(
            vec![audio.spec.clone()],
            movie_ts,
            SEGMENT_TARGET_SECS,
            PART_TARGET_MS,
        )
        .unwrap()
    };

    assert_eq!(
        new().follow(CutPlan::new(1, 0, 0, Vec::new())).unwrap_err(),
        Error::InvalidInput("follow: plan timescale must be > 0")
    );

    let mut f = new();
    f.follow(plan(5, 800)).unwrap();
    let expected = Error::InvalidInput(
        "follow: plan must advance the previous plan's sequence number and start",
    );
    assert_eq!(f.follow(plan(5, 1600)).unwrap_err(), expected);
    assert_eq!(f.follow(plan(6, 400)).unwrap_err(), expected);

    let backwards = vec![
        PartCut::new(0, 90, LEADER_TS),
        PartCut::new(1, 10, LEADER_TS),
    ];
    assert_eq!(
        new()
            .follow(CutPlan::new(1, 0, LEADER_TS, backwards))
            .unwrap_err(),
        Error::InvalidInput("follow: part starts must be non-decreasing")
    );

    let mut own = new();
    for n in 0..200 {
        own.push(audio.spec.track_id, frame(n)).unwrap();
    }
    assert_eq!(
        own.follow(plan(1, 0)).unwrap_err(),
        Error::InvalidInput(
            "follow: segmenter already cut on its own; call follow before the first cut"
        )
    );
}

#[test]
fn follower_push_rejects_a_sample_without_dts() {
    let (audio, movie_ts) = audio_template();
    let mut f = LlHlsSegmenter::with_part_target(
        vec![audio.spec.clone()],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap();
    f.follow(plan(1, 0)).unwrap();
    let s = Sample::new(vec![1, 2], None, None, None, true);
    assert_eq!(
        f.push(audio.spec.track_id, s).unwrap_err(),
        Error::InvalidInput("follower segmenter push: sample has no dts")
    );
}

#[test]
fn flush_with_buffered_samples_and_no_plan_is_an_error() {
    let (audio, movie_ts) = audio_template();
    let mut f = LlHlsSegmenter::with_part_target(
        vec![audio.spec.clone()],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap();
    f.follow(plan(1, 0)).unwrap();
    f.flush().unwrap();
    f.push(audio.spec.track_id, frame(0)).unwrap();
    assert_eq!(
        f.flush().unwrap_err(),
        Error::InvalidInput("flush: follower has buffered samples but no cut plan")
    );
}

fn first_tfdt(segment: &[u8]) -> u64 {
    let mut at = 0;
    while at + 8 <= segment.len() {
        let size = u32::from_be_bytes([
            segment[at],
            segment[at + 1],
            segment[at + 2],
            segment[at + 3],
        ]) as usize;
        if &segment[at + 4..at + 8] == b"moof" {
            let moof =
                transmux::movie_fragment::MovieFragmentBox::parse_body(&segment[at + 8..at + size])
                    .unwrap();
            return moof.traf[0].tfdt.as_ref().unwrap().base_media_decode_time();
        }
        at += size;
    }
    panic!("no moof");
}

fn follower_with_origin(origin_frame: i64) -> LlHlsSegmenter {
    let (audio, movie_ts) = audio_template();
    let mut f = LlHlsSegmenter::with_part_target(
        vec![audio.spec.clone()],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap();
    f.set_timeline_origin(leader_ticks(origin_frame * AAC_FRAME_TICKS), LEADER_TS);
    f
}

#[test]
fn shared_origin_offsets_the_first_follower_segment_by_its_first_sample() {
    let (audio, _) = audio_template();
    let track = audio.spec.track_id;
    let mut f = follower_with_origin(2);
    f.follow(plan(1, 2 * 1024)).unwrap();
    for n in 5..12 {
        f.push(track, frame(n)).unwrap();
    }
    f.flush().unwrap();
    let segs = f.take_ready_segments();
    assert_eq!(segs.len(), 1);
    assert_eq!(first_tfdt(&segs[0].bytes), 3 * AAC_FRAME_TICKS as u64);
}

#[test]
fn segment_after_a_silent_leader_segment_restarts_from_its_own_decode_time() {
    let (audio, _) = audio_template();
    let track = audio.spec.track_id;
    let mut f = follower_with_origin(0);
    f.follow(plan(1, 0)).unwrap();
    for n in 0..10 {
        f.push(track, frame(n)).unwrap();
    }
    f.follow(plan(2, 10 * 1024)).unwrap();
    f.follow(plan(3, 20 * 1024)).unwrap();
    for n in 20..30 {
        f.push(track, frame(n)).unwrap();
    }
    f.flush().unwrap();
    let segs = f.take_ready_segments();
    assert_eq!(
        segs.iter().map(|s| s.segment_seq).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(first_tfdt(&segs[0].bytes), 0);
    assert_eq!(first_tfdt(&segs[1].bytes), 20 * AAC_FRAME_TICKS as u64);
}

#[test]
fn incremental_part_cuts_match_whole_segment() {
    let ir = demux("ts/h264_aac.ts");
    let video = ir
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let audio = ir
        .tracks
        .iter()
        .find(|t| t.spec.config.audio_sample_rate().is_some())
        .expect("audio track");
    let new_segmenter = |track: &Track| {
        LlHlsSegmenter::with_part_target(
            vec![track.spec.clone()],
            ir.movie_timescale,
            SEGMENT_TARGET_SECS,
            PART_TARGET_MS,
        )
        .unwrap()
    };

    let mut leader = new_segmenter(video);
    leader.record_cuts();
    leader.record_part_cuts();
    for s in &video.samples {
        leader.push(video.spec.track_id, s.clone()).unwrap();
    }
    leader.flush().unwrap();
    let part_cuts = leader.take_ready_part_cuts();
    let plans = leader.take_ready_cut_plans();
    assert!(plans.len() >= 2, "need several leader segments");

    let mut incremental = new_segmenter(audio);
    let mut next_sample = 0;
    let mut incremental_parts: Vec<PartInfo> = Vec::new();
    for cut in &part_cuts {
        while let Some(sample) = audio.samples.get(next_sample)
            && sample
                .dts
                .is_some_and(|d| !opens_next(d, audio.spec.timescale, cut.start_dts, cut.timescale))
        {
            incremental
                .push(audio.spec.track_id, sample.clone())
                .unwrap();
            next_sample += 1;
        }
        incremental
            .follow_part(cut.msn, cut.part_index, cut.start_dts, cut.timescale)
            .unwrap();
        incremental_parts.extend(incremental.take_ready_parts());
    }
    let parts_before_flush = incremental_parts.len();
    assert!(
        parts_before_flush > 0,
        "parts must be emitted while cuts are still arriving, not only at flush"
    );
    for sample in &audio.samples[next_sample..] {
        incremental
            .push(audio.spec.track_id, sample.clone())
            .unwrap();
    }
    incremental.flush().unwrap();

    let mut whole = new_segmenter(audio);
    whole.follow(plans[0].clone()).unwrap();
    for s in &audio.samples {
        whole.push(audio.spec.track_id, s.clone()).unwrap();
    }
    for plan in &plans[1..] {
        whole.follow(plan.clone()).unwrap();
    }
    whole.flush().unwrap();

    let whole_parts = whole.take_ready_parts();
    let whole_segs = whole.take_ready_segments();
    incremental_parts.extend(incremental.take_ready_parts());
    let incremental_segs = incremental.take_ready_segments();
    assert_eq!(incremental_parts.len(), whole_parts.len());
    for (a, b) in incremental_parts.iter().zip(&whole_parts) {
        assert_eq!((a.segment_seq, a.part_index), (b.segment_seq, b.part_index));
        assert_eq!(a.duration, b.duration);
        assert_eq!(a.independent, b.independent);
        assert_eq!(mdat_payload(&a.bytes), mdat_payload(&b.bytes));
    }
    assert_eq!(incremental_segs.len(), whole_segs.len());
    for (a, b) in incremental_segs.iter().zip(&whole_segs) {
        assert_eq!(a.segment_seq, b.segment_seq);
        assert_eq!(a.part_count, b.part_count);
        assert_eq!(a.duration, b.duration);
        assert_eq!(mdat_payload(&a.bytes), mdat_payload(&b.bytes));
    }
}

fn part_follower() -> LlHlsSegmenter {
    let (audio, movie_ts) = audio_template();
    LlHlsSegmenter::with_part_target(
        vec![audio.spec],
        movie_ts,
        SEGMENT_TARGET_SECS,
        PART_TARGET_MS,
    )
    .unwrap()
}

#[test]
fn follow_part_holds_a_part_until_a_sample_reaches_the_next_boundary() {
    let mut f = part_follower();
    let track_id = audio_template().0.spec.track_id;
    f.follow_part(1, 0, 0, LEADER_TS).unwrap();
    f.follow_part(1, 1, leader_ticks(4 * 1024), LEADER_TS)
        .unwrap();
    for n in 0..4 {
        f.push(track_id, frame(n)).unwrap();
    }
    assert!(
        f.take_ready_parts().is_empty(),
        "no sample starts at or after the boundary yet"
    );
    f.push(track_id, frame(4)).unwrap();
    let parts = f.take_ready_parts();
    assert_eq!(parts.len(), 1);
    assert_eq!((parts[0].segment_seq, parts[0].part_index), (1, 0));
    assert_eq!(mdat_payload(&parts[0].bytes), payload_of(0..4));
}

#[test]
fn follow_part_closes_the_previous_segment_at_the_new_boundary() {
    let mut f = part_follower();
    let track_id = audio_template().0.spec.track_id;
    f.follow_part(1, 0, 0, LEADER_TS).unwrap();
    for n in 0..8 {
        f.push(track_id, frame(n)).unwrap();
    }
    f.follow_part(2, 0, leader_ticks(6 * 1024), LEADER_TS)
        .unwrap();
    let segs = f.take_ready_segments();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].segment_seq, 1);
    assert_eq!(segs[0].part_count, 1);
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(0..6));
    f.flush().unwrap();
    let segs = f.take_ready_segments();
    assert_eq!(mdat_payload(&segs[0].bytes), payload_of(6..8));
}

#[test]
fn follow_part_rejects_invalid_boundaries_with_typed_errors() {
    let mut f = part_follower();
    assert!(matches!(
        f.follow_part(1, 0, 0, 0),
        Err(Error::InvalidInput(_))
    ));
    f.follow_part(2, 0, leader_ticks(8 * 1024), LEADER_TS)
        .unwrap();
    assert!(matches!(
        f.follow_part(1, 0, leader_ticks(16 * 1024), LEADER_TS),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        f.follow_part(2, 1, leader_ticks(4 * 1024), LEADER_TS),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn follow_part_buffer_overflow_is_a_typed_error() {
    let mut f = part_follower();
    let track_id = audio_template().0.spec.track_id;
    f.follow_part(1, 0, 0, LEADER_TS).unwrap();
    f.follow_part(1, 1, leader_ticks(1 << 40), LEADER_TS)
        .unwrap();
    let mut result = Ok(());
    for n in 0..=MAX_PENDING_SAMPLES_PER_TRACK as i64 {
        result = f.push(track_id, frame(n));
        if result.is_err() {
            break;
        }
    }
    assert!(matches!(result, Err(Error::InvalidInput(_))));
}
