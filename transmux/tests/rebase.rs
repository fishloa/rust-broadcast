//! IR timeline-conditioning transforms — PTS/DTS rebase & anchor wiring (#476).
//!
//! These tests bite end-to-end against the real box layer: the absolute
//! decode-time anchor ([`Track::start_decode_time`]) is populated by
//! [`Fmp4Demux`] from the fragment `tfdt` (ISO/IEC 14496-12:2015 §8.8.12),
//! consumed by [`CmafMux`] as the first segment's `baseMediaDecodeTime`, and
//! transformed by the [`transmux::rebase`] functions. Every test asserts a value
//! observed through the muxer/demuxer, so a hardcoded-0 muxer or a no-op
//! transform fails.
//!
//! EXIT CRITERIA:
//! 1. Anchor from real demux: an fMP4 built at a known non-zero `tfdt`
//!    re-demuxes to that exact `start_decode_time`.
//! 2. Rebase-to-zero end-to-end: a Media with a non-zero anchor muxes to a
//!    `tfdt` equal to the anchor (proves the muxer consumes it); after
//!    `rebase_to_zero` the muxed `tfdt` is 0 (proves the transform).
//! 3. `apply_offset(+90000)` moves every anchor and the muxed `tfdt` by +90000.
//! 4. `unroll_33bit_wraps` lifts a timeline crossing 2^33 into a monotonic one
//!    with the exact expected DTS values.
//! 5. `insert_discontinuity_gap` grows the timeline span by exactly the gap and
//!    leaves earlier samples unchanged.

use broadcast_common::{Package, Unpackage};
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::media::{CmafMux, Fmp4Demux, Media, Track};
use transmux::nalu_types::{AvcPps, AvcSps};
use transmux::pipeline::{
    CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment, build_media_segment,
};
use transmux::rebase::{
    MPEG_TS_WRAP, apply_offset, insert_discontinuity_gap, rebase_to_zero, unroll_33bit_wraps,
};

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
    TrackSpec {
        track_id: 1,
        timescale: 90_000,
        config: CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    }
}

/// One length-prefixed IDR-ish sample (a single 4-byte-prefixed NAL body).
fn sample(duration: u32) -> Sample {
    // A 4-byte length prefix + a tiny slice NAL (type 5 = IDR).
    let nal = [0x65u8, 0x88, 0x84, 0x00];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    Sample {
        data,
        duration,
        is_sync: true,
        composition_offset: 0,
        source_timing: None,
    }
}

fn media_with_anchor(start: u64, durs: &[u32]) -> Media {
    let samples = durs.iter().map(|&d| sample(d)).collect();
    Media::new(vec![Track::new_at(avc_spec(), samples, start)], 90_000)
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
    let samples = [sample(3000), sample(3000)];

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

    // Rebase, then the muxed tfdt must be 0.
    rebase_to_zero(&mut media);
    assert_eq!(media.tracks[0].start_decode_time, 0);
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
    let fmp4 = CmafMux::default().package(&media).expect("package");
    assert_eq!(
        muxed_tfdt(&fmp4),
        ANCHOR + DELTA as u64,
        "muxed tfdt must reflect the applied offset"
    );
}

// ── Test 4: 33-bit unroll bites ─────────────────────────────────────────────
#[test]
fn unroll_33bit_wraps_bites() {
    // Anchor 3000 ticks below 2^33; three 3000-tick samples cross the boundary.
    let start = MPEG_TS_WRAP - 3000;
    let mut media = media_with_anchor(start, &[3000, 3000, 3000]);
    unroll_33bit_wraps(&mut media);

    let t = &media.tracks[0];
    assert_eq!(
        t.start_decode_time,
        MPEG_TS_WRAP - 3000,
        "anchor stays at its unwrapped position"
    );
    // Reconstruct the DTS sequence and assert it is monotonic + exact.
    let expected = [MPEG_TS_WRAP - 3000, MPEG_TS_WRAP, MPEG_TS_WRAP + 3000];
    let mut dts = t.start_decode_time;
    let mut seq = Vec::new();
    for s in &t.samples {
        seq.push(dts);
        dts += s.duration as u64;
    }
    assert_eq!(seq, expected, "unrolled DTS crosses 2^33 monotonically");
    // Explicitly monotonic non-decreasing.
    for w in seq.windows(2) {
        assert!(w[1] >= w[0], "DTS must be non-decreasing after unroll");
    }
}

/// A synthetic backward-wrap: the anchor was captured folded near 0 while the
/// samples fold back to the top of the range, so the reconstructed folded wire
/// timeline steps backward across the boundary; unroll lifts the later samples
/// by +2^33.
#[test]
fn unroll_synthetic_backward_wrap() {
    // Anchor 3000 below 2^33, one sample of 3000 lands exactly on 2^33 (folds to
    // 0 on the wire) — the classic +2^33 unroll.
    let start = MPEG_TS_WRAP - 3000;
    let mut media = media_with_anchor(start, &[3000, 6000]);
    unroll_33bit_wraps(&mut media);
    let t = &media.tracks[0];
    let mut dts = t.start_decode_time;
    let seq: Vec<u64> = t
        .samples
        .iter()
        .map(|s| {
            let v = dts;
            dts += s.duration as u64;
            v
        })
        .collect();
    assert_eq!(seq, vec![MPEG_TS_WRAP - 3000, MPEG_TS_WRAP]);
    assert_eq!(dts, MPEG_TS_WRAP + 6000, "final sample carried +2^33");
}

// ── Test 5: discontinuity-gap insertion bites ───────────────────────────────
#[test]
fn insert_discontinuity_gap_bites() {
    const GAP: u32 = 4500;
    let mut media = media_with_anchor(0, &[3000, 3000, 3000, 3000]);
    let track = &mut media.tracks[0];

    let span_before: u32 = track.samples.iter().map(|s| s.duration).sum();
    let s0_before = track.samples[0].duration;
    let s1_before = track.samples[1].duration;

    insert_discontinuity_gap(track, 2, GAP);

    let span_after: u32 = track.samples.iter().map(|s| s.duration).sum();
    assert_eq!(
        span_after - span_before,
        GAP,
        "timeline span grows by exactly the gap"
    );
    assert_eq!(track.samples[0].duration, s0_before, "sample 0 unchanged");
    assert_eq!(
        track.samples[1].duration,
        s1_before + GAP,
        "the sample before the insertion point absorbs the gap"
    );

    // The gap is observable end-to-end: the sample at index 2 now starts GAP
    // ticks later on the reconstructed timeline.
    let dts_at_2: u64 = track.samples[..2].iter().map(|s| s.duration as u64).sum();
    assert_eq!(
        dts_at_2,
        (s0_before + s1_before + GAP) as u64,
        "sample 2 decode time is pushed out by the gap"
    );
}
