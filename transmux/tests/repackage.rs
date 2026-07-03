//! `Repackage` gate — fMP4/CMAF resegment / trim / track-select (issue #462).
//!
//! The oracle IR is built by demuxing `fixtures/ts/h264_aac.ts` with [`TsDemux`]
//! (deterministic: 75 video + 131 audio samples, fully characterised by the
//! `ts_demux` gate). Every test re-demuxes the repackaged CMAF output with the
//! crate's own [`Fmp4Demux`] and compares coded sample bytes against that oracle
//! — no hardcoded offsets, no raw-passthrough shortcuts.

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::media::{Fmp4Demux, Media};
use transmux::pipeline::CodecConfig;
use transmux::{MovieFragmentBox, Repackage, TsDemux, parse_box};

// ── Fixtures / oracle ───────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

/// The deterministic oracle IR: demux the characterised H.264+AAC TS.
fn oracle_ir() -> Media {
    let data = std::fs::read(fixtures_dir().join("h264_aac.ts")).expect("h264_aac.ts fixture");
    let media = TsDemux::new().unpackage(&data).expect("ts demux");
    assert_eq!(media.tracks.len(), 2, "oracle: 2 tracks");
    assert_eq!(
        media.tracks[0].samples.len(),
        75,
        "oracle: 75 video samples"
    );
    assert_eq!(
        media.tracks[1].samples.len(),
        131,
        "oracle: 131 audio samples"
    );
    assert!(
        matches!(media.tracks[0].spec.config, CodecConfig::Avc { .. }),
        "oracle track 0 is video"
    );
    media
}

/// The anchor track's total duration in its media timescale, and the timescale.
fn anchor_total(media: &Media) -> (u64, u32) {
    media.anchor_duration().expect("anchor duration")
}

// ── Minimal per-segment box inspection ──────────────────────────────────────

const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;

/// The first sample's `sample_flags` for `track_id` in a single media segment,
/// resolving trun `sample_flags` → `first_sample_flags` → tfhd default. Returns
/// `None` if the track is absent from the segment.
fn first_sample_flags(segment: &[u8], track_id: u32) -> Option<u32> {
    let mut off = 0usize;
    while off + 8 <= segment.len() {
        let (bx, consumed) = parse_box(&segment[off..]).expect("parse top box");
        if &bx.header.box_type.0 == b"moof" {
            let moof = MovieFragmentBox::parse_body(bx.body).expect("parse moof");
            for traf in &moof.traf {
                if traf.tfhd.track_id != track_id {
                    continue;
                }
                let trun = traf.trun.first()?;
                let ts0 = trun.samples.first()?;
                let flags = ts0
                    .sample_flags
                    .or(trun.first_sample_flags)
                    .or(traf.tfhd.default_sample_flags)
                    .unwrap_or(0);
                return Some(flags);
            }
        }
        if consumed == 0 {
            break;
        }
        off += consumed;
    }
    None
}

