//! IR timeline-conditioning stages — PTS/DTS rebase & anchor wiring (#476),
//! on the **absolute** timing model (media plane step 2c).
//!
//! These tests bite end-to-end against the real box layer: the absolute
//! decode-time anchor ([`Track::start_decode_time`]) is populated by the
//! MP4 demux from the fragment `tfdt` (ISO/IEC 14496-12:2026 §8.8.12),
//! consumed by [`CmafMuxStage`](transmux::media::CmafMuxStage) as the first segment's `baseMediaDecodeTime`, and
//! transformed by the [`transmux::rebase`] stages. Every test asserts a value
//! observed through the muxer/demuxer, so a hardcoded-0 muxer or a no-op
//! stage fails.
//!
//! EXIT CRITERIA:
//! 1. Anchor from real demux: an fMP4 built at a known non-zero `tfdt`
//!    re-demuxes to that exact `start_decode_time` *and* first-sample `dts`.
//! 2. Rebase-to-zero end-to-end: after [`RebaseStage`] the muxed `tfdt` is 0,
//!    and every sample's absolute `dts` moved with it.
//! 3. [`OffsetStage`] moves every sample `dts` and the muxed `tfdt`.
//! 4. [`GapStage`] pushes the sample at the insertion point (and every later
//!    one) out by exactly the gap, leaving earlier samples alone.
//! 5. A `None`-timed (section-carried) sample is never given a fabricated
//!    timestamp by any stage.

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use broadcast_core::timescale::Timescale;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use isobmff::nalu_types::{AvcPps, AvcSps};
use test_harness::collect::{Collected as Media, CollectedTrack as Track};
use transmux::TrackId;
use transmux::mux_stage::MuxInput;
use transmux::pipeline::{
    CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment, build_media_segment,
};
use transmux::rebase::{GapStage, OffsetStage, RebaseOrigin, RebaseStage};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

