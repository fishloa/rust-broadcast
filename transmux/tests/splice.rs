//! Integration tests for the IR-level timeline splice / concat → SSAI transforms
//! (issue #475): contiguity + byte preservation, end-to-end `tfdt` monotonicity
//! through the muxer, SSAI insert timing, keyframe-alignment snapping, and
//! discontinuity reporting driving the segmenter.

use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, Media,
    MovieFragmentBox, Sample, Segmenter, Track, TrackSpec, concat, parse_box,
    snap_to_preceding_sync, splice_insert,
};

const TIMESCALE: u32 = 90_000;

fn avc_spec(track_id: u32) -> TrackSpec {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![0x67, 0x42, 0x00, 0x1e])],
        pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    TrackSpec {
        track_id,
        timescale: TIMESCALE,
        config: CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    }
}

/// Build a sample whose `data` bytes are a recognizable pattern (so byte
/// preservation is verifiable). `tag` distinguishes samples across media.
fn sample(tag: u8, index: usize, duration: u32, is_sync: bool) -> Sample {
    Sample {
        data: vec![tag, index as u8, 0xAB, 0xCD],
        duration,
        is_sync,
        composition_offset: 0,
    }
}

/// A video track: first sample is a sync sample (keyframe), then `sync_period`
/// samples per GOP.
fn video_track(track_id: u32, tag: u8, count: usize, dur: u32, sync_period: usize) -> Track {
    let samples = (0..count)
        .map(|i| sample(tag, i, dur, i % sync_period == 0))
        .collect();
    Track::new(avc_spec(track_id), samples)
}

fn media_of(track: Track, start_decode_time: u64) -> Media {
    Media::new(vec![track.with_start_decode_time(start_decode_time)], 1000)
}

fn track_span(track: &Track) -> u64 {
    track.samples.iter().map(|s| s.duration as u64).sum()
}

/// Reconstruct each sample's DTS from a track (start + running sum).
fn dts_sequence(track: &Track) -> Vec<u64> {
    let mut dts = track.start_decode_time;
    let mut out = Vec::new();
    for s in &track.samples {
        out.push(dts);
        dts += s.duration as u64;
    }
    out
}

