// SPDX-License-Identifier: MIT OR Apache-2.0
//! Issue #1162 R4 — `LiveJoin` over real captures joined at an arbitrary
//! sample. Expected positions are derived by a test-owned NAL walk
//! (`iter_length_prefixed_nals`, NAL type 5 = IDR, type 7 = SPS) that does not
//! go through `LiveJoin`/`access_unit_is_rap`, so the assertions cross-check
//! the implementation instead of re-running it.

use broadcast_common::Unpackage;
use std::path::PathBuf;
use transmux::{
    DropReason, JoinDecision, JoinPolicy, LiveJoin, RapKind, Sample, TsDemux,
    iter_length_prefixed_nals,
};

fn video_samples(rel: &str) -> (transmux::TrackSpec, Vec<Sample>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel);
    let ts = std::fs::read(path).expect("fixture");
    let m = TsDemux::new().unpackage(&ts).expect("demux");
    let t = m
        .tracks
        .into_iter()
        .find(|t| t.spec.config.video_scan().is_some())
        .expect("video track");
    (t.spec, t.samples)
}

fn nal_types(s: &Sample) -> Vec<u8> {
    iter_length_prefixed_nals(&s.data)
        .unwrap()
        .iter()
        .map(|n| n[0] & 0x1F)
        .collect()
}

/// Indices of samples whose access unit has a NAL of `t`.
fn with_nal(samples: &[Sample], t: u8) -> Vec<usize> {
    (0..samples.len())
        .filter(|&i| nal_types(&samples[i]).contains(&t))
        .collect()
}

/// Run a fresh `LiveJoin` over `samples[cut..]`; returns
/// `(index of first JoinPoint, its kind, drops by reason, all decisions)`.
struct Run {
    join_index: Option<usize>,
    kind: Option<RapKind>,
    before: Vec<usize>,
    leading: Vec<usize>,
    not_self_contained: Vec<usize>,
    kept_pts: Vec<i64>,
}

fn run(spec: &transmux::TrackSpec, samples: &[Sample], cut: usize, policy: JoinPolicy) -> Run {
    let mut j = LiveJoin::for_config(&spec.config)
        .expect("AVC/HEVC track")
        .with_policy(policy);
    let mut r = Run {
        join_index: None,
        kind: None,
        before: vec![],
        leading: vec![],
        not_self_contained: vec![],
        kept_pts: vec![],
    };
    for (i, s) in samples.iter().enumerate().skip(cut) {
        match j.push(s) {
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint) => r.before.push(i),
            JoinDecision::Drop(DropReason::LeadingPicture) => r.leading.push(i),
            JoinDecision::Drop(DropReason::NotSelfContained) => r.not_self_contained.push(i),
            JoinDecision::Drop(_) => unreachable!("non_exhaustive: new reason needs a test"),
            JoinDecision::JoinPoint(k) => {
                if r.join_index.is_none() {
                    r.join_index = Some(i);
                    r.kind = Some(k);
                }
                r.kept_pts.push(s.pts.unwrap());
            }
            JoinDecision::Keep => r.kept_pts.push(s.pts.unwrap()),
            _ => unreachable!("non_exhaustive: new decision needs a test"),
        }
    }
    r
}

#[test]
fn closed_gop_idr_stream_cut_mid_gop_drops_exactly_up_to_the_next_idr() {
    let (spec, samples) = video_samples("fixtures/ts/h264_aac_40s.ts");
    let idrs = with_nal(&samples, 5);
    assert_eq!(&idrs[..3], &[0, 25, 50], "GOP of 25 samples");
    // From the start: joins at sample 0 immediately, nothing dropped.
    let r = run(&spec, &samples, 0, JoinPolicy::AnyRap);
    assert_eq!(r.join_index, Some(0));
    assert_eq!(r.kind, Some(RapKind::Idr));
    assert!(r.before.is_empty() && r.leading.is_empty());
    assert_eq!(r.kept_pts.len(), samples.len());
    // Cut at non-RAP samples 1, 5, 20, 33 (and one right before an IDR).
    for (cut, next_idr) in [(1usize, 25usize), (5, 25), (20, 25), (24, 25), (33, 50)] {
        assert!(!idrs.contains(&cut));
        let r = run(&spec, &samples, cut, JoinPolicy::AnyRap);
        assert_eq!(r.join_index, Some(next_idr), "cut {cut}");
        assert_eq!(r.kind, Some(RapKind::Idr));
        // Exact drop count: every sample from the cut to the IDR, no leading.
        assert_eq!(r.before, (cut..next_idr).collect::<Vec<_>>());
        assert!(r.leading.is_empty());
        assert_eq!(r.kept_pts.len(), samples.len() - next_idr);
        // The first kept sample is the IDR; nothing kept precedes it in output.
        let rap_pts = samples[next_idr].pts.unwrap();
        assert!(r.kept_pts.iter().all(|&p| p >= rap_pts));
    }
}

