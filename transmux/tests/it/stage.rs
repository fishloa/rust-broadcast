//! `broadcast_common::Stage` adoption (media plane steps 2e + 2e-2).
//!
//! Step 2e found that `Stage::feed` hardcoded `&[u8]`, which genuinely fit the
//! byte-stream demux family (`StreamingTsDemux`/`StreamingFlvDemux`/
//! `Mp4Demux`/`RtpStreamDepacketiser`) but not the four segmenters
//! (`Segmenter`/`LlHlsSegmenter`/`LlSegmenter`/`StreamingTsHlsSegmenter`),
//! whose real per-call input is a typed `(track_id, Sample)` — forcing bytes
//! on them would have meant inventing a byte encoding nobody downstream
//! wants. Step 2e-2 fixed the trait instead of forcing the segmenters:
//! `Stage::In<'a>` is now a generic associated type, so byte-stream stages
//! declare `type In<'a> = &'a [u8];` and segmenters declare
//! `type In<'a> = (TrackId, Sample);` — each family states its own honest input.
//!
//! What this file proves, honestly:
//!
//! 1. **The generalised `Stage` genuinely unifies *both* families through one
//!    driver.** [`generic_drive_helper_unifies_ts_flv_and_progressive_demuxers`]
//!    drives three demuxers with three different native APIs before step 2e
//!    (`StreamingTsDemux::feed` returns `()`; `StreamingFlvDemux::feed`
//!    returns `Result<(), FlvError>`; `Mp4Demux` has no incremental
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
//! 3. **`Segmenter`/`LlSegmenter`/`LlHlsSegmenter` deliver every output
//!    exactly once when ONE instance is driven through *both* the inherent
//!    `take_ready*` API and `Stage::feed`/`poll`, interleaved** — the
//!    `*_mixed_api_delivers_each_output_exactly_once` tests below. Before
//!    this fix these three segmenters' `Stage::feed`/`finish` eagerly
//!    relocated everything out of the inherent drain's backing queue on
//!    every call, so the inherent `take_ready*` returned empty after any
//!    `Stage::feed`, and driving `Stage::poll` alone forever returned `None`
//!    for output sitting in `ready` — the two-separate-instances comparisons
//!    above structurally cannot see that, since they never drive one
//!    instance through both APIs.
//!    [`generic_drive_helper_also_spans_segmenter_stages`] additionally now
//!    covers `LlHlsSegmenter` too (previously zero `Stage` coverage at all).

use crate::hub_fixtures::dummy_esds;
use broadcast_common::timescale::Timescale;
use core::time::Duration;
use std::path::PathBuf;
use transmux::TrackId;

use broadcast_common::{Stage, Timestamp};
use transmux::media::Media;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::mux_stage::{MuxInput, MuxOutput, SegmentOut};
use transmux::{
    CodecConfig, DemuxEvent, LlHlsSegmenter, LlHlsStageOutput, LlSegmenter, PartInfo, Sample,
    SegmentInfo, Segmenter, StreamingFlvDemux, StreamingTsDemux, StreamingTsHlsSegmenter,
    TrackSpec, TsDemux,
};

use broadcast_common::Unpackage;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

static FLV: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("flv/av.flv"));
static PROGRESSIVE_MP4: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("mp4/h264-aac-prog.mp4"));

fn fixtures_ts_dir() -> PathBuf {
    test_fixtures::path("ts")
}

fn read(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// ── T6 (test-integrity audit): sample-content comparison, not just counts ──
//
// `generic_drive_helper_unifies_ts_flv_and_progressive_demuxers` used to
// compare only `TrackAdded`/`Sample` COUNTS between the Stage-driven event
// stream and the trusted batch oracle — a Stage adapter that emitted the
// right NUMBER of samples with the wrong payload bytes or the wrong
// dts/pts/duration would pass just as well as a correct one. The helpers
// below collect the actual per-track `Sample`s from the event stream and
// compare them field-for-field (including the coded payload bytes) against
// the oracle.

/// Collect every `DemuxEvent::TrackAdded`/`Sample` from `events` into
/// `(TrackSpec, Vec<Sample>)` pairs, in `TrackAdded` emission order — so the
/// Stage-driven event stream can be compared sample-for-sample against the
/// batch oracle's `Media`, not merely by count.
fn collect_tracks_from_events(events: &[DemuxEvent]) -> Vec<(TrackSpec, Vec<Sample>)> {
    let mut specs: Vec<TrackSpec> = Vec::new();
    let mut samples: std::collections::BTreeMap<TrackId, Vec<Sample>> = Default::default();
    for e in events {
        match e {
            DemuxEvent::TrackAdded(spec) => specs.push(spec.clone()),
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                samples.entry(*track_id).or_default().push(sample.clone());
            }
            _ => {}
        }
    }
    specs
        .into_iter()
        .map(|spec| {
            let s = samples.remove(&spec.track_id).unwrap_or_default();
            (spec, s)
        })
        .collect()
}

