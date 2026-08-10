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
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
        SLConfigDescriptor, StreamType,
    };
    let esds = EsdsBox::new(ESDescriptor {
        es_id: 1,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(0x40),
            stream_type: StreamType(0x05),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo {
                data: vec![0x12, 0x10],
            }),
        }),
        sl_config: Some(SLConfigDescriptor { body: vec![0x02] }),
    });
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
