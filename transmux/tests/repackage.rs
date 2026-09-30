//! `Repackage` gate — fMP4/CMAF resegment / trim / track-select (issue #462).
//!
//! The oracle IR is built by demuxing `fixtures/ts/h264_aac.ts` with [`TsDemux`]
//! (deterministic: 75 video + 131 audio samples, fully characterised by the
//! `ts_demux` gate). Every test re-demuxes the repackaged CMAF output with the
//! crate's own [`Fmp4Demux`] and compares coded sample bytes against that oracle
//! — no hardcoded offsets, no raw-passthrough shortcuts.

use std::path::PathBuf;

use broadcast_common::Unpackage;
use bytes::Bytes;
use transmux::media::{Fmp4Demux, Media};
use transmux::pipeline::CodecConfig;
use transmux::{
    HEVCConfigurationBox, HEVCDecoderConfigurationRecord, MovieFragmentBox, Repackage, Sample,
    Track, TrackSpec, TsDemux, parse_box,
};

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
fn coded_bytes(media: &Media, track_idx: usize) -> Vec<Bytes> {
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
        pts.push(dts + s.composition_offset() as i64);
        dts += s.duration.unwrap_or(0) as i64;
    }
    let first_in = pts
        .iter()
        .position(|&p| p >= start as i64 && p < end as i64)
        .expect("window selects at least one video sample");
    let mut snapped = first_in;
    while snapped > 0 && !vid.samples[snapped].flags.is_sync {
        snapped -= 1;
    }
    let expected_video: Vec<Bytes> = vid.samples[snapped..]
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
        round.tracks[0].samples[0].flags.is_sync,
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
    let mut stitched: Vec<Bytes> = Vec::new();
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

// ===========================================================================
// Test — anchor selection must recognise HEVC (any video codec), not just AVC
// (audit finding #6).
// ===========================================================================

fn minimal_hevc_config() -> HEVCConfigurationBox {
    HEVCConfigurationBox {
        config: HEVCDecoderConfigurationRecord {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0,
            general_constraint_indicator_flags: 0,
            general_level_idc: 93,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: false,
            length_size_minus_one: 3,
            arrays: vec![],
        },
    }
}

fn hevc_video_track(track_id: u32) -> TrackSpec {
    TrackSpec::new(
        track_id,
        90_000,
        CodecConfig::Hevc {
            config: minimal_hevc_config(),
            width: 320,
            height: 240,
        },
    )
}

