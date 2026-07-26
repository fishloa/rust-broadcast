//! IR timeline-conditioning transforms — PTS/DTS rebase & anchor wiring (#476),
//! on the **absolute** timing model (media plane step 2c).
//!
//! These tests bite end-to-end against the real box layer: the absolute
//! decode-time anchor ([`Track::start_decode_time`]) is populated by
//! [`Fmp4Demux`] from the fragment `tfdt` (ISO/IEC 14496-12:2015 §8.8.12),
//! consumed by [`CmafMux`] as the first segment's `baseMediaDecodeTime`, and
//! transformed by the [`transmux::rebase`] functions. Every test asserts a value
//! observed through the muxer/demuxer, so a hardcoded-0 muxer or a no-op
//! transform fails.
//!
//! Since step 2c each `Sample` carries its own absolute `dts`/`pts`, so every
//! transform must move the anchor **and** the samples in lockstep — asserted
//! here. 33-bit wrap-unrolling is no longer a transform in this module at all:
//! it happens once, at the demux edge, and is gated by
//! `tests/absolute_timing.rs`.
//!
//! EXIT CRITERIA:
//! 1. Anchor from real demux: an fMP4 built at a known non-zero `tfdt`
//!    re-demuxes to that exact `start_decode_time` *and* first-sample `dts`.
//! 2. Rebase-to-zero end-to-end: a Media with a non-zero anchor muxes to a
//!    `tfdt` equal to the anchor (proves the muxer consumes it); after
//!    `rebase_to_zero` the muxed `tfdt` is 0 (proves the transform), and every
//!    sample's absolute `dts` moved with it.
//! 3. `apply_offset(+90000)` moves every anchor, every sample `dts`, and the
//!    muxed `tfdt` by +90000.
//! 4. `insert_discontinuity_gap` pushes the sample at the insertion point (and
//!    every later one) out by exactly the gap, leaving earlier samples alone.
//! 5. A `None`-timed (section-carried) sample is never given a fabricated
//!    timestamp by any transform.

use broadcast_common::{Package, Unpackage};
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::media::{CmafMux, Fmp4Demux, Media, Track};
use transmux::nalu_types::{AvcPps, AvcSps};
use transmux::pipeline::{
    CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment, build_media_segment,
};
use transmux::rebase::{apply_offset, insert_discontinuity_gap, rebase_to_zero};

