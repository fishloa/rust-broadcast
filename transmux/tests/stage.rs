//! `broadcast_common::Stage` adoption (media plane steps 2e + 2e-2).
//!
//! Step 2e found that `Stage::feed` hardcoded `&[u8]`, which genuinely fit the
//! byte-stream demux family (`StreamingTsDemux`/`StreamingFlvDemux`/
//! `ProgressiveDemux`/`RtpStreamDepacketiser`) but not the four segmenters
//! (`Segmenter`/`LlHlsSegmenter`/`LlSegmenter`/`StreamingTsHlsSegmenter`),
//! whose real per-call input is a typed `(track_id, Sample)` — forcing bytes
//! on them would have meant inventing a byte encoding nobody downstream
//! wants. Step 2e-2 fixed the trait instead of forcing the segmenters:
//! `Stage::In<'a>` is now a generic associated type, so byte-stream stages
//! declare `type In<'a> = &'a [u8];` and segmenters declare
//! `type In<'a> = (u32, Sample);` — each family states its own honest input.
//!
//! What this file proves, honestly:
//!
//! 1. **The generalised `Stage` genuinely unifies *both* families through one
//!    driver.** [`generic_drive_helper_unifies_ts_flv_and_progressive_demuxers`]
//!    drives three demuxers with three different native APIs before step 2e
//!    (`StreamingTsDemux::feed` returns `()`; `StreamingFlvDemux::feed`
//!    returns `Result<(), FlvError>`; `ProgressiveDemux` has no incremental
//!    `feed` at all) and
//!    [`generic_drive_helper_also_spans_segmenter_stages`] drives two
//!    segmenters with two entirely different `Out` types (`(Vec<u8>,
//!    SegmentMeta)` vs `Chunk`) — all five through the *exact same* generic
//!    `drive::<S: Stage>` function, checked against each type's own trusted
//!    inherent API, not just "it compiles".
//! 2. **`StreamingTsHlsSegmenter`'s enqueue-then-poll `Stage` path yields
//!    exactly what its inline-return inherent `push`/`finish` path does** —
//!    [`ts_hls_segmenter_stage_matches_inline_push`] — since `Stage` had to
//!    change that type's "return the segment directly" shape into "enqueue,
//!    then `poll`", and that bridging must not drop or reorder a segment.

use std::path::PathBuf;

use broadcast_common::{Stage, Timestamp};
use transmux::media::Media;
use transmux::{
    CodecConfig, DecoderConfigDescriptor, DecoderSpecificInfo, DemuxEvent, ESDescriptor, EsdsBox,
    LlSegmenter, ObjectTypeIndication, ProgressiveDemux, SLConfigDescriptor, Sample, SegmentMeta,
    Segmenter, StreamType, StreamingFlvDemux, StreamingTsDemux, StreamingTsHlsSegmenter, TrackSpec,
    TsDemux,
};

use broadcast_common::Unpackage;

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");
const PROGRESSIVE_MP4: &[u8] = include_bytes!("../../fixtures/transmux/h264_aac_prog.mp4");

fn fixtures_ts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