/// A minimal but real AVC track spec (track_id=1, 90 kHz) so `build_init_segment`
/// emits a valid `avc1`/`avcC` the demuxer can round-trip.
fn avc_spec() -> TrackSpec {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![
            0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24,
        ])],
        pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    TrackSpec::new(
        TrackId::new(1),
        timescale(90_000),
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

/// One length-prefixed IDR-ish sample at an absolute decode time of `dts` ticks.
fn sample_at(dts: i64, duration: u32) -> Sample {
    let nal = [0x65u8, 0x88, 0x84, 0x00];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    Sample::new(data, Some(dts), Some(dts), Some(duration), true).expect("valid timing")
}

/// Build a one-track `Media` anchored at `start`, whose samples carry
/// consecutive **absolute** dts/pts stepping by each duration in `durs`.
fn media_with_anchor(start: u64, durs: &[u32]) -> Media {
    let mut dts = start as i64;
    let mut samples = Vec::with_capacity(durs.len());
    for &d in durs {
        samples.push(sample_at(dts, d));
        dts += i64::from(d);
    }
    Media::new(
        vec![Track::new_at(avc_spec(), samples, start)],
        timescale(90_000),
    )
}

/// Drive `stage` over `track`'s samples and fold them back into a track whose
/// anchor is the first emitted sample's `dts`.
fn apply_pair<S>(stage: &mut S, track: &Track) -> Track
where
    S: for<'a> Stage<In<'a> = (TrackId, Sample), Out = (TrackId, Sample), Error = transmux::Error>,
{
    let mut out = Vec::new();
    for sample in &track.samples {
        stage
            .feed((track.spec.track_id, sample.clone()), Timestamp::ZERO)
            .expect("feed");
    }
    stage.finish().expect("finish");
    while let Some((_, sample)) = stage.poll() {
        out.push(sample);
    }
    let start = out
        .iter()
        .find_map(|s| s.timing.and_then(|t| t.dts()))
        .map(|dts| dts.max(0) as u64)
        .unwrap_or(track.start_decode_time);
    Track::new_at(track.spec.clone(), out, start)
}

/// Drive a [`RebaseStage`] (fed `MuxInput`) over `track` and fold its samples
/// back into a track whose anchor is the first emitted sample's `dts`.
fn apply<S>(stage: &mut S, track: &Track) -> Track
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxInput, Error = transmux::Error>,
{
    stage
        .feed(
            MuxInput::Track(Box::new(track.spec.clone())),
            Timestamp::ZERO,
        )
        .expect("announce");
    for sample in &track.samples {
        stage
            .feed(
                MuxInput::Sample {
                    track: track.spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("feed");
    }
    stage.finish().expect("finish");
    let mut out = Vec::new();
    while let Some(item) = stage.poll() {
        if let MuxInput::Sample { sample, .. } = item {
            out.push(sample);
        }
    }
    let start = out
        .iter()
        .find_map(|s| s.timing.and_then(|t| t.dts()))
        .map(|dts| dts.max(0) as u64)
        .unwrap_or(track.start_decode_time);
    Track::new_at(track.spec.clone(), out, start)
}

/// The absolute dts of every sample in track 0.
fn dts_seq(media: &Media) -> Vec<Option<i64>> {
    media.tracks[0]
        .samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.dts()))
        .collect()
}

/// Parse the first `moof`/`traf`/`tfdt` baseMediaDecodeTime out of an fMP4.
fn muxed_tfdt(fmp4: &[u8]) -> u64 {
    use isobmff::movie_fragment::MovieFragmentBox;
    let mut off = 0usize;
    while off + 8 <= fmp4.len() {
        let sz =
            u32::from_be_bytes([fmp4[off], fmp4[off + 1], fmp4[off + 2], fmp4[off + 3]]) as usize;
        if sz < 8 {
            break;
        }
        if &fmp4[off + 4..off + 8] == b"moof" {
            let moof = MovieFragmentBox::parse_body(&fmp4[off + 8..off + sz]).expect("parse moof");
            return moof.traf[0]
                .tfdt
                .as_ref()
                .expect("traf must carry a tfdt")
                .base_media_decode_time();
        }
        off += sz;
    }
    panic!("no moof in muxed fMP4");
}

// ── Test 1: anchor populated from a real demux ──────────────────────────────
#[test]
fn fmp4_demux_populates_start_decode_time_from_tfdt() {
    const KNOWN_BASE: u64 = 123_456;
    let spec = avc_spec();
    let samples = [sample_at(0, 3000), sample_at(3000, 3000)];

    let mut fmp4 =
        build_init_segment(std::slice::from_ref(&spec), timescale(90_000)).expect("init");
    let frag = FragmentTrackData::new(TrackId::new(1), KNOWN_BASE, &samples);
    let media_seg = build_media_segment(1, &[frag]).expect("media segment");
    fmp4.extend_from_slice(&media_seg);

    let media =
        test_harness::collect::media_from_mp4(&fmp4, test_harness::collect::CollectMode::Seekable)
            .expect("demux");
    assert_eq!(media.tracks.len(), 1);
    assert_eq!(
        media.tracks[0].start_decode_time, KNOWN_BASE,
        "the MP4 demux must set start_decode_time from the first fragment tfdt"
    );
    assert_eq!(
        dts_seq(&media),
        vec![Some(KNOWN_BASE as i64), Some(KNOWN_BASE as i64 + 3000)],
        "sample dts must be absolute, anchored on the fragment tfdt"
    );
}

// ── Test 2: rebase-to-zero, observed through the muxer ──────────────────────
#[test]
fn rebase_to_zero_end_to_end() {
    const ANCHOR: u64 = 900_000;
    let mut media = media_with_anchor(ANCHOR, &[3000, 3000, 3000]);

    // Before rebase: the muxer must emit the anchor as the tfdt (proves it is
    // wired, not hardcoded 0).
    let before = crate::cmaf::package(&media).expect("package");
    assert_eq!(
        muxed_tfdt(&before),
        ANCHOR,
        "muxed tfdt must equal the track anchor before rebase"
    );

    let track_id = media.tracks[0].spec.track_id;
    let mut stage = RebaseStage::with_origin(RebaseOrigin::new().with(track_id, ANCHOR as i64));
    media.tracks[0] = apply(&mut stage, &media.tracks[0]);
    assert_eq!(media.tracks[0].start_decode_time, 0);
    assert_eq!(
        dts_seq(&media),
        vec![Some(0), Some(3000), Some(6000)],
        "rebase must move the samples, not just the anchor"
    );
    let after = crate::cmaf::package(&media).expect("package");
    assert_eq!(muxed_tfdt(&after), 0, "muxed tfdt must be 0 after rebase");
}

// ── Test 3: offset bites through the muxer ──────────────────────────────────
#[test]
fn apply_offset_bites() {
    const ANCHOR: u64 = 100_000;
    const DELTA: i64 = 90_000;
    let mut media = media_with_anchor(ANCHOR, &[3000, 3000]);
    let mut stage = OffsetStage::new(DELTA);
    media.tracks[0] = apply_pair(&mut stage, &media.tracks[0]);
    assert_eq!(media.tracks[0].start_decode_time, ANCHOR + DELTA as u64);
    assert_eq!(
        dts_seq(&media),
        vec![
            Some(ANCHOR as i64 + DELTA),
            Some(ANCHOR as i64 + DELTA + 3000)
        ],
        "the offset stage must shift every sample's absolute dts too"
    );
    let fmp4 = crate::cmaf::package(&media).expect("package");
    assert_eq!(
        muxed_tfdt(&fmp4),
        ANCHOR + DELTA as u64,
        "muxed tfdt must reflect the applied offset"
    );
}

// ── Test 4: discontinuity-gap insertion bites ───────────────────────────────
#[test]
fn insert_discontinuity_gap_bites() {
    const GAP: u32 = 4500;
    let media = media_with_anchor(0, &[3000, 3000, 3000, 3000]);
    let mut stage = GapStage::new(GAP, 2);
    let track = apply_pair(&mut stage, &media.tracks[0]);

    let seq: Vec<Option<i64>> = track
        .samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.dts()))
        .collect();
    assert_eq!(
        seq,
        vec![
            Some(0),
            Some(3000),
            Some(6000 + i64::from(GAP)),
            Some(9000 + i64::from(GAP)),
        ],
        "the gap must push the insertion point and every later sample out by exactly GAP"
    );
    assert_eq!(
        track.start_decode_time, 0,
        "a mid-track gap leaves the anchor alone"
    );
}