#[test]
fn open_gop_stream_joins_on_a_recovery_point_and_drops_leading_pictures() {
    let (spec, samples) = video_samples("fixtures/ts/gulli-opengop.ts");
    assert!(with_nal(&samples, 5).is_empty(), "no IDR anywhere");
    let gops = with_nal(&samples, 7);
    assert_eq!(gops, [0, 32, 64, 114, 146]);
    let rp = RapKind::RecoveryPoint {
        recovery_frame_cnt: 0,
        exact_match: true,
        broken_link: false,
    };

    // From sample 0: the stream starts on a recovery point; the 3 pictures
    // after it in decode order precede it in output order (open GOP).
    let r = run(&spec, &samples, 0, JoinPolicy::AnyRap);
    assert_eq!((r.join_index, r.kind), (Some(0), Some(rp)));
    assert!(r.before.is_empty());
    assert_eq!(r.leading, vec![1, 2, 3]);
    let rap_pts = samples[0].pts.unwrap();
    assert!(samples[1..=3].iter().all(|s| s.pts.unwrap() < rap_pts));
    assert!(samples[4].pts.unwrap() > rap_pts);
    assert_eq!(r.kept_pts.len(), 147 - 3);
    assert!(r.kept_pts.iter().all(|&p| p >= rap_pts));

    // Mid-GOP cuts: drop to the next SPS-led GOP, then its leading pictures.
    for (cut, next) in [
        (1usize, 32usize),
        (5, 32),
        (20, 32),
        (31, 32),
        (33, 64),
        (65, 114),
    ] {
        let r = run(&spec, &samples, cut, JoinPolicy::AnyRap);
        assert_eq!(r.join_index, Some(next), "cut {cut}");
        assert_eq!(r.kind, Some(rp));
        assert_eq!(r.before, (cut..next).collect::<Vec<_>>());
        let rap_pts = samples[next].pts.unwrap();
        // Leading = the samples right after the join point with pts below it.
        let want_leading: Vec<usize> = (next + 1..samples.len())
            .take_while(|&i| samples[i].pts.unwrap() < rap_pts)
            .collect();
        assert_eq!(r.leading, want_leading, "cut {cut}");
        // The GOP at 114 has no leading pictures; the others have 3.
        assert_eq!(want_leading.len(), if next == 114 { 0 } else { 3 });
        assert!(r.kept_pts.iter().all(|&p| p >= rap_pts));
        assert_eq!(
            r.before.len() + r.leading.len() + r.kept_pts.len(),
            samples.len() - cut
        );
    }
}

#[test]
fn self_contained_policy_never_joins_the_idr_less_stream() {
    let (spec, samples) = video_samples("fixtures/ts/gulli-opengop.ts");
    let r = run(&spec, &samples, 0, JoinPolicy::SelfContainedOnly);
    assert_eq!(r.join_index, None, "recovery points are not self-contained");
    assert!(r.kept_pts.is_empty());
    // The five SPS-led recovery points are reported as NotSelfContained, every
    // other sample as before-RAP.
    assert_eq!(r.not_self_contained, [0, 32, 64, 114, 146]);
    assert_eq!(r.before.len(), samples.len() - 5);
}

#[test]
fn rejoin_after_failure_waits_for_the_next_random_access_point() {
    let (spec, samples) = video_samples("fixtures/ts/h264_aac_40s.ts");
    let mut j = LiveJoin::for_config(&spec.config).unwrap();
    for s in &samples[..30] {
        j.push(s);
    }
    // Decoder reports a failure at sample 30: ask for the next RAP (sample 50).
    j.rejoin();
    let mut dropped = 0;
    let mut joined = None;
    for (i, s) in samples.iter().enumerate().skip(30) {
        match j.push(s) {
            JoinDecision::Drop(_) => dropped += 1,
            JoinDecision::JoinPoint(k) => {
                joined = Some((i, k));
                break;
            }
            _ => panic!("kept a sample while waiting"),
        }
    }
    assert_eq!(joined, Some((50, RapKind::Idr)));
    assert_eq!(dropped, 20);
}

#[test]
fn open_gop_hevc_style_progressive_fixtures_join_at_sample_zero() {
    for rel in [
        "fixtures/ts/h264/main.ts",
        "fixtures/ts/h264/interlaced.ts",
        "fixtures/ts/hevc/main.ts",
        "fixtures/ts/hevc/main10.ts",
    ] {
        let (spec, samples) = video_samples(rel);
        let r = run(&spec, &samples, 0, JoinPolicy::SelfContainedOnly);
        assert_eq!(r.join_index, Some(0), "{rel}");
        assert_eq!(r.kind, Some(RapKind::Idr), "{rel}");
        assert!(r.before.is_empty() && r.leading.is_empty(), "{rel}");
    }
}