/// A minimal but real AVC track spec (track_id=1, 90 kHz) so `build_init_segment`
/// emits a valid `avc1`/`avcC` the demuxer can round-trip.
fn avc_spec() -> TrackSpec {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        // Real-shaped SPS/PPS NALs (Baseline 66, 16x16-ish).
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
        1,
        90_000,
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

/// One length-prefixed IDR-ish sample (a single 4-byte-prefixed NAL body) at an
/// absolute decode time of `dts` ticks.
fn sample_at(dts: i64, duration: u32) -> Sample {
    // A 4-byte length prefix + a tiny slice NAL (type 5 = IDR).
    let nal = [0x65u8, 0x88, 0x84, 0x00];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    Sample::new(data, Some(dts), Some(dts), Some(duration), true)
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
    Media::new(vec![Track::new_at(avc_spec(), samples, start)], 90_000)
}

/// The absolute dts of every sample in track 0.
fn dts_seq(media: &Media) -> Vec<Option<i64>> {
    media.tracks[0].samples.iter().map(|s| s.dts).collect()
}

/// Parse the first `moof`/`traf`/`tfdt` baseMediaDecodeTime out of an fMP4.
fn muxed_tfdt(fmp4: &[u8]) -> u64 {
    use transmux::movie_fragment::MovieFragmentBox;
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

    // Build a real init + media segment at a KNOWN non-zero tfdt.
    let mut fmp4 = build_init_segment(std::slice::from_ref(&spec), 90_000).expect("init");
    let frag = FragmentTrackData {
        track_id: 1,
        base_media_decode_time: KNOWN_BASE,
        samples: &samples,
    };
    let media_seg = build_media_segment(1, &[frag]).expect("media segment");
    fmp4.extend_from_slice(&media_seg);

    // Re-demux and read the anchor back — must equal the tfdt, not 0.
    let media = Fmp4Demux::new().unpackage(&fmp4).expect("demux");
    assert_eq!(media.tracks.len(), 1);
    assert_eq!(
        media.tracks[0].start_decode_time, KNOWN_BASE,
        "Fmp4Demux must set start_decode_time from the first fragment tfdt"
    );
    // media plane step 2c: the samples themselves are absolute, seeded from the
    // same tfdt — the anchor is no longer the only place the time lives.
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
    let before = CmafMux::default().package(&media).expect("package");
    assert_eq!(
        muxed_tfdt(&before),
        ANCHOR,
        "muxed tfdt must equal the track anchor before rebase"
    );

    // Rebase: the anchor, and every sample's absolute dts, move to 0.
    rebase_to_zero(&mut media);
    assert_eq!(media.tracks[0].start_decode_time, 0);
    assert_eq!(
        dts_seq(&media),
        vec![Some(0), Some(3000), Some(6000)],
        "rebase_to_zero must move the samples, not just the anchor"
    );
    let after = CmafMux::default().package(&media).expect("package");
    assert_eq!(
        muxed_tfdt(&after),
        0,
        "muxed tfdt must be 0 after rebase_to_zero"
    );
}

// ── Test 3: offset bites through the muxer ──────────────────────────────────
#[test]
fn apply_offset_bites() {
    const ANCHOR: u64 = 100_000;
    const DELTA: i64 = 90_000;
    let mut media = media_with_anchor(ANCHOR, &[3000, 3000]);
    apply_offset(&mut media, DELTA);
    assert_eq!(media.tracks[0].start_decode_time, ANCHOR + DELTA as u64);
    assert_eq!(
        dts_seq(&media),
        vec![
            Some(ANCHOR as i64 + DELTA),
            Some(ANCHOR as i64 + DELTA + 3000)
        ],
        "apply_offset must shift every sample's absolute dts too"
    );
    let fmp4 = CmafMux::default().package(&media).expect("package");
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
    let mut media = media_with_anchor(0, &[3000, 3000, 3000, 3000]);
    let track = &mut media.tracks[0];

    insert_discontinuity_gap(track, 2, GAP);

    // Samples before the insertion point are untouched; the one at the
    // insertion point and everything after it is pushed out by exactly GAP.
    let seq: Vec<Option<i64>> = track.samples.iter().map(|s| s.dts).collect();
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
    let mut media = media_with_anchor(1000, &[100, 100]);
    let track = &mut media.tracks[0];
    insert_discontinuity_gap(track, 0, GAP);
    assert_eq!(track.start_decode_time, 1000 + u64::from(GAP));
    assert_eq!(
        track.samples.iter().map(|s| s.dts).collect::<Vec<_>>(),
        vec![Some(1250), Some(1350)]
    );
}

// ── Test 5: transforms never fabricate a timestamp ──────────────────────────

/// A section-carried sample legitimately has `dts`/`pts` of `None`. No
/// transform may invent one (media plane step 2c) — the whole point of the
/// `Option` is that "no timestamp" survives conditioning.
#[test]
fn transforms_never_fabricate_a_timestamp() {
    let mut media = media_with_anchor(5000, &[100, 100]);
    // Make the second sample untimed, as a section-carried sample would be.
    media.tracks[0].samples[1].dts = None;
    media.tracks[0].samples[1].pts = None;

    rebase_to_zero(&mut media);
    assert_eq!(media.tracks[0].samples[1].dts, None, "rebase_to_zero");
    assert_eq!(media.tracks[0].samples[1].pts, None, "rebase_to_zero");

    apply_offset(&mut media, 1234);
    assert_eq!(media.tracks[0].samples[1].dts, None, "apply_offset");
    assert_eq!(media.tracks[0].samples[1].pts, None, "apply_offset");

    insert_discontinuity_gap(&mut media.tracks[0], 1, 999);
    assert_eq!(
        media.tracks[0].samples[1].dts, None,
        "insert_discontinuity_gap"
    );
    assert_eq!(
        media.tracks[0].samples[1].pts, None,
        "insert_discontinuity_gap"
    );
}
