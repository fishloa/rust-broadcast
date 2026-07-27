//! Gate tests for the two segmentation **stalls** all four segmenters shared,
//! plus the anchor-selection and batch/streaming-placement bugs found with
//! them.
//!
//! The two stalls have the same shape: a segmenter that cannot make progress
//! buffers without bound while `Stage::demand()` still answers "not
//! saturated", so a well-behaved driver feeds it until memory runs out.
//!
//! 1. **`duration: Some(0)` on the anchor** (C1). The anchor accumulator used
//!    to advance only from `Sample::duration`, so a stream whose samples carry
//!    `Some(0)` never reached the target and no segment was ever cut. This is
//!    not a theoretical input: `StreamingFlvDemux` derives `duration` as the
//!    forward delta between FLV tag timestamps, so the first sample of every
//!    RTMP publish — and any two tags sharing a timestamp — is `Some(0)`.
//!    `Sample::dts` is absolute, so elapsed media time is derivable without
//!    `duration`; `MediaClock` does exactly that.
//!
//! 2. **A single-IDR / infinite-GOP stream** (C2). One keyframe at the start
//!    and none after is legal and routine (screen capture, low-motion
//!    surveillance), and a segment cannot be cut on a non-sync sample without
//!    breaking the format's random-access guarantee. So the pending buffer is
//!    *bounded* instead: at `MAX_PENDING_SAMPLES_PER_TRACK`, `demand()`
//!    reports `saturated` (the load-bearing half — a cooperative driver stops)
//!    and the next push returns a named error.
//!
//! Every test here must **fail before the fix**; the assertions are written so
//! that the pre-fix behaviour (no output at all / unbounded growth /
//! everything in segment 0) trips them.

use std::path::PathBuf;

use broadcast_common::{Package, Stage, Unpackage};
use transmux::ll_dash::LlSegmenter;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::DataCarriage;
use transmux::segmenter::MAX_PENDING_SAMPLES_PER_TRACK;
use transmux::ts_hls::{StreamingTsHlsSegmenter, TsHlsPackager, TsSegment};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, CodecConfig, DecoderConfigDescriptor,
    DecoderSpecificInfo, ESDescriptor, EsdsBox, HEVCConfigurationBox, Media, MovieFragmentBox,
    ObjectTypeIndication, SLConfigDescriptor, Sample, Segmenter, StreamType, Track, TrackSpec,
    TsDemux,
};

// ── Fixtures ────────────────────────────────────────────────────────────────

/// 90 kHz — the MPEG-2 TS system clock (ISO/IEC 13818-1 §2.4.3.7) and the
/// timescale `TsDemux` gives every track, so segment maths here matches the
/// shipped demux path.
const TS_TIMESCALE: u32 = 90_000;
/// 30 fps at 90 kHz.
const FRAME_TICKS: i64 = 3_000;
/// One second of frames at `FRAME_TICKS`.
const FRAMES_PER_SEC: usize = 30;