/// Find `oracle_tracks`' unique track whose `CodecConfig` variant (its enum
/// discriminant only — dimensions/bitrate may legitimately differ across
/// re-parses) matches `spec`'s, so the Stage-driven track and the batch
/// oracle track for the *same elementary stream* are paired up correctly
/// regardless of `track_id` numbering differences between the two paths.
fn find_matching_oracle_track<'a>(
    spec: &TrackSpec,
    oracle_tracks: &'a [transmux::media::Track],
) -> &'a transmux::media::Track {
    let mut matches = oracle_tracks.iter().filter(|t| {
        core::mem::discriminant(&t.spec.config) == core::mem::discriminant(&spec.config)
    });
    let found = matches
        .next()
        .unwrap_or_else(|| panic!("no oracle track matches codec kind {:?}", spec.config));
    assert!(
        matches.next().is_none(),
        "ambiguous match: more than one oracle track shares codec kind {:?} — \
         this helper needs a less coarse key for this fixture",
        spec.config
    );
    found
}

/// Assert that every sample field the IR actually carries — `dts`, `pts`,
/// `duration`, the sync flag, and the coded payload bytes — matches between
/// the Stage-driven event stream and the batch oracle. Sample *count* alone
/// (what this test used to check) is satisfied by a demuxer that emits the
/// right number of samples with the wrong payload or the wrong timestamps.
fn assert_samples_match(got: &[Sample], oracle: &[Sample], what: &str) {
    assert_eq!(
        got.len(),
        oracle.len(),
        "{what}: sample count must match the batch oracle"
    );
    for (i, (g, o)) in got.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(
            g.timing.and_then(|t| t.dts()),
            o.timing.and_then(|t| t.dts()),
            "{what}: sample {i} dts must match the batch oracle"
        );
        assert_eq!(
            g.timing.and_then(|t| t.pts()),
            o.timing.and_then(|t| t.pts()),
            "{what}: sample {i} pts must match the batch oracle"
        );
        assert_eq!(
            g.timing.and_then(|t| t.duration()),
            o.timing.and_then(|t| t.duration()),
            "{what}: sample {i} duration must match the batch oracle"
        );
        assert_eq!(
            g.flags.is_sync, o.flags.is_sync,
            "{what}: sample {i} sync flag must match the batch oracle"
        );
        assert_eq!(
            g.data, o.data,
            "{what}: sample {i} payload bytes must match the batch oracle byte-for-byte"
        );
    }
}

/// Proves `assert_samples_match` actually discriminates on payload bytes —
/// not vacuous, unlike the count-only check it replaces. Two hand-built
/// samples with identical dts/pts/duration/sync but different `data` (same
/// length, so a length-only or count-only comparison would still pass) must
/// make it panic.
#[test]
#[should_panic(expected = "payload bytes must match")]
fn assert_samples_match_catches_wrong_payload_at_matching_count() {
    let got = [
        Sample::new(vec![0x01, 0x02, 0x03], Some(0), Some(0), Some(10), true)
            .expect("valid timing"),
    ];
    let oracle = [
        Sample::new(vec![0x01, 0x02, 0xFF], Some(0), Some(0), Some(10), true)
            .expect("valid timing"),
    ];
    assert_samples_match(&got, &oracle, "synthetic");
}

/// The generic drive loop, genuinely spanning both `Stage` families: feed
/// each input item (draining `poll()` after every call, since a single
/// `feed` may unlock more than one output), then `finish()` and drain the
/// rest. Works against *any* `Stage` implementor regardless of what its
/// native `feed`/`finish` signatures looked like before adoption, and
/// regardless of whether `S::In<'a>` is a borrowed byte slice (the demux
/// family) or an owned `(TrackId, Sample)` (the segmenter family) — the only
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