/// Extract the `tfdt` `base_media_decode_time` of the first `traf` in every
/// `moof` of a stream of CMAF segments, in order.
fn tfdts(segments: &[Vec<u8>]) -> Vec<u64> {
    let mut out = Vec::new();
    for seg in segments {
        let mut offset = 0usize;
        while offset + 8 <= seg.len() {
            let (bx, consumed) = parse_box(&seg[offset..]).expect("parse box");
            if &bx.header.box_type.0 == b"moof" {
                let moof = MovieFragmentBox::parse_body(bx.body).expect("parse moof");
                let tfdt = moof.traf[0]
                    .tfdt
                    .as_ref()
                    .expect("traf has tfdt")
                    .base_media_decode_time();
                out.push(tfdt);
            }
            if consumed == 0 {
                break;
            }
            offset += consumed;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Test 1 — concat contiguity + byte preservation.
// ---------------------------------------------------------------------------
#[test]
fn concat_is_contiguous_and_preserves_bytes() {
    // A: 6 samples @3000 starting at DTS 90_000. B: 4 samples @3000, first sync.
    let a = media_of(video_track(1, 0xA0, 6, 3000, 3), 90_000);
    let b = media_of(video_track(1, 0xB0, 4, 3000, 4), 0);

    let k = a.tracks[0].samples.len();
    let m = b.tracks[0].samples.len();
    let ta_end = a.tracks[0].start_decode_time + track_span(&a.tracks[0]); // A's end DTS
    let a_span = track_span(&a.tracks[0]);
    let b_span = track_span(&b.tracks[0]);

    let res = concat(&a, &b).unwrap();
    let out = &res.media.tracks[0];

    // K + M samples.
    assert_eq!(out.samples.len(), k + m);

    // Byte-for-byte preservation of both contributions.
    for i in 0..k {
        assert_eq!(out.samples[i].data, a.tracks[0].samples[i].data, "A[{i}]");
    }
    for j in 0..m {
        assert_eq!(
            out.samples[k + j].data,
            b.tracks[0].samples[j].data,
            "B[{j}]"
        );
    }

    // B's first sample DTS == A's end DTS (contiguous, no gap/overlap).
    let dts = dts_sequence(out);
    assert_eq!(dts[k], ta_end, "join DTS == A end DTS");
    // Strictly monotonic non-decreasing overall.
    for w in dts.windows(2) {
        assert!(w[1] >= w[0], "monotonic DTS");
    }

    // Total span == A_span + B_span.
    assert_eq!(track_span(out), a_span + b_span);

    // Discontinuity reported at the join sample.
    assert_eq!(res.discontinuity_points.len(), 1);
    assert_eq!(res.discontinuity_points[0].sample_index, k);
    assert_eq!(res.discontinuity_points[0].track_id, 1);
    assert_eq!(res.discontinuity_points[0].presentation_time, ta_end);
}

// ---------------------------------------------------------------------------
// Test 2 — concat end-to-end tfdt monotonic through the segmenter/muxer.
// ---------------------------------------------------------------------------
#[test]
fn concat_muxed_tfdts_are_monotonic_across_join() {
    let a = media_of(video_track(1, 0xA0, 6, 3000, 3), 90_000);
    let b = media_of(video_track(1, 0xB0, 6, 3000, 3), 0);
    let ta_end = a.tracks[0].start_decode_time + track_span(&a.tracks[0]);

    let res = concat(&a, &b).unwrap();
    let joined = &res.media.tracks[0];

    // Segment at ~0.1s (3 samples/GOP × 3000/90000 ≈ 0.1s) so several segments
    // straddle the join, anchored at the joined track's start_decode_time.
    let mut seg = Segmenter::new(vec![joined.spec.clone()], 1000, 0.1).unwrap();
    // The segmenter counts from 0; offset every emitted tfdt by the anchor.
    let anchor = joined.start_decode_time;
    for s in &joined.samples {
        seg.push(1, s.clone()).unwrap();
    }
    seg.flush().unwrap();
    let segments = seg.take_ready();
    assert!(segments.len() >= 2, "expected multiple segments");

    let mut tfdt_list: Vec<u64> = tfdts(&segments).iter().map(|t| t + anchor).collect();
    // First tfdt is the track anchor.
    assert_eq!(tfdt_list[0], anchor, "first tfdt == anchor");
    // Strictly monotonic non-decreasing.
    for w in tfdt_list.windows(2) {
        assert!(w[1] > w[0], "tfdt strictly increasing: {w:?}");
    }
    // One of the segment boundaries lands exactly on the join DTS (segments are
    // cut on 3-sample GOP boundaries and the join is at sample 6).
    assert!(
        tfdt_list.contains(&ta_end),
        "a segment tfdt == join DTS {ta_end}; got {tfdt_list:?}"
    );
    tfdt_list.dedup();
    assert!(tfdt_list.windows(2).all(|w| w[1] > w[0]));
}

// ---------------------------------------------------------------------------
// Test 3 — splice_insert SSAI timing.
// ---------------------------------------------------------------------------
#[test]
fn splice_insert_ssai_timing() {
    // Base: 9 video samples @3000, keyframes every 3 (indices 0,3,6). Ad: 4
    // samples @3000, first sync.
    let base = media_of(video_track(1, 0xB0, 9, 3000, 3), 0);
    let ad = media_of(video_track(1, 0xAD, 4, 3000, 4), 0);

    let db = track_span(&base.tracks[0]);
    let da = track_span(&ad.tracks[0]);

    // Splice exactly on a keyframe (sample index 3 → DTS 9000).
    let at = 9000;
    let res = splice_insert(&base, &ad, at).unwrap();
    let out = &res.media.tracks[0];

    // result duration == Db + Da.
    assert_eq!(track_span(out), db + da);
    // total sample count == base + ad.
    assert_eq!(out.samples.len(), 9 + 4);

    // base samples before `at` (indices 0..3) unchanged (bytes + timing).
    let dts = dts_sequence(out);
    for (i, s) in out.samples[..3].iter().enumerate() {
        assert_eq!(s.data, base.tracks[0].samples[i].data);
        assert_eq!(dts[i], (i as u64) * 3000, "base head timing unchanged");
    }
    // ad samples present at indices 3..7, rebased to start at `at`.
    for (j, s) in out.samples[3..7].iter().enumerate() {
        assert_eq!(s.data, ad.tracks[0].samples[j].data);
    }
    assert_eq!(dts[3], at, "ad starts at snapped `at`");
    // base remainder (original indices 3..9) shifted forward by Da.
    for (i, s) in out.samples[7..].iter().enumerate() {
        let orig = i + 3; // original base index
        assert_eq!(s.data, base.tracks[0].samples[orig].data);
        let original_dts = (orig as u64) * 3000;
        assert_eq!(dts[7 + i], original_dts + da, "base tail shifted by Da");
    }
    // Both joins monotonic.
    for w in dts.windows(2) {
        assert!(w[1] >= w[0], "monotonic across both joins");
    }

    // Two discontinuity points: ad-in (index 3) and resume (index 7).
    assert_eq!(res.discontinuity_points.len(), 2);
    assert_eq!(res.discontinuity_points[0].sample_index, 3);
    assert_eq!(res.discontinuity_points[0].presentation_time, at);
    assert_eq!(res.discontinuity_points[1].sample_index, 7);
    assert_eq!(res.discontinuity_points[1].presentation_time, at + da);
}

// ---------------------------------------------------------------------------
// Test 4 — keyframe alignment bites.
// ---------------------------------------------------------------------------
#[test]
fn splice_snaps_to_preceding_keyframe() {
    // Keyframes at indices 0,3,6 → DTS 0, 9000, 18000.
    let base = media_of(video_track(1, 0xB0, 9, 3000, 3), 0);
    let ad = media_of(video_track(1, 0xAD, 2, 3000, 2), 0);

    // Request DTS 12000 (sample index 4, NOT a keyframe). Preceding keyframe is
    // index 3 @ DTS 9000.
    let requested = 12000;
    let (snapped, idx) = snap_to_preceding_sync(&base.tracks[0], requested).unwrap();
    assert_eq!(snapped, 9000, "snapped to preceding keyframe DTS");
    assert_eq!(idx, 3);
    assert!(snapped <= requested, "snap is at or before the request");

    // splice_insert honours the snap: ad opens at 9000, not 12000.
    let res = splice_insert(&base, &ad, requested).unwrap();
    assert_eq!(res.discontinuity_points[0].presentation_time, 9000);
    // The base head kept is exactly 3 samples (indices 0..3).
    assert_eq!(res.discontinuity_points[0].sample_index, 3);

    // An exact-keyframe request is unchanged.
    let (snapped2, idx2) = snap_to_preceding_sync(&base.tracks[0], 18000).unwrap();
    assert_eq!((snapped2, idx2), (18000, 6));

    // An ad whose first sample is NOT sync → Err.
    let mut bad_ad_track = video_track(1, 0xAD, 3, 3000, 3);
    bad_ad_track.samples[0].is_sync = false;
    let bad_ad = media_of(bad_ad_track, 0);
    assert!(
        splice_insert(&base, &bad_ad, 9000).is_err(),
        "non-sync ad first sample must error"
    );
    // concat likewise rejects a non-sync first appended sample.
    assert!(concat(&base, &bad_ad).is_err());
}

// ---------------------------------------------------------------------------
// Test 5 — discontinuity reported + drives the segmenter.
// ---------------------------------------------------------------------------
#[test]
fn discontinuity_points_drive_segmenter() {
    // Base 6 samples, ad 3 samples, splice at keyframe index 3 (DTS 9000).
    // GOP = 3, ad GOP = 3, so segment cuts fall on sample indices 0,3,6,9.
    let base = media_of(video_track(1, 0xB0, 6, 3000, 3), 0);
    let ad = media_of(video_track(1, 0xAD, 3, 3000, 3), 0);
    let res = splice_insert(&base, &ad, 9000).unwrap();
    let joined = &res.media.tracks[0];

    // The join sample indices we must signal as discontinuous.
    let disc_indices: Vec<usize> = res
        .discontinuity_points
        .iter()
        .map(|p| p.sample_index)
        .collect();
    // ad-in at 3, resume at 6.
    assert_eq!(disc_indices, vec![3, 6]);

    // Drive the segmenter. With GOP=3 and a 0.1s (9000-tick) target, the
    // segmenter cuts every 3 samples, so samples [0,1,2]→seg0, [3,4,5]→seg1,
    // [6,7,8]→seg2 — sample index `i` opens segment `i / GOP`. `mark_discontinuity`
    // flags the segment produced by the *next* cut, and that cut fires when the
    // sample opening the *following* segment is pushed. So a join opening segment
    // `s` (first sample index `s * GOP`) is flagged by calling `mark_discontinuity`
    // just before pushing the sample at index `(s + 1) * GOP` (the flush handles
    // the trailing segment). Map each join sample index → the segment it opens.
    const GOP: usize = 3;
    let mut seg = Segmenter::new(vec![joined.spec.clone()], 1000, 0.1).unwrap();
    let mut expected_disc_segments: Vec<usize> = disc_indices.iter().map(|i| i / GOP).collect();
    expected_disc_segments.sort_unstable();
    for (i, s) in joined.samples.iter().enumerate() {
        // Is a *new* segment about to be cut by this push (a keyframe past the
        // target)? That cut closes segment `(i / GOP) - 1`; flag it if that
        // segment opened on a join sample.
        if i > 0 && i % GOP == 0 {
            let closing_seg = (i / GOP) - 1;
            if expected_disc_segments.contains(&closing_seg) {
                seg.mark_discontinuity();
            }
        }
        seg.push(1, s.clone()).unwrap();
    }
    // The final (trailing) segment is closed by flush; flag it if it is a join.
    let trailing_seg = joined.samples.len().div_ceil(GOP) - 1;
    if expected_disc_segments.contains(&trailing_seg) {
        seg.mark_discontinuity();
    }
    seg.flush().unwrap();

    let metas: Vec<bool> = seg
        .take_ready_with_meta()
        .iter()
        .map(|(_bytes, meta)| meta.discontinuous)
        .collect();

    // Exactly the segments opening on a join sample are discontinuous.
    for (seg_idx, &disc) in metas.iter().enumerate() {
        let want = expected_disc_segments.contains(&seg_idx);
        assert_eq!(disc, want, "segment {seg_idx} discontinuity flag");
    }
    // And we did mark exactly two discontinuities (ad-in @3→seg1, resume @6→seg2).
    assert_eq!(expected_disc_segments, vec![1, 2]);
    assert_eq!(metas.iter().filter(|d| **d).count(), 2);
}