/// A minimal but structurally-real `avcC` so `build_init_segment` succeeds
/// (same shape as `tests/segmenter.rs`).
fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![transmux::AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

fn dummy_esds() -> EsdsBox {
    EsdsBox::new(ESDescriptor {
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
    })
}

fn avc_track(track_id: u32) -> TrackSpec {
    TrackSpec::new(
        track_id,
        TS_TIMESCALE,
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    )
}

fn aac_track(track_id: u32) -> TrackSpec {
    TrackSpec::new(
        track_id,
        TS_TIMESCALE,
        CodecConfig::Aac {
            esds: dummy_esds(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    )
}

/// A section-carried SCTE-35 track (ISO/IEC 13818-1 Table 2-34 stream_type
/// 0x86): its samples never carry a `duration`, so it can never be the anchor.
fn scte35_track(track_id: u32) -> TrackSpec {
    TrackSpec::new(
        track_id,
        TS_TIMESCALE,
        CodecConfig::Data {
            stream_type: 0x86,
            descriptors: Vec::new(),
            carriage: DataCarriage::Sections,
        },
    )
}

/// The real `hvcC` from `fixtures/ts/hevc/main.ts` — a fabricated one would not
/// decode, and `build_init_segment` needs a genuine parameter set.
fn real_hevc_config() -> (HEVCConfigurationBox, u16, u16) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/hevc/main.ts");
    let ts = std::fs::read(&path).expect("hevc/main.ts fixture must exist");
    let ir = TsDemux::new().unpackage(&ts).expect("demux hevc fixture");
    for t in &ir.tracks {
        if let CodecConfig::Hevc {
            config,
            width,
            height,
        } = &t.spec.config
        {
            return (config.clone(), *width, *height);
        }
    }
    panic!("hevc/main.ts must carry an HEVC track");
}

/// One length-prefixed AVC NAL (4-byte length, ISO/IEC 14496-15 §5.3.4.1.1):
/// an IDR slice (`nal_unit_type` 5) for a sync sample, else a non-IDR slice
/// (type 1). The length prefix has to be real — `ts_mux` walks it to convert
/// the sample to Annex B.
fn nal_payload(sync: bool) -> Vec<u8> {
    let nal_header = if sync { 0x65 } else { 0x41 };
    vec![0x00, 0x00, 0x00, 0x04, nal_header, 0x88, 0x84, 0x00]
}

/// A video access unit at absolute `dts`, deliberately carrying
/// **`duration: Some(0)`** — exactly what an RTMP publish's first tag (and any
/// two same-timestamp tags) produce. Before the fix these advanced no
/// accumulator anywhere.
fn zero_duration_au(dts: i64, sync: bool) -> Sample {
    Sample::new(nal_payload(sync), Some(dts), Some(dts), Some(0), sync)
}

/// A video access unit with a real duration.
fn au(dts: i64, sync: bool) -> Sample {
    Sample::new(
        nal_payload(sync),
        Some(dts),
        Some(dts),
        Some(FRAME_TICKS as u32),
        sync,
    )
}

/// `n` frames at 30 fps, all `duration: Some(0)`, with a sync sample every
/// `FRAMES_PER_SEC` (a 1-second GOP) — the C1 stream.
fn zero_duration_stream(n: usize) -> Vec<Sample> {
    (0..n)
        .map(|i| zero_duration_au(i as i64 * FRAME_TICKS, i % FRAMES_PER_SEC == 0))
        .collect()
}

/// Per-`traf` `(track_id, base_media_decode_time)` of a `moof`-bearing segment.
fn tfdts(segment: &[u8]) -> Vec<(u32, u64)> {
    let mut off = 0usize;
    let moof = loop {
        assert!(off + 8 <= segment.len(), "segment must contain a moof");
        let size = u32::from_be_bytes(segment[off..off + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && off + size <= segment.len(), "malformed box");
        if &segment[off + 4..off + 8] == b"moof" {
            break &segment[off + 8..off + size];
        }
        off += size;
    };
    MovieFragmentBox::parse_body(moof)
        .expect("moof parses")
        .traf
        .iter()
        .map(|t| {
            (
                t.tfhd.track_id,
                t.tfdt
                    .as_ref()
                    .map(|b| b.base_media_decode_time())
                    .unwrap_or(0),
            )
        })
        .collect()
}

// ── C1: a `duration` of Some(0) on the anchor must still cut segments ───────
//
// Pre-fix, every one of the four assertions below saw ZERO segments: the
// accumulator was `+= sample.duration.unwrap_or(0)`, so it never reached the
// target and no cut ever fired, while `pending` grew for the whole stream.
//
// With `MediaClock`, the anchor advances on the dts delta instead. The first
// sample contributes 0 (no previous dts to subtract), so a 1-second target is
// first reached one frame late — segment boundaries therefore fall on the sync
// samples at 60, 90, 120 … rather than 30, 60, 90. That is the arithmetic the
// counts below pin.

/// Frames fed by the C1 tests: 5 seconds at 30 fps.
const C1_FRAMES: usize = 5 * FRAMES_PER_SEC;
/// Segments a 1-second target cuts out of `C1_FRAMES` frames, *excluding* the
/// trailing partial one closed by flush: cuts fire at frames 60, 90, 120.
const C1_CUTS_BEFORE_FLUSH: usize = 3;

#[test]
fn c1_zero_duration_anchor_still_cuts_segments_in_cmaf_segmenter() {
    let mut seg = Segmenter::new(vec![avc_track(1)], TS_TIMESCALE, 1.0).expect("construct");
    for s in zero_duration_stream(C1_FRAMES) {
        seg.push(1, s).expect("push");
    }
    let cut_before_flush = seg.take_ready().len();
    seg.flush().expect("flush");
    let trailing = seg.take_ready();

    assert_eq!(
        cut_before_flush, C1_CUTS_BEFORE_FLUSH,
        "a Some(0)-duration anchor must still cut on its keyframes — before the fix this \
         was 0 and `pending` grew for the whole stream"
    );
    assert_eq!(trailing.len(), 1, "flush closes the trailing segment");
}

#[test]
fn c1_zero_duration_anchor_tfdt_advances_across_segments() {
    // The same stall also pinned every segment's `tfdt` at 0, because
    // `base_decode` was a plain duration sum. Each segment must open later on
    // the media timeline than the one before it.
    let mut seg = Segmenter::new(vec![avc_track(1)], TS_TIMESCALE, 1.0).expect("construct");
    let mut bases = Vec::new();
    for s in zero_duration_stream(C1_FRAMES) {
        seg.push(1, s).expect("push");
        for (bytes, _meta) in seg.take_ready_with_meta() {
            bases.push(tfdts(&bytes)[0].1);
        }
    }
    seg.flush().expect("flush");
    for (bytes, _meta) in seg.take_ready_with_meta() {
        bases.push(tfdts(&bytes)[0].1);
    }

    assert!(bases.len() > 2, "expected several segments, got {bases:?}");
    assert_eq!(bases[0], 0, "the first segment opens at the origin");
    assert!(
        bases.windows(2).all(|w| w[1] > w[0]),
        "each segment's tfdt must advance past the previous one; before the fix every \
         segment reported 0: {bases:?}"
    );
}

#[test]
fn c1_zero_duration_anchor_still_cuts_segments_in_ll_dash() {
    // chunk_samples = 10 so chunking is exercised alongside the segment cuts.
    let mut seg = LlSegmenter::new(vec![avc_track(1)], TS_TIMESCALE, 1.0, 10).expect("construct");
    for s in zero_duration_stream(C1_FRAMES) {
        seg.push(1, s).expect("push");
    }
    seg.flush().expect("flush");
    let starts = seg
        .take_ready()
        .iter()
        .filter(|c| c.is_segment_start)
        .count();
    assert_eq!(
        starts,
        C1_CUTS_BEFORE_FLUSH + 1,
        "each cut opens a new segment (plus the one flush closes) — before the fix the \
         whole stream was one never-closed segment"
    );
}

#[test]
fn c1_zero_duration_anchor_still_cuts_parts_and_segments_in_ll_hls() {
    // 1 s segments, 250 ms parts.
    let mut seg = LlHlsSegmenter::with_part_target(vec![avc_track(1)], TS_TIMESCALE, 1.0, 250)
        .expect("construct");
    for s in zero_duration_stream(C1_FRAMES) {
        seg.push(1, s).expect("push");
    }
    let parts_before_flush = seg.take_ready_parts().len();
    let segments_before_flush = seg.take_ready_segments().len();
    seg.flush().expect("flush");

    assert_eq!(
        segments_before_flush, C1_CUTS_BEFORE_FLUSH,
        "a Some(0)-duration anchor must still close segments — before the fix this was 0"
    );
    assert!(
        parts_before_flush > segments_before_flush,
        "parts must also be emitted (>1 per segment); before the fix no part was emitted \
         either, got {parts_before_flush}"
    );
}

#[test]
fn c1_zero_duration_anchor_still_cuts_segments_in_streaming_ts_hls() {
    let mut seg = StreamingTsHlsSegmenter::new(vec![avc_track(1)], 1, usize::MAX)
        .expect("construct: video is anchor-capable");
    let mut cuts: Vec<TsSegment> = Vec::new();
    for s in zero_duration_stream(C1_FRAMES) {
        // Pre-fix this call *errored* on the first sample: the streaming
        // TS-HLS path rejected `duration: None` outright and only tolerated
        // Some(0) by never advancing.
        if let Some(cut) = seg.push(1, s).expect("push") {
            cuts.push(cut);
        }
    }
    if let Some(tail) = seg.finish().expect("finish") {
        cuts.push(tail);
    }
    assert_eq!(
        cuts.len(),
        C1_CUTS_BEFORE_FLUSH + 1,
        "before the fix this was 1 (the whole stream flushed as one segment)"
    );
    for c in &cuts {
        assert!(c.duration > 0.0, "every cut segment must have a duration");
        assert_eq!(c.bytes.len() % 188, 0, "whole TS packets");
    }
}

// ── C1b: a section-only track set is a construction error, not a stall ──────

#[test]
fn section_only_track_set_is_rejected_by_every_segmenter() {
    let tracks = || vec![scte35_track(10), scte35_track(11)];

    assert!(
        Segmenter::new(tracks(), TS_TIMESCALE, 1.0).is_err(),
        "CMAF Segmenter must refuse a track set with no anchor-capable track"
    );
    assert!(
        LlSegmenter::new(tracks(), TS_TIMESCALE, 1.0, 1).is_err(),
        "LL-DASH LlSegmenter must refuse it too"
    );
    assert!(
        LlHlsSegmenter::with_part_target(tracks(), TS_TIMESCALE, 1.0, 250).is_err(),
        "LL-HLS LlHlsSegmenter must refuse it too"
    );
    assert!(
        StreamingTsHlsSegmenter::new(tracks(), 1, 4).is_err(),
        "streaming TS-HLS already refused it; it must keep doing so"
    );
}

// ── C2: an infinite GOP is bounded, not unbounded ───────────────────────────
//
// One keyframe then nothing but non-sync samples. No cut is possible without
// violating the random-access guarantee, so the segmenter must stop accepting
// input rather than buffer forever. Each test asserts *all three* halves:
// `demand().saturated` flips, the next push errors, and the number accepted is
// exactly the documented bound (so a bound that silently grew would fail too).

/// The C2 stream: sync only at index 0, then `MAX + 8` non-sync frames, all
/// with real durations (so this isolates the no-keyframe stall from C1).
fn infinite_gop_stream() -> Vec<Sample> {
    (0..MAX_PENDING_SAMPLES_PER_TRACK + 8)
        .map(|i| au(i as i64 * FRAME_TICKS, i == 0))
        .collect()
}

/// Drive `push` until it errors, returning `(accepted, saw_saturated)`.
/// `saw_saturated` records whether `demand()` reported saturation *before* the
/// rejecting call — that is the load-bearing half of the fix, since a
/// cooperative driver must be able to stop without ever hitting the error.
fn drive_until_rejected<S, F>(stage: &mut S, n: usize, mut push: F) -> (usize, bool)
where
    S: Stage,
    F: FnMut(&mut S, usize) -> Result<(), transmux::Error>,
{
    let mut accepted = 0usize;
    let mut saw_saturated = false;
    for i in 0..n {
        if Stage::demand(stage).saturated {
            saw_saturated = true;
        }
        match push(stage, i) {
            Ok(()) => accepted += 1,
            Err(_) => return (accepted, saw_saturated),
        }
    }
    panic!("push never rejected: the un-cut buffer is still unbounded");
}

#[test]
fn c2_infinite_gop_is_bounded_in_cmaf_segmenter() {
    let samples = infinite_gop_stream();
    let mut seg = Segmenter::new(vec![avc_track(1)], TS_TIMESCALE, 1.0).expect("construct");
    let (accepted, saw_saturated) = drive_until_rejected(&mut seg, samples.len(), |s, i| {
        s.push(1, samples[i].clone())
    });
    assert_eq!(
        accepted, MAX_PENDING_SAMPLES_PER_TRACK,
        "exactly the documented bound must be buffered before the segmenter refuses"
    );
    assert!(
        saw_saturated,
        "demand() must report saturated before push errors, so a cooperative driver can \
         stop without ever hitting the error"
    );
    // The bound is not a wedge: flush closes the trailing partial segment (a
    // trailing segment is allowed not to start on a keyframe) and input flows
    // again.
    seg.flush().expect("flush after the bound");
    assert_eq!(seg.take_ready().len(), 1);
    assert!(
        !Stage::demand(&seg).saturated,
        "flush clears the saturation"
    );
    seg.push(1, au(999_000, false))
        .expect("accepts input again");
}

#[test]
fn c2_infinite_gop_is_bounded_in_ll_dash() {
    let samples = infinite_gop_stream();
    // chunk_samples larger than the bound, so no chunk drains `pending` and
    // the un-cut buffer is what is actually under test.
    let mut seg = LlSegmenter::new(
        vec![avc_track(1)],
        TS_TIMESCALE,
        1.0,
        MAX_PENDING_SAMPLES_PER_TRACK * 2,
    )
    .expect("construct");
    let (accepted, saw_saturated) = drive_until_rejected(&mut seg, samples.len(), |s, i| {
        s.push(1, samples[i].clone())
    });
    assert_eq!(accepted, MAX_PENDING_SAMPLES_PER_TRACK);
    assert!(saw_saturated, "demand() must flip before push errors");
}

#[test]
fn c2_infinite_gop_is_bounded_in_ll_hls() {
    let samples = infinite_gop_stream();
    let mut seg = LlHlsSegmenter::with_part_target(vec![avc_track(1)], TS_TIMESCALE, 1.0, 250)
        .expect("construct");
    let (accepted, saw_saturated) = drive_until_rejected(&mut seg, samples.len(), |s, i| {
        s.push(1, samples[i].clone())
    });
    assert_eq!(accepted, MAX_PENDING_SAMPLES_PER_TRACK);
    assert!(saw_saturated, "demand() must flip before push errors");
}

#[test]
fn c2_infinite_gop_is_bounded_in_streaming_ts_hls() {
    let samples = infinite_gop_stream();
    let mut seg =
        StreamingTsHlsSegmenter::new(vec![avc_track(1)], 1, usize::MAX).expect("construct");
    let (accepted, saw_saturated) = drive_until_rejected(&mut seg, samples.len(), |s, i| {
        s.push(1, samples[i].clone()).map(|_| ())
    });
    assert_eq!(accepted, MAX_PENDING_SAMPLES_PER_TRACK);
    assert!(saw_saturated, "demand() must flip before push errors");
}

// ── Anchor selection: any video codec, not just AVC ─────────────────────────

/// An HEVC video + AAC audio media with **audio first** must anchor on the
/// video track. `LlSegmenter`/`LlHlsSegmenter` used to test
/// `matches!(config, CodecConfig::Avc { .. })`, so they anchored on the audio:
/// segments did not begin on an IRAP, and because every AAC sample is a sync
/// sample every part was advertised `INDEPENDENT=YES` even when it started
/// mid-GOP — an actively wrong signal to a player.
#[test]
fn hevc_video_after_audio_is_still_the_anchor_and_parts_report_real_independence() {
    let (hvcc, width, height) = real_hevc_config();
    const AUDIO_ID: u32 = 1;
    const VIDEO_ID: u32 = 2;

    let video = TrackSpec::new(
        VIDEO_ID,
        TS_TIMESCALE,
        CodecConfig::Hevc {
            config: hvcc,
            width,
            height,
        },
    );
    // Audio deliberately FIRST in the track list.
    let tracks = vec![aac_track(AUDIO_ID), video];

    // 1 s segments, 250 ms parts → ~7.5 video frames per part, so parts land
    // mid-GOP and their independence flag is meaningful.
    let mut seg = LlHlsSegmenter::with_part_target(tracks, TS_TIMESCALE, 1.0, 250)
        .expect("construct with an HEVC anchor");

    // Two 1-second GOPs of video, IRAP only at the head of each.
    for i in 0..(2 * FRAMES_PER_SEC + 2) {
        let dts = i as i64 * FRAME_TICKS;
        seg.push(VIDEO_ID, au(dts, i % FRAMES_PER_SEC == 0))
            .expect("push video");
    }

    let parts = seg.take_ready_parts();
    let segments = seg.take_ready_segments();
    assert!(
        !segments.is_empty(),
        "the video track must drive segment cuts; anchored on audio (which never got a \
         sample here) nothing would ever be cut"
    );
    assert!(
        parts.len() > 2,
        "expected several parts, got {}",
        parts.len()
    );
    assert!(
        parts[0].independent,
        "the first part opens on the IRAP, so it is genuinely independent"
    );
    assert!(
        parts.iter().any(|p| !p.independent),
        "a part starting mid-GOP must report independent = false; anchored on the audio \
         track every part was reported independent"
    );

    // Same anchor rule in LL-DASH.
    let (hvcc2, w2, h2) = real_hevc_config();
    let ll = LlSegmenter::new(
        vec![
            aac_track(AUDIO_ID),
            TrackSpec::new(
                VIDEO_ID,
                TS_TIMESCALE,
                CodecConfig::Hevc {
                    config: hvcc2,
                    width: w2,
                    height: h2,
                },
            ),
        ],
        TS_TIMESCALE,
        1.0,
        4,
    )
    .expect("construct");
    // `part_target_secs`-free check: the anchor is the video track iff pushing
    // only video advances the segment clock and cuts.
    let mut ll = ll;
    for i in 0..(2 * FRAMES_PER_SEC + 2) {
        ll.push(
            VIDEO_ID,
            au(i as i64 * FRAME_TICKS, i % FRAMES_PER_SEC == 0),
        )
        .expect("push video");
    }
    assert!(
        ll.take_ready()
            .iter()
            .filter(|c| c.is_segment_start)
            .count()
            > 1,
        "LL-DASH must anchor on the HEVC track too"
    );
}

// ── Batch/streaming equivalence for a section track ─────────────────────────

/// The segment index a section sample landed in, for each path.
fn section_segment_index(segments: &[Vec<u8>], needle: &[u8]) -> Option<usize> {
    segments
        .iter()
        .position(|s| s.windows(needle.len()).any(|w| w == needle))
}

/// An SCTE-35-style section sample at a **late** timestamp must land in the
/// same segment whether the media is partitioned in one batch
/// (`TsHlsPackager::package` → `partition_tracks`) or incrementally
/// (`StreamingTsHlsSegmenter`).
///
/// Before the fix `partition_tracks` accumulated only `duration`, which a
/// section sample never carries, so its placement time stayed at 0 and **every
/// section sample was muxed into segment 0** — an ad-insertion cue for t=4 s
/// was signalled at t=0 — while the streaming path put it in the segment that
/// was open when it arrived. That directly contradicted the byte-identity the
/// module docs claim for the two paths.
#[test]
fn section_sample_lands_in_the_same_segment_batch_and_streaming() {
    const VIDEO_ID: u32 = 1;
    const SCTE_ID: u32 = 2;
    /// A recognisable `splice_info_section` body (table_id 0xFC) — used as the
    /// needle to find which segment carried it.
    const CUE: [u8; 6] = [0xFC, 0x30, 0x11, 0xDE, 0xAD, 0xBE];
    /// The cue's absolute decode time: 4 s into a stream cut at 1 s, so it
    /// belongs in segment 4, as far from segment 0 as this stream allows.
    const CUE_DTS: i64 = 4 * TS_TIMESCALE as i64;

    let video: Vec<Sample> = (0..(6 * FRAMES_PER_SEC))
        .map(|i| au(i as i64 * FRAME_TICKS, i % FRAMES_PER_SEC == 0))
        .collect();
    let section = Sample::new(CUE.to_vec(), Some(CUE_DTS), Some(CUE_DTS), None, true);

    // Batch.
    let media = Media::new(
        vec![
            Track::new(avc_track(VIDEO_ID), video.clone()),
            Track::new(scte35_track(SCTE_ID), vec![section.clone()]),
        ],
        TS_TIMESCALE,
    );
    let batch = TsHlsPackager::new(1)
        .package(&media)
        .expect("batch package");
    let batch_idx =
        section_segment_index(&batch.segments, &CUE).expect("batch output must carry the cue");

    // Streaming: push interleaved by decode time, exactly as the module docs
    // require for the two paths to agree.
    let mut seg = StreamingTsHlsSegmenter::new(
        vec![avc_track(VIDEO_ID), scte35_track(SCTE_ID)],
        1,
        usize::MAX,
    )
    .expect("construct");
    let mut streamed: Vec<Vec<u8>> = Vec::new();
    let mut pushed_cue = false;
    for s in &video {
        if !pushed_cue && s.dts.unwrap_or(0) >= CUE_DTS {
            if let Some(cut) = seg.push(SCTE_ID, section.clone()).expect("push cue") {
                streamed.push(cut.bytes);
            }
            pushed_cue = true;
        }
        if let Some(cut) = seg.push(VIDEO_ID, s.clone()).expect("push video") {
            streamed.push(cut.bytes);
        }
    }
    assert!(pushed_cue, "the test stream must reach the cue's timestamp");
    if let Some(tail) = seg.finish().expect("finish") {
        streamed.push(tail.bytes);
    }
    let stream_idx =
        section_segment_index(&streamed, &CUE).expect("streaming output must carry the cue");

    assert_eq!(
        batch_idx, stream_idx,
        "batch and streaming must place a timestamped section sample in the same segment; \
         before the fix batch always said 0 (batch={batch_idx}, streaming={stream_idx})"
    );
    assert!(
        batch_idx > 0,
        "a cue at {CUE_DTS} ticks belongs well past segment 0, not in it"
    );
}