/// A gap at index 0 has no preceding sample, so it shifts the whole track —
/// anchor included — keeping the two in lockstep.
#[test]
fn insert_discontinuity_gap_at_zero_shifts_anchor_and_samples() {
    const GAP: u32 = 250;
    let media = media_with_anchor(1000, &[100, 100]);
    let mut stage = GapStage::new(GAP, 0);
    let track = apply_pair(&mut stage, &media.tracks[0]);
    assert_eq!(track.start_decode_time, 1000 + u64::from(GAP));
    assert_eq!(
        track
            .samples
            .iter()
            .map(|s| s.timing.and_then(|t| t.dts()))
            .collect::<Vec<_>>(),
        vec![Some(1250), Some(1350)]
    );
}

// ── Test 5: stages never fabricate a timestamp ──────────────────────────────

/// A section-carried sample legitimately has `dts`/`pts` of `None`. No stage
/// may invent one (media plane step 2c).
#[test]
fn transforms_never_fabricate_a_timestamp() {
    let mut media = media_with_anchor(5000, &[100, 100]);
    media.tracks[0].samples[1].timing = None;
    let track_id = media.tracks[0].spec.track_id;

    let mut rebase = RebaseStage::with_origin(RebaseOrigin::new().with(track_id, 5000));
    media.tracks[0] = apply(&mut rebase, &media.tracks[0]);
    assert_eq!(
        media.tracks[0].samples[1].timing.and_then(|t| t.dts()),
        None,
        "rebase"
    );
    assert_eq!(
        media.tracks[0].samples[1].timing.and_then(|t| t.pts()),
        None,
        "rebase"
    );

    let mut offset = OffsetStage::new(1234);
    media.tracks[0] = apply_pair(&mut offset, &media.tracks[0]);
    assert_eq!(
        media.tracks[0].samples[1].timing.and_then(|t| t.dts()),
        None,
        "offset"
    );
    assert_eq!(
        media.tracks[0].samples[1].timing.and_then(|t| t.pts()),
        None,
        "offset"
    );

    let mut gap = GapStage::new(999, 1);
    media.tracks[0] = apply_pair(&mut gap, &media.tracks[0]);
    assert_eq!(
        media.tracks[0].samples[1].timing.and_then(|t| t.dts()),
        None,
        "gap"
    );
    assert_eq!(
        media.tracks[0].samples[1].timing.and_then(|t| t.pts()),
        None,
        "gap"
    );
}
