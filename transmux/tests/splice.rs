//! Integration tests for the IR-level timeline splice / concat → SSAI transforms
//! (issue #475): contiguity + byte preservation, end-to-end `tfdt` monotonicity
//! through the muxer, SSAI insert timing, keyframe-alignment snapping,
//! discontinuity reporting driving the segmenter, and — a genuine per-fragment
//! `tfdt`-reseed gap between `Track::start_decode_time` and a sample's true
//! absolute `dts` (media plane step 2c) — that `concat`'s join position and
//! `snap_to_preceding_sync`'s snap read that true `dts` rather than
//! reconstructing decode time as `start_decode_time + Σ duration`.
//!
//! The `#782` block below tests the rebase-onto-the-join fix specifically:
//! unlike the tests above (which check `dts_sequence`, a reconstruction from
//! `start_decode_time + Σ duration` that literally cannot observe whether a
//! spliced-in sample's own absolute `dts` was rebased), every `#782` test
//! reads `Sample::dts`/`Sample::pts` directly.
//!
//! PROVENANCE: the headline `#782` case is exercised **both** ways. The
//! real-fixture test (`splice_insert_rebases_real_fixture_onto_the_join`)
//! runs on the committed `fixtures/ts/h264_aac.ts` capture — real H.264 +
//! AAC, real keyframes, and genuinely different track timescales (90 000
//! video / 44 100 audio) — reusing the exact base/ad split that
//! `examples/ssai_ad_stitch.rs` (issue #664) performs, so the mixed-timescale
//! and SSAI-continuity claims rest on real demuxed data rather than on
//! hand-chosen numbers. The remaining `#782` tests are synthetic
//! (`video_track`/`data_track`/`media_of*`), because no committed capture
//! isolates a *deliberately unrelated* pair of absolute anchors (Test 1's
//! 10-hour offset), a controlled composition-offset pattern, a 4-deep splice
//! chain with non-exact-dividing rescales, or an untimed section-carried
//! track — each needs values chosen to make the property observable.