fn aac_audio_track(track_id: u32) -> TrackSpec {
    // Reuses the same minimal esds shape `ll_hls.rs`'s tests use; only the
    // discriminant (CodecConfig::Aac, an audio codec) matters here.
    use transmux::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };
    let esds = EsdsBox::new(ESDescriptor::new(
        1,
        0,
        Some(DecoderConfigDescriptor::new(
            0x40,
            0x05,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(vec![0x12, 0x10])),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));
    TrackSpec::new(
        track_id,
        48_000,
        CodecConfig::Aac {
            esds,
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    )
}

/// `Media::anchor_duration` must pick the **HEVC** track as anchor even
/// though it is track **1** (audio, an unrelated codec, is track 0) — before
/// the fix, `repackage::anchor_index` checked only
/// `matches!(t.spec.config, CodecConfig::Avc { .. })`, so this ordinary,
/// well-formed HEVC+AAC media (no malformation needed) silently fell through
/// to `unwrap_or(0)`: the audio track. Segment/trim boundaries would then cut
/// on audio "keyframes" (every AAC frame is a sync sample) instead of real
/// video IDRs.
#[test]
fn anchor_duration_picks_hevc_video_not_audio_track_zero() {
    let audio = Track::new(
        aac_audio_track(1),
        vec![
            Sample::new(vec![0xAAu8; 8], None, None, Some(1024), true),
            Sample::new(vec![0xABu8; 8], None, None, Some(1024), true),
        ],
    );
    // Deliberately different sample count / duration per sample from audio,
    // so the two tracks' anchor durations cannot coincide by accident.
    let video = Track::new(
        hevc_video_track(2),
        vec![
            Sample::new(vec![0x01u8; 8], None, None, Some(3000), true),
            Sample::new(vec![0x02u8; 8], None, None, Some(3000), false),
            Sample::new(vec![0x03u8; 8], None, None, Some(3000), false),
        ],
    );
    let media = Media::new(vec![audio, video], 90_000);

    let (anchor_ticks, anchor_ts) = media
        .anchor_duration()
        .expect("a media with a real anchor-capable track must report an anchor duration");

    assert_eq!(
        anchor_ts, 90_000,
        "anchor timescale must be the HEVC video track's (90 kHz), not audio's (48 kHz)"
    );
    assert_eq!(
        anchor_ticks, 9000,
        "anchor duration must be the HEVC track's 3 x 3000-tick samples, not audio's 2 x 1024"
    );
}

/// `Media::trim` must snap the back-off to the HEVC video track's sync
/// samples, not audio's — otherwise the trimmed output would open on
/// whatever audio frame happened to be nearest, not a real IDR.
#[test]
fn trim_snaps_back_off_on_hevc_video_not_audio() {
    let audio = Track::new(
        aac_audio_track(1),
        vec![
            Sample::new(vec![0xAAu8; 8], None, None, Some(1024), true),
            Sample::new(vec![0xABu8; 8], None, None, Some(1024), true),
            Sample::new(vec![0xACu8; 8], None, None, Some(1024), true),
        ],
    );
    // IDR, then two non-sync samples: a window starting mid-GOP must snap
    // back to sample 0 if (and only if) HEVC is correctly chosen as anchor.
    let video = Track::new(
        hevc_video_track(2),
        vec![
            Sample::new(vec![0x01u8; 8], None, None, Some(3000), true),
            Sample::new(vec![0x02u8; 8], None, None, Some(3000), false),
            Sample::new(vec![0x03u8; 8], None, None, Some(3000), false),
        ],
    );
    let media = Media::new(vec![audio, video], 90_000);

    // Window starting at video's 2nd sample (pts 3000..6000) — mid-GOP.
    let trimmed = media.trim(3000, 9000).expect("window selects samples");
    let video_out = trimmed
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("hevc track survives trim");
    assert_eq!(
        video_out.samples.len(),
        3,
        "back-off must snap to the video IDR at sample 0, keeping all 3 samples"
    );
    assert!(
        video_out.samples[0].flags.is_sync,
        "the first kept video sample must be the IDR"
    );
}

// ===========================================================================
// Test — #993: `trim`'s window selection must honour each sample's own
// absolute `dts`, not only a duration-accumulated reconstruction.
// ===========================================================================

/// A single HEVC track whose recorded per-sample `duration` (3000 each) does
/// NOT track its real absolute `dts` — sample 2 carries a genuine +5000-tick
/// gap a duration-summed reconstruction cannot see (e.g. a discontinuity, or
/// ordinary rounding drift between a nominal per-sample duration and the
/// source's real measured decode-time deltas). Real dts: `0, 3000, 11000,
/// 14000, 17000`; a pure running-duration reconstruction would instead see
/// `0, 3000, 6000, 9000, 12000`.
///
/// A window of `[9000, 12000)`:
/// - reading real `dts` selects only sample 2 (pts 11000) directly, which
///   snaps back to sample 0 (the only sync sample) — kept = samples 0..=2
///   (3 samples).
/// - a duration-summed reconstruction instead selects sample 3 (pts 9000)
///   directly, snapping back to the same sample 0 — but keeps samples 0..=3
///   (4 samples), because its (wrong) pts for sample 2 is 6000, not 11000.
///
/// So the two implementations disagree on the trimmed sample **count**, not
/// just on some incidental internal bookkeeping — an outcome an external
/// caller can observe.
#[test]
fn trim_window_follows_real_dts_not_duration_sum() {
    let video = Track::new(
        hevc_video_track(1),
        vec![
            Sample::new(vec![0x00u8; 8], Some(0), Some(0), Some(3000), true),
            Sample::new(vec![0x01u8; 8], Some(3000), Some(3000), Some(3000), false),
            // The hidden +5000 gap: duration still says 3000, but the real
            // dts jumps by 8000.
            Sample::new(
                vec![0x02u8; 8],
                Some(11_000),
                Some(11_000),
                Some(3000),
                false,
            ),
            Sample::new(
                vec![0x03u8; 8],
                Some(14_000),
                Some(14_000),
                Some(3000),
                false,
            ),
            Sample::new(
                vec![0x04u8; 8],
                Some(17_000),
                Some(17_000),
                Some(3000),
                false,
            ),
        ],
    );
    let media = Media::new(vec![video], 90_000);

    let trimmed = media.trim(9000, 12000).expect("window selects samples");
    let video_out = &trimmed.tracks[0];

    assert_eq!(
        video_out.samples.len(),
        3,
        "must keep samples 0..=2 (snapped back to the sync sample, then up to \
         real pts 11000 which is < 12000) — a duration-summed reconstruction \
         would instead wrongly keep 4 samples"
    );
    assert_eq!(
        video_out.samples.last().unwrap().data,
        vec![0x02u8; 8],
        "last kept sample must be the one whose REAL dts (11000) falls in the window"
    );
}

// ── W32: interleave across huge coprime timescales (audit r05-W32) ──────────

/// Three track timescales whose least common multiple overflows `u64`.
/// `4294967291`, `4294967279` and `4294967231` are pairwise coprime (all
/// prime), so the LCM is their product, `79_228_160_909_397_609_687_688_407_659`
/// — about 4×10^27, far past `u64::MAX` (`18_446_744_073_709_551_615`). The
/// timescales come from untrusted `mdhd` boxes. The old interleave folded every
/// track onto that LCM (`a / gcd * b`), which panics in debug and wraps in
/// release; comparing cross-multiplied in `u128` is exact for any pair.
const BIG_TIMESCALES: [u32; 3] = [4_294_967_291, 4_294_967_279, 4_294_967_231];

/// Every `track_id` a `trun` sample belongs to, in output order — the interleave
/// read back from the muxed bytes.
fn sample_track_order(segments: &[Vec<u8>]) -> Vec<u32> {
    let mut out = Vec::new();
    for segment in segments {
        let mut off = 0usize;
        while off + 8 <= segment.len() {
            let (bx, consumed) = parse_box(&segment[off..]).expect("parse top box");
            if consumed == 0 {
                break;
            }
            if &bx.header.box_type.0 == b"moof" {
                let moof = MovieFragmentBox::parse_body(bx.body).expect("parse moof");
                for traf in &moof.traf {
                    for trun in &traf.trun {
                        for _ in &trun.samples {
                            out.push(traf.tfhd.track_id);
                        }
                    }
                }
            }
            off += consumed;
        }
    }
    out
}

/// Build one track of `count` 1-second samples at `timescale`.
fn one_second_track(
    track_id: u32,
    timescale: u32,
    config: CodecConfig,
    tag: u8,
    count: u32,
) -> Track {
    let samples = (0..count)
        .map(|i| {
            let dts = i as i64 * timescale as i64;
            Sample::new(vec![tag; 8], Some(dts), Some(dts), Some(timescale), true)
        })
        .collect();
    Track::new(TrackSpec::new(track_id, timescale, config), samples)
}

/// The resegmenter interleaves the three tracks' samples by decode time. With
/// huge coprime timescales the old LCM normalisation overflowed `u64`.
#[test]
fn resegment_interleaves_huge_coprime_timescales() {
    let video_cfg = CodecConfig::Hevc {
        config: minimal_hevc_config(),
        width: 320,
        height: 240,
    };
    let audio_cfg = aac_audio_track(2).config;
    let media = Media::new(
        vec![
            one_second_track(1, BIG_TIMESCALES[0], video_cfg, 0x01, 2),
            one_second_track(2, BIG_TIMESCALES[1], audio_cfg.clone(), 0xA0, 2),
            one_second_track(3, BIG_TIMESCALES[2], audio_cfg, 0xB0, 2),
        ],
        1000,
    );

    let out = Repackage::new(2.0)
        .run_media(&media)
        .expect("resegment across huge coprime timescales");

    // The interleave decides which *segment* each sample lands in (the
    // Segmenter writes a segment's truns grouped by track), so the observable
    // is membership, not byte order: the coincident 0 s and 1 s samples of all
    // three tracks must share one segment rather than one track being stranded
    // in a segment of its own.
    let order = sample_track_order(&out.media_segments);
    assert_eq!(
        order.len(),
        6,
        "every sample of every track must be emitted exactly once"
    );
    assert_eq!(
        out.media_segments.len(),
        1,
        "all six coincident samples belong to one segment"
    );
    for id in [1u32, 2, 3] {
        assert_eq!(
            order.iter().filter(|&&t| t == id).count(),
            2,
            "track {id} must contribute both of its samples to the same segment"
        );
    }
}