fn read(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The generic drive loop, genuinely spanning both `Stage` families: feed
/// each input item (draining `poll()` after every call, since a single
/// `feed` may unlock more than one output), then `finish()` and drain the
/// rest. Works against *any* `Stage` implementor regardless of what its
/// native `feed`/`finish` signatures looked like before adoption, and
/// regardless of whether `S::In<'a>` is a borrowed byte slice (the demux
/// family) or an owned `(u32, Sample)` (the segmenter family) — the only
/// thing pinning `'a` down is whatever the caller's `inputs` iterator
/// actually borrows (nothing, for the owned segmenter case).
fn drive<'a, S, I>(stage: &mut S, inputs: I) -> Vec<S::Out>
where
    S: Stage,
    S::Error: core::fmt::Debug,
    I: IntoIterator<Item = S::In<'a>>,
{
    let mut out = Vec::new();
    for (i, input) in inputs.into_iter().enumerate() {
        stage
            .feed(input, Timestamp::from_nanos(i as u64))
            .expect("feed");
        while let Some(ev) = stage.poll() {
            out.push(ev);
        }
    }
    stage.finish().expect("finish");
    while let Some(ev) = stage.poll() {
        out.push(ev);
    }
    out
}

/// Drive `StreamingTsDemux` via the generic `drive()` helper, chunked into
/// small (317-byte, deliberately not TS-packet-aligned) pieces, and check the
/// resulting `Sample`/`TrackAdded` counts against the trusted batch `TsDemux`
/// — the same oracle `transmux/tests/streaming_demux.rs` uses.
#[test]
fn generic_drive_helper_unifies_ts_flv_and_progressive_demuxers() {
    // --- StreamingTsDemux ---------------------------------------------------
    let ts_bytes = read(&fixtures_ts_dir().join("h264_aac.ts"));
    let oracle_ts: Media = TsDemux::new().unpackage(&ts_bytes).expect("batch TS demux");
    let oracle_ts_samples: usize = oracle_ts.tracks.iter().map(|t| t.samples.len()).sum();

    let ts_chunks: Vec<&[u8]> = ts_bytes.chunks(317).collect();
    let mut ts_stage = StreamingTsDemux::new();
    let ts_events = drive(&mut ts_stage, ts_chunks);
    let ts_added = ts_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
        .count();
    let ts_samples = ts_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
        .count();
    assert_eq!(
        ts_added,
        oracle_ts.tracks.len(),
        "Stage-driven StreamingTsDemux must add the same tracks as the batch oracle"
    );
    assert_eq!(
        ts_samples, oracle_ts_samples,
        "Stage-driven StreamingTsDemux must yield the same sample count as the batch oracle"
    );

    // --- StreamingFlvDemux ---------------------------------------------------
    let flv_bytes: &[u8] = FLV;
    let oracle_flv: Media = transmux::FlvDemux::new()
        .unpackage(flv_bytes)
        .expect("batch FLV demux");
    let oracle_flv_samples: usize = oracle_flv.tracks.iter().map(|t| t.samples.len()).sum();

    let flv_chunks: Vec<&[u8]> = flv_bytes.chunks(257).collect();
    let mut flv_stage = StreamingFlvDemux::new();
    let flv_events = drive(&mut flv_stage, flv_chunks);
    let flv_added = flv_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
        .count();
    let flv_samples = flv_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
        .count();
    assert_eq!(
        flv_added,
        oracle_flv.tracks.len(),
        "Stage-driven StreamingFlvDemux must add the same tracks as the batch oracle"
    );
    assert_eq!(
        flv_samples, oracle_flv_samples,
        "Stage-driven StreamingFlvDemux must yield the same sample count as the batch oracle"
    );

    // --- ProgressiveDemux -----------------------------------------------------
    // Whole-file parse (see the type's own docs): `Out = Media`, one value
    // popped from `poll()` once, after `finish()` — proving `drive()` handles
    // an `Out` type that isn't `DemuxEvent` at all just as well.
    let oracle_prog: Media = ProgressiveDemux::new()
        .unpackage(PROGRESSIVE_MP4)
        .expect("batch progressive demux");

    let prog_chunks: Vec<&[u8]> = PROGRESSIVE_MP4.chunks(4096).collect();
    let mut prog_stage = ProgressiveDemux::new();
    let mut prog_media = drive(&mut prog_stage, prog_chunks);
    assert_eq!(
        prog_media.len(),
        1,
        "ProgressiveDemux emits exactly one Media"
    );
    let media = prog_media.pop().unwrap();
    assert_eq!(
        media.tracks.len(),
        oracle_prog.tracks.len(),
        "Stage-driven ProgressiveDemux must yield the same track count as Unpackage::unpackage"
    );
    let prog_samples: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
    let oracle_prog_samples: usize = oracle_prog.tracks.iter().map(|t| t.samples.len()).sum();
    assert_eq!(
        prog_samples, oracle_prog_samples,
        "Stage-driven ProgressiveDemux must yield the same sample count as Unpackage::unpackage"
    );
}

/// [`StreamingTsDemux::demand`]'s `saturated` flag must be honest: it tracks
/// the one bound this demuxer actually enforces end-to-end, the
/// never-claimed-PID `unattributed` replay buffer. Flood a PID that never
/// appears in any PAT/PMT (so every packet lands in `unattributed`) until the
/// buffer is packed to its cap, and confirm `demand().saturated` flips to
/// `true` — a real `Limits`-bound transition, not a fabricated one.
#[test]
fn demand_saturated_flips_true_at_the_unattributed_bytes_bound() {
    const NEVER_CLAIMED_PID: u16 = 0x0234;

    fn payload_only_packet(pid: u16, cc: u8) -> [u8; 188] {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = 0x47;
        pkt[1] = ((pid >> 8) as u8) & 0x1F; // payload_unit_start_indicator = 0
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x10 | (cc & 0x0F); // adaptation_field_control = payload only
        pkt
    }

    let mut demux = StreamingTsDemux::new();
    let mut cc: u8 = 0;
    let mut saw_saturated = false;
    // 4 MiB / ~184 payload bytes per packet ~= 22808 packets to cross the
    // cap; 30_000 comfortably clears it (matches the scale the crate's own
    // MAX_PES_BUFFER_BYTES flood test already uses without being slow).
    for _ in 0..30_000u32 {
        let pkt = payload_only_packet(NEVER_CLAIMED_PID, cc);
        cc = cc.wrapping_add(1);
        Stage::feed(&mut demux, &pkt, Timestamp::ZERO).expect("feed never errors");
        while Stage::poll(&mut demux).is_some() {} // drain — no PMT ever resolves this PID
        if Stage::demand(&demux).saturated {
            saw_saturated = true;
            break;
        }
    }
    assert!(
        saw_saturated,
        "demand().saturated must flip true once the unattributed replay buffer hits its cap"
    );
}

// ── Segmenter-family Stage fixtures ─────────────────────────────────────────
//
// A minimal but structurally-real AAC `esds`, so `build_init_segment`
// succeeds — the same fixture shape `transmux/tests/segmenter.rs` uses.
// Audio-only (no video track) keeps this self-contained: every `Sample` is a
// sync sample, so the anchor cuts purely on accumulated duration, with no
// need for a real `avcC`.

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

fn audio_track_spec() -> TrackSpec {
    TrackSpec::new(
        1,
        48_000,
        CodecConfig::Aac {
            esds: dummy_esds(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    )
}

/// 50 audio samples at 1024 ticks (~21.3 ms) each @ 48 kHz — audio samples
/// are always sync (`Sample::from_raw`), so the anchor cuts purely on
/// accumulated duration against the segmenter's target.
fn audio_samples(n: usize) -> Vec<(u32, Sample)> {
    (0..n)
        .map(|_| (1u32, Sample::from_raw(vec![0u8; 4], None, None, Some(1024))))
        .collect()
}

/// The second half of the acceptance evidence this step's brief asks for:
/// [`drive`] genuinely spans the segmenter family too, not just the
/// byte-stream demux family above. Drives `Segmenter` and `LlSegmenter` —
/// two segmenters with entirely different `Out` types (`(Vec<u8>,
/// SegmentMeta)` vs `Chunk`) — through the *same* generic `drive::<S: Stage>`
/// function used for the demuxers, and checks the result against each type's
/// own trusted inherent `push`/`take_ready*`/`flush` API run over the exact
/// same samples — not just "it compiles".
#[test]
fn generic_drive_helper_also_spans_segmenter_stages() {
    let track = audio_track_spec();
    let samples = audio_samples(50);

    // --- Segmenter: inherent API (oracle) vs Stage-driven -------------------
    let mut oracle_seg = Segmenter::new(vec![track.clone()], 1000, 0.1).unwrap();
    let mut oracle_seg_out: Vec<(Vec<u8>, SegmentMeta)> = Vec::new();
    for (track_id, sample) in samples.clone() {
        oracle_seg.push(track_id, sample).unwrap();
        oracle_seg_out.extend(oracle_seg.take_ready_with_meta());
    }
    oracle_seg.flush().unwrap();
    oracle_seg_out.extend(oracle_seg.take_ready_with_meta());
    assert!(
        !oracle_seg_out.is_empty(),
        "fixture must actually exercise at least one segment cut"
    );

    let mut stage_seg = Segmenter::new(vec![track.clone()], 1000, 0.1).unwrap();
    let stage_seg_out = drive(&mut stage_seg, samples.clone());
    assert_eq!(
        stage_seg_out, oracle_seg_out,
        "Stage-driven Segmenter must yield the exact same (bytes, meta) sequence as the inherent push/take_ready_with_meta/flush API"
    );

    // --- LlSegmenter: inherent API (oracle) vs Stage-driven ------------------
    let mut oracle_ll = LlSegmenter::new(vec![track.clone()], 1000, 0.1, 4).unwrap();
    let mut oracle_ll_out: Vec<transmux::Chunk> = Vec::new();
    for (track_id, sample) in samples.clone() {
        oracle_ll.push(track_id, sample).unwrap();
        oracle_ll_out.extend(oracle_ll.take_ready());
    }
    oracle_ll.flush().unwrap();
    oracle_ll_out.extend(oracle_ll.take_ready());
    assert!(
        !oracle_ll_out.is_empty(),
        "fixture must actually exercise at least one chunk emission"
    );

    let mut stage_ll = LlSegmenter::new(vec![track], 1000, 0.1, 4).unwrap();
    let stage_ll_out = drive(&mut stage_ll, samples);
    assert_eq!(
        stage_ll_out.len(),
        oracle_ll_out.len(),
        "Stage-driven LlSegmenter must yield the same chunk count as the inherent push/take_ready/flush API"
    );
    for (stage_chunk, oracle_chunk) in stage_ll_out.iter().zip(oracle_ll_out.iter()) {
        // `Chunk` isn't `PartialEq` (it isn't a wire-comparable spec type in
        // its own right), so compare the fields that matter field-by-field.
        assert_eq!(stage_chunk.data, oracle_chunk.data);
        assert_eq!(stage_chunk.segment_number, oracle_chunk.segment_number);
        assert_eq!(stage_chunk.is_segment_start, oracle_chunk.is_segment_start);
        assert_eq!(stage_chunk.sequence_number, oracle_chunk.sequence_number);
    }
}

/// `StreamingTsHlsSegmenter` is the one segmenter whose inherent
/// `push`/`finish` already return the cut segment **inline** rather than via
/// a drain — under `Stage` those calls instead enqueue into a staging queue
/// for `poll` to hand back (see the impl's own docs). This test proves that
/// bridging is behaviour-neutral: driving the segmenter via `Stage::feed`/
/// `poll`/`finish` yields exactly the same sequence of segments, in the same
/// order, as collecting `push`'s/`finish`'s inline `Option<TsSegment>`
/// return values directly.
#[test]
fn ts_hls_segmenter_stage_matches_inline_push() {
    let track = audio_track_spec();
    let samples = audio_samples(50);

    // --- Inline inherent API (oracle) ---------------------------------------
    let mut inline_seg = StreamingTsHlsSegmenter::new(vec![track.clone()], 1, 10).unwrap();
    let mut inline_out = Vec::new();
    for (track_id, sample) in samples.clone() {
        if let Some(seg) = inline_seg.push(track_id, sample).unwrap() {
            inline_out.push(seg);
        }
    }
    if let Some(seg) = inline_seg.finish().unwrap() {
        inline_out.push(seg);
    }
    assert!(
        !inline_out.is_empty(),
        "fixture must actually exercise at least one segment cut"
    );

    // --- Stage enqueue-then-poll path ----------------------------------------
    let mut stage_seg = StreamingTsHlsSegmenter::new(vec![track], 1, 10).unwrap();
    let stage_out = drive(&mut stage_seg, samples);

    assert_eq!(
        stage_out.len(),
        inline_out.len(),
        "Stage-driven enqueue-then-poll must yield the same segment count as the inline push/finish return values"
    );
    for (staged, inline) in stage_out.iter().zip(inline_out.iter()) {
        assert_eq!(staged.bytes, inline.bytes);
        assert_eq!(staged.duration, inline.duration);
        assert_eq!(staged.discontinuous, inline.discontinuous);
        assert_eq!(staged.uri, inline.uri);
        assert_eq!(staged.sequence, inline.sequence);
    }
}