use broadcast_common::Unpackage;
use transmux::pipeline::DataCarriage;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, Media,
    MovieFragmentBox, Sample, Segmenter, Track, TrackSpec, TsDemux, concat, parse_box,
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
    TrackSpec::new(
        track_id,
        TIMESCALE,
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

/// Build a sample whose `data` bytes are a recognizable pattern (so byte
/// preservation is verifiable). `tag` distinguishes samples across media.
fn sample(tag: u8, index: usize, duration: u32, is_sync: bool) -> Sample {
    // Absolute dts/pts on the sample's own uniform grid (media plane step 2c):
    // sample `index` sits at `index * duration`.
    let dts = index as i64 * i64::from(duration);
    Sample::new(
        vec![tag, index as u8, 0xAB, 0xCD],
        Some(dts),
        Some(dts),
        Some(duration),
        is_sync,
    )
}

/// A video track: first sample is a sync sample (keyframe), then `sync_period`
/// samples per GOP.
fn video_track(track_id: u32, tag: u8, count: usize, dur: u32, sync_period: usize) -> Track {
    let samples = (0..count)
        .map(|i| sample(tag, i, dur, i % sync_period == 0))
        .collect();
    Track::new(avc_spec(track_id), samples)
}

fn media_of(mut track: Track, start_decode_time: u64) -> Media {
    // `sample()` builds each sample's absolute dts/pts on a zero-based grid
    // (`index * duration`) independent of the track's intended start — shift
    // every sample by `start_decode_time` here so `Sample::dts`/`Sample::pts`
    // stay consistent with `Track::start_decode_time` (the media plane step
    // 2c invariant that `track_end_decode_time`/`snap_to_preceding_sync`
    // in `splice.rs` now rely on, having moved off reconstructing decode
    // time as `start_decode_time + Σ duration`).
    let delta = start_decode_time as i64;
    for s in &mut track.samples {
        if let Some(d) = s.dts {
            s.dts = Some(d + delta);
        }
        if let Some(p) = s.pts {
            s.pts = Some(p + delta);
        }
    }
    Media::new(vec![track.with_start_decode_time(start_decode_time)], 1000)
}

/// A video track whose samples sit on an absolute dts grid starting at
/// `start_dts`, and whose `start_decode_time` is that same value — the media
/// plane step-2c invariant, expressed for a track that does not begin at 0.
fn video_track_at(
    track_id: u32,
    tag: u8,
    count: usize,
    dur: u32,
    sync_period: usize,
    start_dts: i64,
) -> Track {
    let samples = (0..count)
        .map(|i| {
            let dts = start_dts + i as i64 * i64::from(dur);
            Sample::new(
                vec![tag, i as u8, 0xAB, 0xCD],
                Some(dts),
                Some(dts),
                Some(dur),
                i % sync_period == 0,
            )
        })
        .collect();
    Track::new_at(avc_spec(track_id), samples, start_dts.max(0) as u64)
}

/// A [`CodecConfig::Data`] track on an absolute dts grid starting at
/// `start_dts`, with `start_decode_time` equal to it.
fn data_track_at(
    track_id: u32,
    timescale: u32,
    tag: u8,
    count: usize,
    dur: u32,
    start_dts: i64,
    carriage: DataCarriage,
) -> Track {
    let samples = (0..count)
        .map(|i| {
            let dts = start_dts + i as i64 * i64::from(dur);
            Sample::new(vec![tag, i as u8], Some(dts), Some(dts), Some(dur), true)
        })
        .collect();
    Track::new_at(
        TrackSpec::new(
            track_id,
            timescale,
            CodecConfig::Data {
                stream_type: 0x86,
                descriptors: Vec::new(),
                carriage,
            },
        ),
        samples,
        start_dts.max(0) as u64,
    )
}

/// A generic non-video track built on [`CodecConfig::Data`] — a stand-in for
/// an audio (or, when `timed = false`, section-carried) track. `splice.rs`'s
/// rebase logic only keys off codec-*kind* identity (video vs. not) and
/// timescale, never the concrete codec, so `Data` exercises the same code
/// paths without a full AAC `esds`/`OpusSpecificBox` fixture.
///
/// `timed = true` builds samples with real, evenly-spaced `dts`/`pts` (like
/// [`sample`], zero-based); `timed = false` builds section-carried samples
/// (`dts`/`pts`/`duration` all `None`, ISO/IEC 13818-1 §2.4.4 PSI/private
/// sections — SCTE-35/DSM-CC), matching how `Fmp4Demux`/`TsDemux` actually
/// populate one in this crate. Every sample is a sync sample (real audio is
/// always sync; a section has no such distinction).
fn data_track(
    track_id: u32,
    timescale: u32,
    tag: u8,
    count: usize,
    dur: u32,
    carriage: DataCarriage,
    timed: bool,
) -> Track {
    let samples = (0..count)
        .map(|i| {
            if timed {
                let dts = i as i64 * i64::from(dur);
                Sample::new(vec![tag, i as u8], Some(dts), Some(dts), Some(dur), true)
            } else {
                Sample::new(vec![tag, i as u8], None, None, None, true)
            }
        })
        .collect();
    Track::new(
        TrackSpec::new(
            track_id,
            timescale,
            CodecConfig::Data {
                stream_type: 0x86, // SCTE-35 (ISO/IEC 13818-1 Table 2-34) — arbitrary; only used as a non-video codec-kind stand-in here
                descriptors: Vec::new(),
                carriage,
            },
        ),
        samples,
    )
}

/// A two-track `Media` (e.g. video + a second matched track), each shifted to
/// its own `start_decode_time` — the multi-track analogue of [`media_of`].
fn media_of_two(video: Track, video_start: u64, other: Track, other_start: u64) -> Media {
    fn shifted(mut t: Track, start: u64) -> Track {
        let delta = start as i64;
        for s in &mut t.samples {
            if let Some(d) = s.dts {
                s.dts = Some(d + delta);
            }
            if let Some(p) = s.pts {
                s.pts = Some(p + delta);
            }
        }
        t.with_start_decode_time(start)
    }
    Media::new(
        vec![shifted(video, video_start), shifted(other, other_start)],
        1000,
    )
}

fn track_span(track: &Track) -> u64 {
    track
        .samples
        .iter()
        .map(|s| s.duration.unwrap_or(0) as u64)
        .sum()
}

/// Reconstruct each sample's DTS from a track (start + running sum).
fn dts_sequence(track: &Track) -> Vec<u64> {
    let mut dts = track.start_decode_time;
    let mut out = Vec::new();
    for s in &track.samples {
        out.push(dts);
        dts += s.duration.unwrap_or(0) as u64;
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
    bad_ad_track.samples[0].flags.is_sync = false;
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

// ---------------------------------------------------------------------------
// Test 6 — a genuine tfdt-reseed gap (F2) must be read from the sample's own
// absolute dts, not reconstructed as start_decode_time + Σ duration.
// ---------------------------------------------------------------------------
#[test]
fn concat_and_snap_honour_a_genuine_tfdt_gap() {
    // `a`: 3 samples, every one a sync sample (sync_period = 1), on what
    // looks like a normal 3000-tick grid — except the LAST sample's true
    // absolute dts jumps ahead by a real 50_000-tick gap, exactly as a
    // per-fragment `tfdt` reseed legitimately produces
    // (`media.rs::absorb_fragment`; `Track::start_decode_time` only ever
    // records the *first* fragment's anchor). Naive reconstruction
    // (`start_decode_time + Σ duration`) would place the track's end at
    // 3 * 3000 = 9000 and the last sample at 6000 — both wrong once the gap
    // is real.
    let mut a_track = video_track(1, 0xA0, 3, 3000, 1);
    let gap: i64 = 50_000;
    let last = a_track.samples.len() - 1;
    a_track.samples[last].dts = Some(a_track.samples[last].dts.unwrap() + gap);
    a_track.samples[last].pts = Some(a_track.samples[last].pts.unwrap() + gap);
    let a = Media::new(vec![a_track], 1000);
    let true_last_dts = a.tracks[0].samples[last].dts.unwrap() as u64;
    let true_end = true_last_dts + a.tracks[0].samples[last].duration.unwrap() as u64;
    assert_eq!(true_last_dts, 56_000);
    assert_eq!(true_end, 59_000);

    // concat: the join must land at the true post-gap end, not the naive
    // reconstruction (9000).
    let b = media_of(video_track(1, 0xB0, 2, 3000, 2), 0);
    let res = concat(&a, &b).unwrap();
    assert_eq!(
        res.discontinuity_points[0].presentation_time, true_end,
        "join must use the gapped sample's true absolute dts, not \
         start_decode_time + Σ duration"
    );

    // snap_to_preceding_sync: a request that falls inside the gapped
    // sample's span must snap to ITS true dts (56_000), not the naive
    // reconstructed value (6000) a duration-sum walk would have produced.
    let (snapped, idx) = snap_to_preceding_sync(&a.tracks[0], 57_000).unwrap();
    assert_eq!(
        (snapped, idx),
        (56_000, last),
        "snap must read the gapped sample's true absolute dts"
    );
}

// ---------------------------------------------------------------------------
// #782 Test 1 (the headline) — two independently-anchored assets (one at 0,
// one 10 hours in) must be rebased onto the join: real per-sample `dts` is
// strictly monotonic across it, and each side's own internal spacing is
// unchanged. Fails before the fix: the unfixed code appended `b`'s samples
// with their own-file absolute `dts`/`pts` untouched, so `b`'s first real
// `dts` (near 0) would land immediately after `a`'s real end `dts` (10
// hours+), producing a multi-hour BACKWARD jump — non-monotonic.
// ---------------------------------------------------------------------------
#[test]
fn concat_rebases_unrelated_timelines_onto_the_join() {
    // a: a "programme" 10 hours into its own broadcast day (90 kHz ticks).
    let ten_hours_ticks: u64 = 10 * 3600 * u64::from(TIMESCALE);
    let a = media_of(video_track(1, 0xA0, 5, 3000, 5), ten_hours_ticks);
    // b: an independently-demuxed asset anchored at 0 — its own file's
    // clock, unrelated to `a`'s.
    let b = media_of(video_track(1, 0xB0, 4, 3000, 4), 0);

    let a_last = a.tracks[0].samples.last().unwrap();
    let a_end = a_last.dts.unwrap() + a_last.duration.unwrap() as i64;

    let res = concat(&a, &b).unwrap();
    let out = &res.media.tracks[0];

    // Real per-sample dts — NOT `dts_sequence`'s `start_decode_time + Σ
    // duration` reconstruction, which cannot see this bug at all (it never
    // reads `Sample::dts`).
    let real_dts: Vec<i64> = out.samples.iter().map(|s| s.dts.unwrap()).collect();

    for w in real_dts.windows(2) {
        assert!(
            w[1] >= w[0],
            "real dts must be monotonic across the join, got {w:?}"
        );
    }
    assert_eq!(
        real_dts[5], a_end,
        "b's first real dts must meet a's real end"
    );
    for w in real_dts[5..].windows(2) {
        assert_eq!(
            w[1] - w[0],
            3000,
            "b's internal spacing is unchanged by the rebase"
        );
    }
}

// ---------------------------------------------------------------------------
// #782 Test 2 — every spliced sample's composition offset (`pts - dts`) is
// byte-identical before and after the splice.
// ---------------------------------------------------------------------------
#[test]
fn concat_preserves_composition_offsets_exactly() {
    let mut a = media_of(video_track(1, 0xA0, 3, 3000, 3), 0);
    let mut b = media_of(video_track(1, 0xB0, 4, 3000, 4), 777_777);

    // Varied, nonzero (including negative) composition offsets, as a B-frame
    // reorder pattern would produce.
    let offsets = [0i64, 500, -250, 1200];
    for (i, s) in a.tracks[0].samples.iter_mut().enumerate() {
        s.pts = Some(s.dts.unwrap() + offsets[i % offsets.len()]);
    }
    for (i, s) in b.tracks[0].samples.iter_mut().enumerate() {
        s.pts = Some(s.dts.unwrap() + offsets[i % offsets.len()]);
    }
    let b_offsets_before: Vec<i64> = b.tracks[0]
        .samples
        .iter()
        .map(|s| s.composition_offset() as i64)
        .collect();

    let res = concat(&a, &b).unwrap();
    let out = &res.media.tracks[0];
    let k = a.tracks[0].samples.len();

    for (j, off_before) in b_offsets_before.iter().enumerate() {
        assert_eq!(
            out.samples[k + j].composition_offset() as i64,
            *off_before,
            "composition offset unchanged by the rebase for b[{j}]"
        );
    }
    // a's own samples are untouched entirely (both dts and pts).
    for i in 0..k {
        assert_eq!(out.samples[i].dts, a.tracks[0].samples[i].dts);
        assert_eq!(out.samples[i].pts, a.tracks[0].samples[i].pts);
    }
}

// ---------------------------------------------------------------------------
// #782 Test 3 — SSAI round trip (programme → ad → programme) is ONE
// continuous timeline on real sample stamps, not on `start_decode_time` /
// `dts_sequence`. `splice_insert` performs both joins (ad-in + resume) in a
// single call, so this exercises the full path end to end. Also fails before
// the fix, for the same reason as Test 1.
// ---------------------------------------------------------------------------
#[test]
fn splice_insert_ssai_round_trip_is_continuous_on_real_stamps() {
    // Base "programme": 9 samples @3000, anchored normally at 0. Ad: 4
    // samples @3000, anchored at a wildly unrelated 5,000,000 ticks — an ad
    // file demuxed with its own independent clock.
    let base = media_of(video_track(1, 0xB0, 9, 3000, 3), 0);
    let ad = media_of(video_track(1, 0xAD, 4, 3000, 4), 5_000_000);

    let at = 9000; // sample index 3, a keyframe
    let res = splice_insert(&base, &ad, at).unwrap();
    let out = &res.media.tracks[0];
    let real_dts: Vec<i64> = out.samples.iter().map(|s| s.dts.unwrap()).collect();

    // Base head (indices 0..3) unchanged.
    for (i, &d) in real_dts[..3].iter().enumerate() {
        assert_eq!(d, (i as i64) * 3000, "base head unchanged");
    }
    // Ad (indices 3..7) opens exactly at the splice point, regardless of its
    // own file's unrelated 5,000,000-tick anchor.
    assert_eq!(
        real_dts[3], at as i64,
        "ad opens exactly at the splice point"
    );
    for w in real_dts[3..7].windows(2) {
        assert_eq!(w[1] - w[0], 3000, "ad's internal spacing preserved");
    }
    // Base tail (indices 7..9) resumes exactly where the (rebased) ad ends —
    // one continuous timeline.
    let ad_end = real_dts[6] + 3000;
    assert_eq!(
        real_dts[7], ad_end,
        "base resumes exactly where the ad ends"
    );
    for w in real_dts[7..].windows(2) {
        assert_eq!(w[1] - w[0], 3000, "base tail spacing unchanged");
    }
    for w in real_dts.windows(2) {
        assert!(w[1] >= w[0], "real dts monotonic across both joins: {w:?}");
    }
}

// ---------------------------------------------------------------------------
// #782 Test 4 — mixed timescales (a 48 kHz non-video track alongside 90 kHz
// video) keep A/V alignment: the single reference-derived offset (from the
// video track) is rescaled into the other track's own timescale, rather than
// computed independently for it (which would silently re-align them).
// ---------------------------------------------------------------------------
#[test]
fn concat_preserves_av_alignment_across_mixed_timescales() {
    const AUDIO_TS: u32 = 48_000;

    // a: video ends at exactly 15_000 (90kHz) — a multiple of 15, so the
    // 90kHz→48kHz conversion (ratio 8/15) below is exact and this test's own
    // arithmetic is unambiguous; #782 Test 5 covers the non-exact/rounding
    // case deliberately.
    let a_video = video_track(1, 0xA0, 5, 3000, 5);
    let a_audio = data_track(2, AUDIO_TS, 0xA1, 5, 1000, DataCarriage::Pes, true);
    let a = media_of_two(a_video, 0, a_audio, 0);

    // b: an independently-demuxed asset anchored at 0 for video, with its
    // matched track pre-rolled 500 (48kHz) ticks ahead of video — a
    // realistic small A/V lead that must survive the splice unchanged.
    let b_video = video_track(1, 0xB0, 3, 3000, 3);
    let b_audio = data_track(2, AUDIO_TS, 0xB1, 3, 1000, DataCarriage::Pes, true);
    let b = media_of_two(b_video, 0, b_audio, 500);

    let res = concat(&a, &b).unwrap();
    let video_out = res
        .media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == 1)
        .unwrap();
    let audio_out = res
        .media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == 2)
        .unwrap();

    let video_join_index = 5; // a_video had 5 samples
    let audio_join_index = 5; // a_audio had 5 samples

    let video_join_dts = video_out.samples[video_join_index].dts.unwrap();
    let audio_join_dts = audio_out.samples[audio_join_index].dts.unwrap();

    assert_eq!(
        video_join_dts, 15_000,
        "video (reference track) join is exact"
    );

    // Audio's join = video's join rescaled 90kHz→48kHz (8/15, exact here)
    // plus b's own 500-tick internal lead — the SAME single offset,
    // expressed in audio ticks, not an independently re-derived alignment.
    let expected_audio_join = 15_000i64 * 48_000 / 90_000 + 500;
    assert_eq!(
        audio_join_dts, expected_audio_join,
        "audio join uses the reference-derived offset rescaled into its own timescale"
    );
}

/// The single-splice offset decision #782-3 mandates: rescale the two
/// *absolute* reference-track endpoints (the join point and the incoming
/// asset's own start) into the target track's timescale separately, then
/// subtract — never rescale an already-computed delta (which is what would
/// let rounding compound across repeated splices). Reimplemented here (not
/// calling the crate's private `rescale_ticks`) so this test encodes the
/// documented contract, not the implementation.
fn expected_offset_48k(join_dts_ref_90k: i64, incoming_ref_start_90k: i64) -> i64 {
    let rescale = |t: i64| (t as i128 * 48_000i128 / 90_000i128) as i64;
    rescale(join_dts_ref_90k) - rescale(incoming_ref_start_90k)
}

// ---------------------------------------------------------------------------
// #782 Test 5 — repeated splices (4 in a row) do not accumulate rounding
// drift. Deliberately non-exact-dividing deltas (90kHz/48kHz is only exact on
// multiples of 15) force a fractional-tick rounding on every single splice;
// each join is checked against a FRESH per-step computation (never a
// carried-forward running total), so a regression that reused a previous
// splice's already-rounded delta (compounding error with each join) would
// diverge from this and fail — most likely within the first couple of steps.
// ---------------------------------------------------------------------------
#[test]
fn concat_repeated_does_not_accumulate_rounding_drift() {
    const AUDIO_TS: u32 = 48_000;

    let mut media = media_of_two(
        video_track(1, 0xA0, 4, 3001, 4),
        0,
        data_track(2, AUDIO_TS, 0xA1, 4, 1601, DataCarriage::Pes, true),
        0,
    );

    for step in 0u8..4 {
        let video_before_len = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == 1)
            .unwrap()
            .samples
            .len();
        let audio_before_len = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == 2)
            .unwrap()
            .samples
            .len();
        let video_before_last = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == 1)
            .unwrap()
            .samples
            .last()
            .unwrap()
            .clone();
        let video_join_dts_exact =
            video_before_last.dts.unwrap() + video_before_last.duration.unwrap() as i64;

        // Each increment is its own small, unrelated, independently-anchored
        // asset — never derived from the growing result so far. Anchors are
        // deliberately not multiples of 15, to force rounding at every step.
        let increment_video_start: i64 = 11 + step as i64;
        let increment_audio_start: i64 = 5 + step as i64;
        let increment = media_of_two(
            video_track(1, 0x10 + step, 3, 3001, 3),
            increment_video_start as u64,
            data_track(2, AUDIO_TS, 0x20 + step, 3, 1601, DataCarriage::Pes, true),
            increment_audio_start as u64,
        );

        media = concat(&media, &increment).unwrap().media;

        let video_out = media.tracks.iter().find(|t| t.spec.track_id == 1).unwrap();
        let audio_out = media.tracks.iter().find(|t| t.spec.track_id == 2).unwrap();

        let video_join_dts = video_out.samples[video_before_len].dts.unwrap();
        assert_eq!(
            video_join_dts, video_join_dts_exact,
            "step {step}: reference track join is always exact (same timescale, no rescale)"
        );

        let audio_join_dts = audio_out.samples[audio_before_len].dts.unwrap();
        let offset = expected_offset_48k(video_join_dts_exact, increment_video_start);
        let expected_audio_join = increment_audio_start + offset;
        assert_eq!(
            audio_join_dts, expected_audio_join,
            "step {step}: audio join must match a FRESH per-step rescale, not a \
             carried-forward (and compounding) delta"
        );
    }
}