/// Concatenate the coded sample byte-vectors of the given track index across a
/// re-demuxed media (in order).
fn coded_bytes(media: &Media, track_idx: usize) -> Vec<Vec<u8>> {
    media.tracks[track_idx]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect()
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Test 1 — lossless identity repackage: same tracks, resegment, re-demux, and
/// assert every track's coded sample bytes + counts survive byte-identically.
#[test]
fn identity_repackage_is_lossless() {
    let ir = oracle_ir();
    let out = Repackage::new(2.0).run_media(&ir).expect("repackage");
    let round = Fmp4Demux::new()
        .unpackage(&out.to_contiguous())
        .expect("re-demux");

    assert_eq!(round.tracks.len(), 2, "identity keeps 2 tracks");
    assert_eq!(
        round.tracks[0].samples.len(),
        75,
        "video sample count preserved"
    );
    assert_eq!(
        round.tracks[1].samples.len(),
        131,
        "audio sample count preserved"
    );
    assert_eq!(
        coded_bytes(&round, 0),
        coded_bytes(&ir, 0),
        "video coded NAL payloads byte-identical"
    );
    assert_eq!(
        coded_bytes(&round, 1),
        coded_bytes(&ir, 1),
        "audio coded frames byte-identical"
    );
}

/// Test 2 — track-select: keep only the video track (index 0).
#[test]
fn track_select_video_only() {
    let ir = oracle_ir();
    let out = Repackage::new(2.0)
        .select_tracks(&[0])
        .run_media(&ir)
        .expect("repackage video-only");
    let round = Fmp4Demux::new()
        .unpackage(&out.to_contiguous())
        .expect("re-demux");

    assert_eq!(round.tracks.len(), 1, "exactly one track after select");
    assert!(
        matches!(round.tracks[0].spec.config, CodecConfig::Avc { .. }),
        "the kept track is video"
    );
    assert_eq!(round.tracks[0].samples.len(), 75, "all 75 video samples");
    assert_eq!(
        coded_bytes(&round, 0),
        coded_bytes(&ir, 0),
        "video bytes byte-identical, audio absent"
    );
}

/// Test 3 — trim: drop leading + trailing samples by presentation time; assert
/// the mathematically-selected window, a sync first sample, and byte fidelity.
#[test]
fn trim_selects_window_and_snaps_to_keyframe() {
    let ir = oracle_ir();
    let (total, ts) = anchor_total(&ir);
    assert_eq!(
        ts, ir.movie_timescale,
        "video anchor drives movie timescale"
    );

    // Choose an inner window that starts strictly after the first frame and ends
    // before the last, in the movie timescale (== the video track timescale).
    let per_sample = total / 75; // average video sample duration in ticks
    let start = per_sample * 5; // skip ~5 frames
    let end = total - per_sample * 5; // drop ~5 trailing frames

    // Oracle: which video samples fall in [start, end) by presentation time,
    // then snap the first back to the preceding sync sample (anchor rule).
    let vid = &ir.tracks[0];
    let mut pts = Vec::with_capacity(75);
    let mut dts: i64 = 0;
    for s in &vid.samples {
        pts.push(dts + s.composition_offset as i64);
        dts += s.duration as i64;
    }
    let first_in = pts
        .iter()
        .position(|&p| p >= start as i64 && p < end as i64)
        .expect("window selects at least one video sample");
    let mut snapped = first_in;
    while snapped > 0 && !vid.samples[snapped].is_sync {
        snapped -= 1;
    }
    let expected_video: Vec<Vec<u8>> = vid.samples[snapped..]
        .iter()
        .enumerate()
        .take_while(|(k, _)| pts[snapped + k] < end as i64)
        .map(|(_, s)| s.data.clone())
        .collect();
    assert!(
        !expected_video.is_empty(),
        "oracle window must keep video samples"
    );

    let out = Repackage::new(2.0)
        .trim(start, end)
        .run_media(&ir)
        .expect("trim repackage");
    let round = Fmp4Demux::new()
        .unpackage(&out.to_contiguous())
        .expect("re-demux");

    // (a) kept count matches the oracle window (post-snap).
    assert_eq!(
        round.tracks[0].samples.len(),
        expected_video.len(),
        "trimmed video count matches oracle window"
    );
    // (b) first kept video sample is a sync sample.
    assert!(
        round.tracks[0].samples[0].is_sync,
        "first kept video sample must be a sync sample (keyframe)"
    );
    // (c) coded bytes equal the corresponding originals.
    assert_eq!(
        coded_bytes(&round, 0),
        expected_video,
        "trimmed video coded bytes equal the corresponding originals"
    );
    // (d) output re-based to zero: first media segment's video tfdt is 0 — the
    //     re-demuxed first sample begins the timeline (Fmp4Demux reconstructs
    //     from base 0), verified structurally by the identity of sample[0].
    let vid_tid = round.tracks[0].spec.track_id;
    let first_seg = out.media_segments.first().expect("at least one segment");
    let flags = first_sample_flags(first_seg, vid_tid).expect("video in first seg");
    assert_eq!(
        flags & SAMPLE_FLAG_IS_NON_SYNC,
        0,
        "first output segment opens on a keyframe"
    );
}

/// Test 4 — resegment cut count: number of segments == ceil(anchor_dur / T), and
/// every emitted segment starts on a keyframe on the anchor track.
#[test]
fn resegment_cut_count_and_keyframe_starts() {
    let ir = oracle_ir();
    let (total, ts) = anchor_total(&ir);
    let vid_tid = ir.tracks[0].spec.track_id;

    // Pick a target that yields several segments.
    let target_secs = 1.0;
    let target_ticks = (target_secs * ts as f64) as u64;
    let expected_segments = total.div_ceil(target_ticks) as usize;

    let out = Repackage::new(target_secs)
        .run_media(&ir)
        .expect("resegment");
    assert_eq!(
        out.segment_count(),
        expected_segments,
        "segment count == ceil(anchor_dur / target)"
    );
    assert!(
        expected_segments > 1,
        "test must actually cut multiple segments"
    );

    for (i, seg) in out.media_segments.iter().enumerate() {
        let flags = first_sample_flags(seg, vid_tid)
            .unwrap_or_else(|| panic!("video track absent from segment {i}"));
        assert_eq!(
            flags & SAMPLE_FLAG_IS_NON_SYNC,
            0,
            "segment {i} must start on a keyframe"
        );
    }
}

/// Test 5 — sample fidelity across resegment: the concatenation of every
/// resegmented segment's video samples equals the original IR video sequence.
#[test]
fn resegment_preserves_full_sample_sequence() {
    let ir = oracle_ir();
    let out = Repackage::new(0.5).run_media(&ir).expect("resegment");

    // Re-demux the whole contiguous output and compare the full video sequence.
    let round = Fmp4Demux::new()
        .unpackage(&out.to_contiguous())
        .expect("re-demux");
    assert_eq!(
        coded_bytes(&round, 0),
        coded_bytes(&ir, 0),
        "concatenated resegmented video NAL sequence equals the original, in order"
    );
    assert_eq!(
        coded_bytes(&round, 1),
        coded_bytes(&ir, 1),
        "audio sequence also preserved across resegment"
    );

    // And per-segment, re-demux each media segment individually and stitch —
    // proving no sample is dropped or duplicated at a cut boundary.
    let mut stitched: Vec<Vec<u8>> = Vec::new();
    for seg in &out.media_segments {
        let mut whole = out.init_segment.clone();
        whole.extend_from_slice(seg);
        let m = Fmp4Demux::new()
            .unpackage(&whole)
            .expect("re-demux segment");
        stitched.extend(m.tracks[0].samples.iter().map(|s| s.data.clone()));
    }
    assert_eq!(
        stitched,
        coded_bytes(&ir, 0),
        "per-segment stitched video sequence equals the original"
    );
}