/// Keep only `MuxOutput::Segment` items.
fn segments(outputs: Vec<MuxOutput>) -> Vec<SegmentOut> {
    outputs
        .into_iter()
        .filter_map(|output| match output {
            MuxOutput::Segment(segment) => Some(segment),
            _ => None,
        })
        .collect()
}

/// Wrap `(TrackId, Sample)` pairs as [`MuxInput::Sample`] items.
fn mux_inputs(samples: Vec<(TrackId, Sample)>) -> Vec<MuxInput> {
    samples
        .into_iter()
        .map(|(track_id, sample)| MuxInput::Sample {
            track: TrackId::new(track_id.value()),
            sample,
        })
        .collect()
}

/// Drive `StreamingTsDemux` via the generic `drive()` helper, chunked into
/// small (317-byte, deliberately not TS-packet-aligned) pieces, and check the
/// resulting per-track samples — dts/pts/duration/sync flag/payload bytes,
/// not just counts — against the trusted batch `TsDemux` oracle (the same
/// oracle `transmux/tests/it/streaming_demux.rs` uses).
#[test]
fn generic_drive_helper_unifies_ts_flv_and_progressive_demuxers() {
    // --- StreamingTsDemux ---------------------------------------------------
    let ts_bytes = read(&fixtures_ts_dir().join("h264-aac.ts"));
    let oracle_ts: Media = TsDemux::new().unpackage(&ts_bytes).expect("batch TS demux");

    let ts_chunks: Vec<&[u8]> = ts_bytes.chunks(317).collect();
    let mut ts_stage = StreamingTsDemux::new();
    let ts_events = drive(&mut ts_stage, ts_chunks);
    let ts_tracks = collect_tracks_from_events(&ts_events);
    assert_eq!(
        ts_tracks.len(),
        oracle_ts.tracks.len(),
        "Stage-driven StreamingTsDemux must add the same tracks as the batch oracle"
    );
    for (spec, samples) in &ts_tracks {
        let oracle_track = find_matching_oracle_track(spec, &oracle_ts.tracks);
        assert_samples_match(samples, &oracle_track.samples, "StreamingTsDemux");
    }

    // --- StreamingFlvDemux ---------------------------------------------------
    let flv_bytes: &[u8] = FLV.as_slice();
    let oracle_flv: Media = transmux::FlvDemux::new()
        .unpackage(flv_bytes)
        .expect("batch FLV.as_slice() demux");

    let flv_chunks: Vec<&[u8]> = flv_bytes.chunks(257).collect();
    let mut flv_stage = StreamingFlvDemux::new();
    let flv_events = drive(&mut flv_stage, flv_chunks);
    let flv_tracks = collect_tracks_from_events(&flv_events);
    assert_eq!(
        flv_tracks.len(),
        oracle_flv.tracks.len(),
        "Stage-driven StreamingFlvDemux must add the same tracks as the batch oracle"
    );
    for (spec, samples) in &flv_tracks {
        let oracle_track = find_matching_oracle_track(spec, &oracle_flv.tracks);
        assert_samples_match(samples, &oracle_track.samples, "StreamingFlvDemux");
    }

    // --- Mp4Demux -----------------------------------------------------
    // The streaming stage, driven through the same generic helper: it emits
    // the crate's event vocabulary, so the folded events are compared with the
    // whole-image collector's `Media`.
    let oracle_prog: Media = test_harness::collect::media_from_mp4(
        PROGRESSIVE_MP4.as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("batch progressive demux");

    let prog_inputs: Vec<Mp4Input<'_>> = PROGRESSIVE_MP4
        .as_slice()
        .chunks(4096)
        .map(Mp4Input::Next)
        .collect();
    let mut prog_stage = Mp4Demux::sequential(Mp4DemuxConfig::default());
    let events = drive(&mut prog_stage, prog_inputs);
    let media = test_harness::collect::media_from_events(events);
    assert_eq!(
        media.tracks.len(),
        oracle_prog.tracks.len(),
        "Stage-driven Mp4Demux must yield the same track count as the whole-image collector"
    );
    for spec_track in &media.tracks {
        let oracle_track = find_matching_oracle_track(&spec_track.spec, &oracle_prog.tracks);
        assert_samples_match(&spec_track.samples, &oracle_track.samples, "Mp4Demux");
    }
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

/// [`Mp4Demux`]'s sequential buffer is bounded by the reader's hostile-input
/// caps: feeding more than `max_need_bytes` in small chunks must be rejected
/// with a typed [`Error::LimitExceeded`] rather than growing the buffer past
/// the cap.
#[test]
fn sequential_stage_rejects_input_past_its_byte_cap() {
    const MAX_BYTES: u64 = 4096;
    const CHUNK: usize = 64;

    let limits = isobmff::Limits::default()
        .with_max_moov_bytes(MAX_BYTES)
        .with_max_fragment_bytes(MAX_BYTES);
    let config = Mp4DemuxConfig::default().with_limits(limits);
    let mut demux = Mp4Demux::sequential(config);
    let mut chunk = vec![0u8; CHUNK];
    let declared = u32::try_from(1usize << 20).expect("box size");
    chunk[..4].copy_from_slice(&declared.to_be_bytes());
    chunk[4..8].copy_from_slice(b"free");
    let mut err = None;
    let mut saw_saturated = false;
    for _ in 0..(usize::try_from(MAX_BYTES).unwrap() / CHUNK + 4) {
        if Stage::demand(&demux).saturated {
            saw_saturated = true;
        }
        if let Err(e) = Stage::feed(&mut demux, Mp4Input::Next(&chunk), Timestamp::ZERO) {
            err = Some(e);
            break;
        }
    }
    assert!(
        saw_saturated,
        "demand().saturated must flip true before the cap is actually exceeded"
    );
    match err.expect("feed must eventually reject input past the byte cap") {
        transmux::Error::LimitExceeded { limit, .. } => {
            assert_eq!(limit, MAX_BYTES, "the error must name the configured bound");
        }
        other => panic!("expected LimitExceeded naming the bound, got {other:?}"),
    }
}

/// A driver that honours [`Demand`] never sees the cap error and never lets the
/// stage hold more than the cap: over a real init plus media segment larger
/// than the cap, every feed is sized by `demand().want_bytes`.
#[test]
fn a_driver_honouring_demand_never_exceeds_the_sequential_cap() {
    const CAP: u64 = 4096;

    let mut bytes = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
    bytes.extend_from_slice(&test_fixtures::read("hls/cmaf-fmp4/index0.m4s"));

    let limits = isobmff::Limits::default()
        .with_max_moov_bytes(CAP)
        .with_max_fragment_bytes(CAP);
    let config = Mp4DemuxConfig::default().with_limits(limits);
    let mut demux = Mp4Demux::sequential(config);
    let mut offset = 0usize;
    let mut samples = 0usize;
    loop {
        while let Some(event) = Stage::poll(&mut demux) {
            if matches!(event, DemuxEvent::Sample { .. }) {
                samples += 1;
            }
        }
        assert!(
            u64::try_from(demux.buffered().0).unwrap_or(u64::MAX) <= CAP,
            "the stage held {} bytes against a {CAP}-byte cap",
            demux.buffered().0
        );
        let demand = Stage::demand(&demux);
        if demand.saturated {
            break;
        }
        if offset >= bytes.len() {
            break;
        }
        let want = demand.want_bytes.max(1).min(bytes.len() - offset);
        Stage::feed(
            &mut demux,
            Mp4Input::Next(&bytes[offset..offset + want]),
            Timestamp::ZERO,
        )
        .expect("a demand-sized feed never exceeds the cap");
        offset += want;
    }
    Stage::finish(&mut demux).expect("the demand-sized stream finishes");
    while let Some(event) = Stage::poll(&mut demux) {
        if matches!(event, DemuxEvent::Sample { .. }) {
            samples += 1;
        }
    }
    assert!(samples > 0, "the stream yielded samples");
}

// ── Segmenter-family Stage fixtures ─────────────────────────────────────────
//
// A minimal but structurally-real AAC `esds`, so `build_init_segment`
// succeeds — the same fixture shape `transmux/tests/it/segmenter.rs` uses.
// Audio-only (no video track) keeps this self-contained: every `Sample` is a
// sync sample, so the anchor cuts purely on accumulated duration, with no
// need for a real `avcC`.

fn audio_track_spec() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(1),
        timescale(48_000),
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
fn audio_samples(n: usize) -> Vec<(TrackId, Sample)> {
    (0..n)
        .map(|_| {
            (
                TrackId::new(1),
                Sample::from_raw(vec![0u8; 4], None, None, Some(1024)).expect("valid timing"),
            )
        })
        .collect()
}

/// Drain an [`LlSegmenter`] through the one delivery API it has left.
fn drain_ll(seg: &mut LlSegmenter, out: &mut Vec<transmux::mux_stage::MuxOutput>) {
    while let Some(item) = seg.poll() {
        out.push(item);
    }
}

/// The second half of the acceptance evidence this step's brief asks for:
/// [`drive`] genuinely spans the segmenter family too, not just the
/// byte-stream demux family above. Drives `Segmenter` and `LlSegmenter` —
/// two segmenters with entirely different `Out` types (a segment list vs
/// `MuxOutput`) — through the *same* generic `drive::<S: Stage>`
/// function used for the demuxers, and checks the result against each type's
/// own trusted inherent `push`/`take_ready*`/`flush` API run over the exact
/// same samples — not just "it compiles".
#[test]
fn generic_drive_helper_also_spans_segmenter_stages() {
    let track = audio_track_spec();
    let samples = audio_samples(50);

    // --- Segmenter: inherent API (oracle) vs Stage-driven -------------------
    let mut oracle_seg = Segmenter::new(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
    )
    .unwrap();
    let oracle_seg_out = segments(drive(&mut oracle_seg, mux_inputs(samples.clone())));
    assert!(
        !oracle_seg_out.is_empty(),
        "fixture must actually exercise at least one segment cut"
    );

    let mut stage_seg = Segmenter::new(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
    )
    .unwrap();
    let stage_seg_out = segments(drive(&mut stage_seg, mux_inputs(samples.clone())));
    assert_eq!(
        stage_seg_out, oracle_seg_out,
        "a Segmenter driven through the generic drive() helper must yield the exact same segment sequence as one driven by a plain feed/poll loop"
    );

    // --- LlSegmenter: inherent API (oracle) vs Stage-driven ------------------
    let mut oracle_ll = LlSegmenter::new(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
        4,
    )
    .unwrap();
    let mut oracle_ll_out: Vec<transmux::mux_stage::MuxOutput> = Vec::new();
    for (track_id, sample) in samples.clone() {
        oracle_ll
            .push(TrackId::new(track_id.value()), sample)
            .unwrap();
        drain_ll(&mut oracle_ll, &mut oracle_ll_out);
    }
    oracle_ll.flush().unwrap();
    drain_ll(&mut oracle_ll, &mut oracle_ll_out);
    assert!(
        !oracle_ll_out.is_empty(),
        "fixture must actually exercise at least one chunk emission"
    );

    let mut stage_ll = LlSegmenter::new(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
        4,
    )
    .unwrap();
    let stage_ll_out = drive(&mut stage_ll, mux_inputs(samples.clone()));
    assert_eq!(
        stage_ll_out.len(),
        oracle_ll_out.len(),
        "Stage-driven LlSegmenter must yield the same chunk count as the inherent push/take_ready/flush API"
    );
    assert_eq!(
        stage_ll_out, oracle_ll_out,
        "Stage-driven LlSegmenter must yield the same chunks as the inherent push/flush API"
    );

    // --- LlHlsSegmenter: inherent API (oracle) vs Stage-driven ---------------
    // Previously zero `Stage` coverage of any kind for this type; this also
    // exercises `Stage::Out = LlHlsStageOutput`, the enum carrying both the
    // part and segment channels through one `poll()`.
    let mut oracle_hls = LlHlsSegmenter::with_part_target(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
        Duration::from_millis(30),
    )
    .unwrap();
    let mut oracle_hls_parts: Vec<PartInfo> = Vec::new();
    let mut oracle_hls_segments: Vec<SegmentInfo> = Vec::new();
    for (track_id, sample) in samples.clone() {
        oracle_hls
            .push(TrackId::new(track_id.value()), sample)
            .unwrap();
        oracle_hls_parts.extend(oracle_hls.take_ready_parts());
        oracle_hls_segments.extend(oracle_hls.take_ready_segments());
    }
    oracle_hls.flush().unwrap();
    oracle_hls_parts.extend(oracle_hls.take_ready_parts());
    oracle_hls_segments.extend(oracle_hls.take_ready_segments());
    assert!(
        !oracle_hls_parts.is_empty(),
        "fixture must actually exercise at least one part emission"
    );
    assert!(
        !oracle_hls_segments.is_empty(),
        "fixture must actually exercise at least one segment emission"
    );

    let mut stage_hls = LlHlsSegmenter::with_part_target(
        vec![track],
        timescale(1000),
        Duration::from_millis(100),
        Duration::from_millis(30),
    )
    .unwrap();
    let stage_hls_out = drive(&mut stage_hls, mux_inputs(samples));
    let stage_hls_parts: Vec<&PartInfo> = stage_hls_out
        .iter()
        .filter_map(|o| match o {
            LlHlsStageOutput::Part(p) => Some(p),
            _ => None,
        })
        .collect();
    let stage_hls_segments: Vec<&SegmentInfo> = stage_hls_out
        .iter()
        .filter_map(|o| match o {
            LlHlsStageOutput::Segment(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(
        stage_hls_parts.len(),
        oracle_hls_parts.len(),
        "Stage-driven LlHlsSegmenter must yield the same part count as the inherent push/take_ready_parts/flush API"
    );
    for (stage_part, oracle_part) in stage_hls_parts.iter().zip(oracle_hls_parts.iter()) {
        assert_eq!(stage_part.bytes, oracle_part.bytes);
        assert_eq!(stage_part.duration, oracle_part.duration);
        assert_eq!(stage_part.independent, oracle_part.independent);
        assert_eq!(stage_part.segment_seq, oracle_part.segment_seq);
        assert_eq!(stage_part.part_index, oracle_part.part_index);
    }
    assert_eq!(
        stage_hls_segments.len(),
        oracle_hls_segments.len(),
        "Stage-driven LlHlsSegmenter must yield the same segment count as the inherent push/take_ready_segments/flush API"
    );
    for (stage_seg, oracle_seg) in stage_hls_segments.iter().zip(oracle_hls_segments.iter()) {
        assert_eq!(stage_seg.bytes, oracle_seg.bytes);
        assert_eq!(stage_seg.duration, oracle_seg.duration);
        assert_eq!(stage_seg.segment_seq, oracle_seg.segment_seq);
        assert_eq!(stage_seg.part_count, oracle_seg.part_count);
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
        inline_seg
            .push(TrackId::new(track_id.value()), sample)
            .unwrap();
        while let Some(item) = inline_seg.poll() {
            inline_out.push(item);
        }
    }
    inline_seg.finish().unwrap();
    while let Some(item) = inline_seg.poll() {
        inline_out.push(item);
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

// ── Mixed-API tests: ONE instance driven through both the inherent and ─────
// ── `Stage` APIs, deliberately out of lockstep (the case the two-separate- ─
// ── instances tests above structurally cannot catch — see the module ──────
// ── doc's point 3) ──────────────────────────────────────────────────────────
//
// Each test feeds a whole batch through `Stage::feed` alone (never draining
// via `poll` in between, so several cuts can pile up unread), then drains
// only *some* of it via `Stage::poll`, then hands the rest to the *inherent*
// `take_ready*` drain — the sequence the old eager-relocation bug broke,
// since `feed` had already relocated everything out of the queue the
// inherent drain reads, so that call found nothing even though output was
// still waiting. The remaining input is then driven through the inherent
// `push`/`take_ready*` API, and finally `Stage::finish` + a `poll`/inherent
// sweep collects any tail. The result is compared, in order, against a
// second instance driven purely through the inherent API over the same
// input.

/// `Segmenter`.
#[test]
fn segmenter_feeds_and_polls_in_lockstep_with_an_undrained_batch() {
    let track = audio_track_spec();
    let samples = audio_samples(50);

    // Oracle: one instance driven by a plain feed/poll loop.
    let mut oracle = Segmenter::new(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
    )
    .unwrap();
    let oracle_out = segments(drive(&mut oracle, mux_inputs(samples.clone())));
    assert!(
        oracle_out.len() >= 4,
        "fixture must cut several segments so an un-drained batch has more \
         than Stage::poll's partial drain can cover"
    );

    // Mixed: ONE instance. Feed the first batch with NO draining at all, then
    // drain only *part* of it via `poll`, and feed the rest before finishing.
    // Every output must still arrive exactly once, in order.
    let mut mixed =
        Segmenter::new(vec![track], timescale(1000), Duration::from_millis(100)).unwrap();
    let mut mixed_out: Vec<SegmentOut> = Vec::new();
    let mut samples = samples.into_iter();
    let batch = 30;
    for (i, (track_id, sample)) in samples.by_ref().take(batch).enumerate() {
        mixed
            .feed(
                MuxInput::Sample {
                    track: TrackId::new(track_id.value()),
                    sample,
                },
                Timestamp::from_nanos(i as u64),
            )
            .unwrap();
    }
    for _ in 0..2 {
        if let Some(MuxOutput::Segment(segment)) = mixed.poll() {
            mixed_out.push(segment);
        }
    }
    for (track_id, sample) in samples {
        mixed
            .feed(
                MuxInput::Sample {
                    track: TrackId::new(track_id.value()),
                    sample,
                },
                Timestamp::ZERO,
            )
            .unwrap();
    }
    mixed.finish().unwrap();
    while let Some(MuxOutput::Segment(segment)) = mixed.poll() {
        mixed_out.push(segment);
    }

    assert_eq!(
        mixed_out, oracle_out,
        "an un-drained feed batch followed by a partial poll on ONE Segmenter instance must \
         deliver exactly the same segments, in the same order, as a plain feed/poll run — \
         nothing lost, nothing duplicated"
    );
}

/// `LlSegmenter`: same mixed-API proof as
/// [`segmenter_mixed_api_delivers_each_output_exactly_once`], for the chunk
/// output.
#[test]
fn ll_segmenter_mixed_api_delivers_each_output_exactly_once() {
    let track = audio_track_spec();
    let samples = audio_samples(50);

    let mut oracle = LlSegmenter::new(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
        4,
    )
    .unwrap();
    let mut oracle_out: Vec<transmux::mux_stage::MuxOutput> = Vec::new();
    for (track_id, sample) in samples.clone() {
        oracle.push(TrackId::new(track_id.value()), sample).unwrap();
        drain_ll(&mut oracle, &mut oracle_out);
    }
    oracle.flush().unwrap();
    drain_ll(&mut oracle, &mut oracle_out);
    assert!(
        oracle_out.len() >= 4,
        "fixture must cut several chunks so an un-drained batch has more \
         than Stage::poll's partial drain can cover"
    );

    let mut mixed =
        LlSegmenter::new(vec![track], timescale(1000), Duration::from_millis(100), 4).unwrap();
    let mut mixed_out: Vec<transmux::mux_stage::MuxOutput> = Vec::new();
    let mut samples = samples.into_iter();
    let batch = 30;
    for (i, (track_id, sample)) in samples.by_ref().take(batch).enumerate() {
        mixed
            .feed(
                MuxInput::Sample {
                    track: TrackId::new(track_id.value()),
                    sample,
                },
                Timestamp::from_nanos(i as u64),
            )
            .unwrap();
    }
    for _ in 0..2 {
        if let Some(c) = mixed.poll() {
            mixed_out.push(c);
        }
    }
    drain_ll(&mut mixed, &mut mixed_out);

    for (track_id, sample) in samples {
        mixed.push(TrackId::new(track_id.value()), sample).unwrap();
        drain_ll(&mut mixed, &mut mixed_out);
    }
    mixed.finish().unwrap();
    while let Some(c) = mixed.poll() {
        mixed_out.push(c);
    }
    drain_ll(&mut mixed, &mut mixed_out);

    assert_eq!(
        mixed_out.len(),
        oracle_out.len(),
        "mixing Stage::feed/poll with the inherent push/take_ready on ONE LlSegmenter instance \
         must deliver the same chunk count as a purely-inherent run — nothing lost, nothing duplicated"
    );
    assert_eq!(
        mixed_out, oracle_out,
        "mixing Stage::feed/poll with the inherent push on ONE LlSegmenter instance \
         must deliver the same chunks, in the same order"
    );
}

/// `LlHlsSegmenter`: same mixed-API proof, across *both* its output channels
/// — parts and segments — via `Stage::Out = LlHlsStageOutput`. This is also
/// the type's first `Stage` test coverage of any kind.
#[test]
fn ll_hls_segmenter_mixed_api_delivers_each_output_exactly_once() {
    let track = audio_track_spec();
    let samples = audio_samples(50);

    let mut oracle = LlHlsSegmenter::with_part_target(
        vec![track.clone()],
        timescale(1000),
        Duration::from_millis(100),
        Duration::from_millis(30),
    )
    .unwrap();
    let mut oracle_parts: Vec<PartInfo> = Vec::new();
    let mut oracle_segments: Vec<SegmentInfo> = Vec::new();
    for (track_id, sample) in samples.clone() {
        oracle.push(TrackId::new(track_id.value()), sample).unwrap();
        oracle_parts.extend(oracle.take_ready_parts());
        oracle_segments.extend(oracle.take_ready_segments());
    }
    oracle.flush().unwrap();
    oracle_parts.extend(oracle.take_ready_parts());
    oracle_segments.extend(oracle.take_ready_segments());
    assert!(
        oracle_parts.len() >= 4,
        "fixture must emit several parts so an un-drained batch has more \
         than Stage::poll's partial drain can cover"
    );
    assert!(
        oracle_segments.len() >= 4,
        "fixture must emit several segments so an un-drained batch has more \
         than Stage::poll's partial drain can cover"
    );

    // Same un-drained-batch/partial-poll/inherent-catch-up shape as the two
    // tests above, but splitting `Stage::poll`'s output across both channels
    // by variant, and using `take_ready_parts`/`take_ready_segments` for the
    // inherent catch-up.
    let mut mixed = LlHlsSegmenter::with_part_target(
        vec![track],
        timescale(1000),
        Duration::from_millis(100),
        Duration::from_millis(30),
    )
    .unwrap();
    let mut mixed_parts: Vec<PartInfo> = Vec::new();
    let mut mixed_segments: Vec<SegmentInfo> = Vec::new();
    let mut samples = samples.into_iter();
    let batch = 30;
    for (i, (track_id, sample)) in samples.by_ref().take(batch).enumerate() {
        mixed
            .feed(
                MuxInput::Sample {
                    track: TrackId::new(track_id.value()),
                    sample,
                },
                Timestamp::from_nanos(i as u64),
            )
            .unwrap();
    }
    for _ in 0..2 {
        if let Some(out) = mixed.poll() {
            match out {
                LlHlsStageOutput::Part(p) => mixed_parts.push(p),
                LlHlsStageOutput::Segment(s) => mixed_segments.push(s),
                _ => unreachable!(
                    "LlHlsStageOutput is non_exhaustive but only two variants exist today"
                ),
            }
        }
    }
    // The inherent drains must see whatever the batch above left ready.
    mixed_parts.extend(mixed.take_ready_parts());
    mixed_segments.extend(mixed.take_ready_segments());

    for (track_id, sample) in samples {
        mixed.push(TrackId::new(track_id.value()), sample).unwrap();
        mixed_parts.extend(mixed.take_ready_parts());
        mixed_segments.extend(mixed.take_ready_segments());
    }
    mixed.finish().unwrap();
    while let Some(out) = mixed.poll() {
        match out {
            LlHlsStageOutput::Part(p) => mixed_parts.push(p),
            LlHlsStageOutput::Segment(s) => mixed_segments.push(s),
            _ => {
                unreachable!("LlHlsStageOutput is non_exhaustive but only two variants exist today")
            }
        }
    }
    mixed_parts.extend(mixed.take_ready_parts());
    mixed_segments.extend(mixed.take_ready_segments());
    assert!(mixed.take_ready_parts().is_empty());
    assert!(mixed.take_ready_segments().is_empty());

    assert_eq!(
        mixed_parts.len(),
        oracle_parts.len(),
        "mixing Stage::feed/poll with the inherent push/take_ready_parts/take_ready_segments on \
         ONE LlHlsSegmenter instance must deliver the same part count as a purely-inherent run — \
         nothing lost, nothing duplicated"
    );
    for (m, o) in mixed_parts.iter().zip(oracle_parts.iter()) {
        assert_eq!(m.bytes, o.bytes);
        assert_eq!(m.duration, o.duration);
        assert_eq!(m.independent, o.independent);
        assert_eq!(m.segment_seq, o.segment_seq);
        assert_eq!(m.part_index, o.part_index);
    }

    assert_eq!(
        mixed_segments.len(),
        oracle_segments.len(),
        "mixing Stage::feed/poll with the inherent push/take_ready_parts/take_ready_segments on \
         ONE LlHlsSegmenter instance must deliver the same segment count as a purely-inherent run \
         — nothing lost, nothing duplicated"
    );
    for (m, o) in mixed_segments.iter().zip(oracle_segments.iter()) {
        assert_eq!(m.bytes, o.bytes);
        assert_eq!(m.duration, o.duration);
        assert_eq!(m.segment_seq, o.segment_seq);
        assert_eq!(m.part_count, o.part_count);
    }
}