// ---------------------------------------------------------------------------
// #782 Test 6 — a section-carried (`dts: None`) sample survives a splice
// with its timestamp still `None` — never fabricated — and does not panic,
// whether or not a reference track exists elsewhere in the same media.
// ---------------------------------------------------------------------------
#[test]
fn concat_leaves_untimed_section_samples_as_none() {
    // Pure section-only media: nothing anywhere carries a timestamp, so
    // there is no reference track to derive an offset from — nothing to
    // rebase, nothing to fabricate.
    let a = Media::new(
        vec![data_track(
            1,
            90_000,
            0xA0,
            2,
            0,
            DataCarriage::Sections,
            false,
        )],
        1000,
    );
    let b = Media::new(
        vec![data_track(
            1,
            90_000,
            0xB0,
            2,
            0,
            DataCarriage::Sections,
            false,
        )],
        1000,
    );
    let res = concat(&a, &b).unwrap();
    assert_eq!(res.media.tracks[0].samples.len(), 4);
    for s in &res.media.tracks[0].samples {
        assert!(
            s.dts.is_none() && s.pts.is_none(),
            "section-carried sample must stay untimed, never fabricated"
        );
    }

    // A realistic mixed case: a timed video track (the reference) alongside
    // an untimed section-carried track (e.g. an SCTE-35 PID), on unrelated
    // timelines. The video track rebases normally; the section track has
    // nothing to rebase and must not panic or fabricate a timestamp even
    // though a valid reference track exists elsewhere in the same media.
    let a2 = media_of_two(
        video_track(1, 0xA1, 3, 3000, 3),
        0,
        data_track(2, 90_000, 0xA2, 2, 0, DataCarriage::Sections, false),
        0,
    );
    let b2 = media_of_two(
        video_track(1, 0xB1, 2, 3000, 2),
        9_000_000,
        data_track(2, 90_000, 0xB2, 2, 0, DataCarriage::Sections, false),
        0,
    );
    let res2 = concat(&a2, &b2).unwrap();
    let video_out = res2
        .media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == 1)
        .unwrap();
    let data_out = res2
        .media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == 2)
        .unwrap();
    assert_eq!(video_out.samples.len(), 5);
    assert!(
        video_out.samples.iter().all(|s| s.dts.is_some()),
        "video track stays timed"
    );
    assert_eq!(data_out.samples.len(), 4);
    assert!(
        data_out
            .samples
            .iter()
            .all(|s| s.dts.is_none() && s.pts.is_none()),
        "section track stays untimed even though a reference track exists elsewhere"
    );
}

// ---------------------------------------------------------------------------
// #782 Test 7 (REAL FIXTURE) — the same defect on real demuxed data, with
// real mixed timescales. Reuses the base/ad split that
// `examples/ssai_ad_stitch.rs` (issue #664) performs on the committed
// `fixtures/ts/h264_aac.ts` capture: an `ad` Media whose tracks are anchored
// `start_decode_time: 0` while their samples keep the capture's real absolute
// `dts` — precisely the anchor/stamp disagreement #782 is about, and exactly
// what a demuxed-from-its-own-file ad looks like. The #664 SSAI test asserts
// only on manifest/cue structure and so never observed this; this test reads
// the real sample stamps.
//
// Fails before the fix: the ad's samples kept their original 306 000-tick
// (GOP-3) position instead of being rebased onto the 216 000-tick splice
// boundary, and the resumed base was never pushed past the ad, so the two
// contributions overlapped.
// ---------------------------------------------------------------------------
#[test]
fn splice_insert_rebases_real_fixture_onto_the_join() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let source: Media = TsDemux::new().unpackage(&bytes[..]).expect("TS demux");

    let vi = source
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("fixture has an AVC track");
    let ai = source
        .tracks
        .iter()
        .position(|t| !matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("fixture has an audio track");
    let video = &source.tracks[vi];
    let audio = &source.tracks[ai];

    // The timescales genuinely differ — this is what makes the rescale path
    // real rather than an identity no-op.
    assert_eq!(video.spec.timescale, 90_000, "real video timescale");
    assert_eq!(audio.spec.timescale, 44_100, "real audio timescale");
    assert_ne!(
        video.spec.timescale, audio.spec.timescale,
        "the mixed-timescale path must not degenerate to an identity rescale"
    );

    // The capture's own real keyframes.
    let keyframes: Vec<usize> = video
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.flags.is_sync)
        .map(|(i, _)| i)
        .collect();
    assert!(
        keyframes.len() >= 3,
        "fixture must carry >= 3 real keyframes, found {}",
        keyframes.len()
    );
    let splice_idx = keyframes[1]; // splice here (inside the base clip)
    let content_split = keyframes[2]; // base/ad content boundary

    // Split the audio at the same wall-clock offset as the content split.
    let content_split_ticks: u64 = video.samples[..content_split]
        .iter()
        .map(|s| s.duration.unwrap_or(0) as u64)
        .sum();
    let content_split_secs = content_split_ticks as f64 / f64::from(video.spec.timescale);
    let audio_target = (content_split_secs * f64::from(audio.spec.timescale)).round() as u64;
    let mut acc = 0u64;
    let mut audio_split = audio.samples.len();
    for (i, s) in audio.samples.iter().enumerate() {
        if acc >= audio_target {
            audio_split = i;
            break;
        }
        acc += s.duration.unwrap_or(0) as u64;
    }

    let splice_dts = video.samples[splice_idx].dts.unwrap() as u64;

    let base = Media::new(
        vec![
            Track::new_at(
                video.spec.clone(),
                video.samples[..content_split].to_vec(),
                video.start_decode_time,
            ),
            Track::new_at(
                audio.spec.clone(),
                audio.samples[..audio_split].to_vec(),
                audio.start_decode_time,
            ),
        ],
        source.movie_timescale,
    );
    // The ad: anchored at 0 while its samples keep the capture's real
    // absolute dts — the #782 condition, as `examples/ssai_ad_stitch.rs`
    // builds it.
    let ad_video_first_dts = video.samples[content_split].dts.unwrap();
    let ad = Media::new(
        vec![
            Track::new_at(
                video.spec.clone(),
                video.samples[content_split..].to_vec(),
                0,
            ),
            Track::new_at(audio.spec.clone(), audio.samples[audio_split..].to_vec(), 0),
        ],
        source.movie_timescale,
    );
    assert_ne!(
        ad_video_first_dts as u64, splice_dts,
        "the ad's own real dts must differ from the splice point, or this test proves nothing"
    );

    let res = splice_insert(&base, &ad, splice_dts).expect("splice");

    // Every track — both timescales — must be strictly monotonic on REAL
    // per-sample stamps, and lose no samples.
    let out_video = res
        .media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == video.spec.track_id)
        .unwrap();
    let out_audio = res
        .media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == audio.spec.track_id)
        .unwrap();
    assert_eq!(
        out_video.samples.len(),
        video.samples.len(),
        "no video samples lost"
    );
    assert_eq!(
        out_audio.samples.len(),
        audio.samples.len(),
        "no audio samples lost"
    );

    for t in [out_video, out_audio] {
        let dts: Vec<i64> = t
            .samples
            .iter()
            .map(|s| s.dts.expect("real fixture samples are all timed"))
            .collect();
        for (i, w) in dts.windows(2).enumerate() {
            assert!(
                w[1] >= w[0],
                "track {} real dts must be monotonic; went backwards at sample {i}: {} -> {}",
                t.spec.track_id,
                w[0],
                w[1]
            );
        }
        // The anchor must agree with the first real sample stamp.
        assert_eq!(
            t.start_decode_time,
            u64::try_from(dts[0]).unwrap(),
            "track {} anchor must equal its first sample's dts",
            t.spec.track_id
        );
    }

    // The ad's first video sample now decodes exactly at the splice boundary,
    // regardless of its own 306 000-tick position in the source capture.
    let out_video_dts: Vec<i64> = out_video.samples.iter().map(|s| s.dts.unwrap()).collect();
    assert_eq!(
        out_video_dts[splice_idx], splice_dts as i64,
        "the ad opens exactly at the real splice boundary"
    );

    // ...and the resumed base picks up exactly where the rebased ad ends —
    // one continuous timeline, no gap and no overlap.
    let ad_len = video.samples.len() - content_split;
    let ad_span: u64 = video.samples[content_split..]
        .iter()
        .map(|s| s.duration.unwrap_or(0) as u64)
        .sum();
    assert_eq!(
        out_video_dts[splice_idx + ad_len],
        (splice_dts + ad_span) as i64,
        "the resumed base starts exactly where the rebased ad ends"
    );
}

// ---------------------------------------------------------------------------
// Test — #992: `match_tracks` must never map two `a` tracks onto the same `b`
// track (dual-audio content with only partial `track_id` overlap).
// ---------------------------------------------------------------------------

/// `a` has two "audio" (Data stand-in) tracks: id 7 and id 99. `b` has two:
/// id 1 and id 7. `a`'s id-7 track finds its id match at `b`'s *second* slot
/// (`b[1]`); `a`'s id-99 track has no id match anywhere in `b` and falls back
/// to its own position (`a` index 1) — which is the exact `b` index the id
/// match above just claimed. A `match_tracks` with no "claimed" tracking maps
/// BOTH `a` tracks onto `b[1]` and never references `b[0]` at all — issue
/// #992. With the fix, this must either (a) find a genuinely injective
/// mapping, or (b) fail outright — anything but silently colliding.
#[test]
fn match_tracks_never_collides_on_partial_id_overlap() {
    let a0 = data_track(7, 1000, b'A', 3, 1000, DataCarriage::Pes, true);
    let a1 = data_track(99, 1000, b'B', 3, 1000, DataCarriage::Pes, true);
    let b0 = data_track(1, 1000, b'C', 3, 1000, DataCarriage::Pes, true);
    let b1 = data_track(7, 1000, b'D', 3, 1000, DataCarriage::Pes, true);

    let a = Media::new(vec![a0, a1], 1000);
    let b = Media::new(vec![b0, b1], 1000);

    match concat(&a, &b) {
        // Ambiguous input (a[1] has no real counterpart in b once a[0]
        // claims its only id match) is safely rejected rather than guessed.
        Err(_) => {}
        Ok(res) => {
            // If it *does* succeed, the two output tracks must not have been
            // spliced against the same `b` track — the pre-fix bug fed
            // `b[1]`'s tag (`D`) into both, dropping `b[0]`'s tag (`C`)
            // entirely.
            assert_eq!(res.media.tracks.len(), 2, "both a tracks survive");
            for t in &res.media.tracks {
                let tags: std::collections::BTreeSet<u8> =
                    t.samples.iter().map(|s| s.data[0]).collect();
                assert_eq!(
                    tags.len(),
                    1,
                    "a spliced track must carry one b-side tag, not a mix"
                );
            }
            let tag0 = res.media.tracks[0].samples[0].data[0];
            let tag1 = res.media.tracks[1].samples[0].data[0];
            assert_ne!(
                tag0, tag1,
                "two distinct a tracks must not be spliced against the same b track \
                 (both got b's tag {tag0:#04x})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test — r04-W35: the #992 injectivity search must look for an *unclaimed* id
// match, not take the first id match and fall back to position.
// ---------------------------------------------------------------------------

/// Three same-codec, same-timescale tracks — three audio languages, say — where
/// `b`'s ids are a permutation of `a`'s and the *first* id match for `a[1]` is
/// already claimed.
///
/// `a` ids `[2, 2, 3]`, `b` ids `[2, 3, 2]`:
/// - `a[0]` (id 2) → first id match is `b[0]` ✅ (this is why `a[0]` wins it)
/// - `a[1]` (id 2) → first id match is again `b[0]`, already taken, so the
///   pre-fix code fell back to the *positional* index `b[1]` — whose id is 3.
/// - `a[2]` (id 3) → first id match is `b[1]`, now taken, so it fell back to
///   positional `b[2]` — whose id is 2.
///
/// `a[1]` and `a[2]` therefore swap: id-2 content is spliced against id-3
/// content and vice versa. Because all three tracks share a codec kind and a
/// timescale, nothing in the compatibility check fires, so the languages are
/// silently cross-wired — the failure mode r04-W35 calls out, and one #992's
/// original "…then fall back to position" fix did not close. The correct
/// search takes the first *unclaimed* id match: `a[1]` → `b[2]`, `a[2]` →
/// `b[1]`.
#[test]
fn match_tracks_prefers_unclaimed_id_match_over_positional_fallback() {
    let a_tracks = vec![
        data_track(2, 1000, b'A', 3, 1000, DataCarriage::Pes, true),
        data_track(2, 1000, b'B', 3, 1000, DataCarriage::Pes, true),
        data_track(3, 1000, b'C', 3, 1000, DataCarriage::Pes, true),
    ];
    let b_tracks = vec![
        data_track(2, 1000, b'X', 3, 1000, DataCarriage::Pes, true),
        data_track(3, 1000, b'Y', 3, 1000, DataCarriage::Pes, true),
        data_track(2, 1000, b'Z', 3, 1000, DataCarriage::Pes, true),
    ];
    let a = Media::new(a_tracks, 1000);
    let b = Media::new(b_tracks, 1000);

    let res = concat(&a, &b).expect("an unclaimed id match exists for every a track");

    // Each output track must carry the b-side content of the b track that
    // shares its id: a id 2 -> b ids 2, 2 (slots 0 and 2); a id 3 -> b id 3
    // (slot 1). The pre-fix cross-wiring gives a id 2 the `Y` tag and a id 3
    // the `Z` tag.
    // Each `a` track contributed 3 samples and each `b` track 3, so the second
    // half of every output track is the appended `b` content.
    let tags: Vec<u8> = res
        .media
        .tracks
        .iter()
        .map(|t| {
            let appended: std::collections::BTreeSet<u8> =
                t.samples[3..].iter().map(|s| s.data[0]).collect();
            assert_eq!(
                appended.len(),
                1,
                "the appended half of a track must be one b track's samples"
            );
            appended.into_iter().next().unwrap()
        })
        .collect();
    assert_eq!(tags.len(), 3);
    let (tag_a, tag_b, tag_c) = (tags[0], tags[1], tags[2]);

    assert_eq!(
        (tag_a, tag_b, tag_c),
        (b'X', b'Z', b'Y'),
        "a's id-2 tracks must take b's id-2 tracks (slots 0 and 2) and a's \
         id-3 track b's id-3 track (slot 1); a cross-swap (X, Y, Z) means the \
         positional fallback beat the unclaimed id match"
    );
}

// ---------------------------------------------------------------------------
// Test — r04-W36: the non-video cut position must come from absolute `dts`, not
// from a running sum of `duration`s.
// ---------------------------------------------------------------------------

/// The video track and the second track share a timescale here (1000) purely so
/// the video-relative offset needs no rescaling and the assertion states the
/// bug directly rather than compounding it with rounding.
///
/// The second track has a **timestamp gap**: its samples begin at dts 5000 while
/// `start_decode_time` is 0 — exactly what a `tfdt` reseed or a dropped run of
/// audio produces. `splice_insert` snaps the request to the video track's sync
/// sample at 4000 ticks, a *wall-clock* offset of 4000 from the base start, and
/// the second track must be cut at the first sample at or after 4000 — its
/// sample 0 (dts 5000).
///
/// Summing durations from 0 instead reaches 4000 after four samples and cuts at
/// sample 4 (dts 9000), dropping four samples' worth of audio and moving the cut
/// 4000 ticks later than the video's — the A/V desync r04-W36 describes. The
/// gap must exceed the requested offset for the two to differ, so the base
/// samples are 1000 ticks each and the gap 5000.
#[test]
fn splice_cut_on_a_gapped_track_uses_absolute_dts() {
    // Video: 1000-tick samples on a zero-based grid (dts 0, 1000, 2000, ...),
    // every 4th a sync sample, so the request at 4500 snaps back to 4000.
    let video = video_track(7, b'V', 8, 1000, 4);
    // The matching track: same codec-kind/timescale, but shifted by a real
    // 5000-tick gap and with its own absolute dts grid.
    let mut other = data_track(9, 1000, b'A', 12, 1000, DataCarriage::Pes, true);
    for (i, s) in other.samples.iter_mut().enumerate() {
        let dts = 5000 + i as i64 * 1000;
        s.dts = Some(dts);
        s.pts = Some(dts);
    }
    let base = Media::new(vec![video, other], 1000);

    // Ad: same two tracks, zero-based.
    let ad_video = video_track(7, b'W', 2, 1000, 1);
    let ad_other = data_track(9, 1000, b'B', 2, 1000, DataCarriage::Pes, true);
    let ad = Media::new(vec![ad_video, ad_other], 1000);

    let res = splice_insert(&base, &ad, 4500).expect("splice_insert");

    // The discontinuity point marks the first sample kept after the cut, so its
    // index *is* the cut index: 0 (dts 5000), because 5000 is already at or
    // beyond the 4000-tick offset. The duration-sum version returns 4.
    let point = res
        .discontinuity_points
        .iter()
        .find(|p| p.track_id == 9)
        .expect("a discontinuity point for the spliced track");
    assert_eq!(
        point.sample_index, 0,
        "the second track must be cut at the first sample at or after the          video's 4000-tick offset: its dts 5000 (sample 0), not after four          samples' worth of duration, which would ignore the 5000-tick gap"
    );

    // Nothing was cut from the second track: all its dts are at or beyond the
    // boundary, so the whole base track survives alongside the ad.
    let spliced = res.media.tracks.get(1).expect("the second track survives");
    assert_eq!(
        spliced.samples.iter().filter(|s| s.data[0] == b'A').count(),
        12,
        "every base sample of the second track survives the splice"
    );
    assert!(
        spliced.samples.iter().any(|s| s.data[0] == b'B'),
        "the ad content must be present"
    );
}

// ---------------------------------------------------------------------------
// r04-W36 (review): the cut must be one absolute instant on both tracks
// ---------------------------------------------------------------------------

/// The video cut is an absolute `dts` on the video's own timeline; the
/// non-video cut must be the same *absolute* instant, not the same offset from
/// each track's own `start_decode_time`.
///
/// Here the audio track starts 1 s (90000 ticks at 90 kHz) after the video's
/// `start_decode_time` — a real capture where audio joins late, or a `tfdt`
/// reseed. The requested splice snaps back to the video's sync sample at
/// absolute dts 100000. The audio cut must therefore be the first audio sample
/// whose absolute dts is at or beyond 100000, which is index 10 (dts 100000) —
/// not the first sample at or beyond 100000 *relative to the audio's own later
/// start* (which would be index 100).
#[test]
fn nonvideo_cut_uses_the_absolute_instant_not_the_track_relative_offset() {
    // Video: TIMESCALE (90 kHz) ticks, 1000-tick samples, sync every 4th,
    // starting at 0.
    let video = video_track_at(7, b'V', 400, 1000, 4, 0);
    // Audio (Data stand-in): the same timescale, but its first sample sits
    // 90000 ticks after the video's start (a 1 s late join), and it runs long
    // enough that a track-relative cut would land far away.
    let audio = data_track_at(9, TIMESCALE, b'A', 4000, 1000, 90_000, DataCarriage::Pes);

    let base = Media::new(vec![video, audio], TIMESCALE);
    let ad_video = video_track_at(7, b'W', 2, 1000, 1, 0);
    let ad_audio = data_track_at(9, TIMESCALE, b'B', 2, 1000, 0, DataCarriage::Pes);
    let ad = Media::new(vec![ad_video, ad_audio], TIMESCALE);

    // Request a splice well past the audio's first sample, so a
    // track-relative comparison and an absolute one genuinely diverge: the
    // video snaps to 100000, and the audio's first sample at or after absolute
    // 100000 is index 10 (dts 100000), not index 100 (which is where
    // "100000 ticks past the audio's own 90000 start" would land).
    let res = splice_insert(&base, &ad, 100_000).expect("splice_insert");

    let point = res
        .discontinuity_points
        .iter()
        .find(|p| p.track_id == 9)
        .expect("a splice point for the non-video track");

    // The video cut snaps back to the sync sample at absolute dts 100000
    // (100000 is a sync index with sync_period 4), so the audio cut must be the
    // first audio sample at or after that absolute instant: index 10.
    assert_eq!(
        point.sample_index, 10,
        "the audio cut must be at the absolute instant the video was cut at \
         (100000), not 100000 ticks past the audio's own later start (index 100)"
    );

    // The first ten audio samples (dts 90000..99000) sit before the cut and the
    // rest at or after it, so the splice point is the boundary the audio content
    // is split on — the split marker is at index 10, and the track still carries
    // every sample.
    let spliced = res.media.tracks.get(1).expect("the second track");
    assert_eq!(
        spliced.samples.iter().filter(|s| s.data[0] == b'A').count(),
        4000,
        "a splice point is a boundary, not a deletion: all 4000 base audio          samples survive"
    );
    assert_eq!(
        spliced.samples[point.sample_index].dts,
        Some(100_000),
        "the boundary sample must be the first at the absolute cut instant"
    );
}

// ---------------------------------------------------------------------------
// review round 3: the cut rescale must round up, not truncate
// ---------------------------------------------------------------------------

/// A 90 kHz video track and a 48 kHz audio track, with the video cut landing on
/// a tick that is not a whole audio sample.
///
/// 90001 video ticks is 1.000011… audio seconds at 48 kHz: 90001 × 48000 /
/// 90000 = 48000.53 audio ticks. Flooring gives 48000, which is *before* the
/// instant the video was cut at, so the audio cut lands one sample early — a
/// sample the video still contains is dropped from the audio side, and A/V
/// slips by one audio frame (20.8 µs here, but the same reasoning applies at
/// any rate). Ceiling gives 48001, the first audio sample at or after the
/// video's instant, which is what "cut at the same instant" means.
///
/// The figures are literal: 48000 ticks is exactly 1 s at 48 kHz, so an audio
/// sample grid of 1 tick each makes the index arithmetic obvious.
#[test]
fn cut_rescale_rounds_up_so_audio_is_never_ahead_of_video() {
    // Video: 90 kHz, 1-tick samples, every tick a sync sample so the request
    // snaps to itself.
    let video = video_track_at(7, b'V', 120_000, 1, 1, 0);
    // Audio: 48 kHz, 1-tick samples, starting at absolute 0.
    let audio = data_track_at(9, 48_000, b'A', 120_000, 1, 0, DataCarriage::Pes);
    let base = Media::new(vec![video, audio], 90_000);

    let ad_video = video_track_at(7, b'W', 2, 1, 1, 0);
    let ad_audio = data_track_at(9, 48_000, b'B', 2, 1, 0, DataCarriage::Pes);
    let ad = Media::new(vec![ad_video, ad_audio], 90_000);

    // Ask for a splice at 90001 video ticks: 90001 * 48000 / 90000 = 48000.53.
    let res = splice_insert(&base, &ad, 90_001).expect("splice_insert");
    let point = res
        .discontinuity_points
        .iter()
        .find(|p| p.track_id == 9)
        .expect("a splice point for the audio track");
    assert_eq!(
        point.sample_index, 48_001,
        "the audio cut must be the first sample at or *after* 48000.53 ticks, \
         i.e. 48001; 48000 is before the video's instant"
    );

    // And the same at a sample-perfect instant, where both agree.
    let res = splice_insert(&base, &ad, 90_000).expect("splice_insert");
    let point = res
        .discontinuity_points
        .iter()
        .find(|p| p.track_id == 9)
        .expect("a splice point for the audio track");
    assert_eq!(
        point.sample_index, 48_000,
        "90000 video ticks is exactly 48000"
    );
}

/// A track whose samples carry negative absolute `dts` must not have its cut
/// computed from a wrapped or saturated value.
/// A track whose samples carry negative absolute `dts` must have its cut found
/// by signed comparison, not by wrapping the negative values.
///
/// `Sample::dts` is `i64`, and a rebased timeline puts early samples at negative
/// absolute times. Converting with `as u64` made the very first (negative)
/// sample look enormous, so a cut anywhere in the track reported index 0; the
/// comparison is now in `i128`, where a negative sample is simply "before the
/// cut".
#[test]
fn negative_dts_track_cut_is_not_saturated_to_a_wrong_index() {
    // Video: 90 kHz, samples from 0.
    let video = video_track_at(7, b'V', 100, 1, 1, 0);
    // Audio: 90 kHz, samples from -100 to +99.
    let mut audio = data_track_at(9, 90_000, b'A', 200, 1, 0, DataCarriage::Pes);
    for (i, s) in audio.samples.iter_mut().enumerate() {
        let dts = i as i64 - 100;
        s.dts = Some(dts);
        s.pts = Some(dts);
    }
    audio.start_decode_time = 0;
    let base = Media::new(vec![video, audio], 90_000);

    let ad_video = video_track_at(7, b'W', 1, 1, 1, 0);
    let ad_audio = data_track_at(9, 90_000, b'B', 1, 1, 0, DataCarriage::Pes);
    let ad = Media::new(vec![ad_video, ad_audio], 90_000);

    // Cutting at video tick 0 must land on the first audio sample whose own dts
    // is >= 0 — index 100, since the first hundred are negative. Wrapping the
    // negatives would have reported index 0.
    let res = splice_insert(&base, &ad, 0).expect("splice_insert");
    let point = res
        .discontinuity_points
        .iter()
        .find(|p| p.track_id == 9)
        .expect("a splice point for the audio track");
    assert_eq!(
        point.sample_index, 100,
        "the first audio sample at or after absolute 0 is the one whose dts is          exactly 0 (index 100); the 100 negative samples come first"
    );

    // A cut past every sample reports the sample count, never a wrapped index.
    let res = splice_insert(&base, &ad, 99).expect("splice_insert");
    let point = res
        .discontinuity_points
        .iter()
        .find(|p| p.track_id == 9)
        .expect("a splice point for the audio track");
    assert_eq!(point.sample_index, 199);
}
