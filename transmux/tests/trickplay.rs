//! Integration tests for the I-frame-only trick-play track transform (#477).
//!
//! Three biting tests:
//!
//! 1. **Synthetic**: a hand-built 8-sample track with sync at indices 0 and 4
//!    verifies the duration-folding arithmetic; a naïve filter that keeps
//!    durations unchanged FAILS the duration-conservation assert, and a raw
//!    passthrough FAILS the sample-count assert.
//!
//! 2. **Real fixture**: `av_frag.mp4` (AVC+AAC fMP4) is demuxed into a
//!    [`Media`]; the video track is passed through [`derive_iframe_track`] and
//!    the result is validated for count thinning, `is_sync` on every sample,
//!    and total-duration conservation.
//!
//! 3. **No-sync error**: a track with no sync samples returns
//!    [`Error::InvalidInput`] rather than producing an empty track.

use std::fs;
use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::error::Error;
use transmux::media::{Fmp4Demux, Track};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::trickplay::derive_iframe_track;

/// Duration used for each synthetic sample (arbitrary, but all equal makes the
/// expected folded durations trivial to verify: count × per-sample duration).
const SYNTHETIC_SAMPLE_DURATION: u32 = 100;

/// Number of synthetic samples; sync at indices 0 and 4.
const SYNTHETIC_SAMPLE_COUNT: usize = 8;

/// Index of the second sync sample in the synthetic track.
const SYNTHETIC_SYNC2_INDEX: usize = 4;

/// Number of samples in each "GOP" for the synthetic track (4 non-sync after
/// each keyframe, so each keyframe spans 4 × SYNTHETIC_SAMPLE_DURATION).
const SYNTHETIC_GOP_SIZE: usize = 4;

fn synthetic_spec() -> TrackSpec {
    TrackSpec {
        track_id: 1,
        timescale: 90000,
        config: CodecConfig::Avc {
            config: AVCConfigurationBox {
                config: AVCDecoderConfigurationRecord {
                    configuration_version: 1,
                    profile_indication: 66,
                    profile_compatibility: 0,
                    level_indication: 30,
                    length_size_minus_one: 3,
                    sps: vec![],
                    pps: vec![],
                    chroma_format: None,
                    bit_depth_luma_minus8: None,
                    bit_depth_chroma_minus8: None,
                    sps_ext: vec![],
                },
            },
            width: 1280,
            height: 720,
        },
    }
}

fn synthetic_track() -> Track {
    let samples: Vec<Sample> = (0..SYNTHETIC_SAMPLE_COUNT)
        .map(|i| Sample {
            data: vec![i as u8; 4],
            duration: SYNTHETIC_SAMPLE_DURATION,
            is_sync: i == 0 || i == SYNTHETIC_SYNC2_INDEX,
            composition_offset: 0,
        })
        .collect();
    Track::new(synthetic_spec(), samples)
}

fn av_frag_fixture() -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "mp4",
        "cmaf",
        "av_frag.mp4",
    ]
    .iter()
    .collect();
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Test 1 — Synthetic: duration folding bites
// ---------------------------------------------------------------------------

/// Verify the derive_iframe_track transform on a hand-built 8-sample track.
///
/// Source layout (each cell = one sample, "K" = keyframe):
/// ```text
/// idx:   0(K)  1     2     3     4(K)  5     6     7
/// dur:   100   100   100   100   100   100   100   100
/// ```
///
/// Expected output (2 kept samples, durations folded to cover the gap to the
/// next keyframe / end of track):
/// ```text
/// kept:  0(K)                   4(K)
/// dur:   400                    400
/// ```
///
/// Total source duration = 800; total derived duration = 800 (conserved).
///
/// **Why it bites:**
/// - A naïve filter keeping per-sample durations unchanged yields durations of
///   [100, 100], making `derived_total = 200 ≠ source_total = 800`.
/// - A raw passthrough (keeping all 8 samples) yields `count = 8 ≠ 2`.
#[test]
fn synthetic_duration_folding() {
    let src = synthetic_track();
    let source_total: u64 = src.samples.iter().map(|s| s.duration as u64).sum();

    let trick = derive_iframe_track(&src).expect("derive must succeed");

    // Exactly 2 sync samples are kept.
    assert_eq!(
        trick.samples.len(),
        2,
        "expected exactly 2 kept samples (one per keyframe)"
    );

    // Every output sample is a sync sample.
    for (i, s) in trick.samples.iter().enumerate() {
        assert!(s.is_sync, "output sample {i} must be is_sync");
    }

    // Duration of keyframe 0 spans indices 0..4 → 4 × 100 = 400.
    let expected_dur_0 = (SYNTHETIC_GOP_SIZE as u32) * SYNTHETIC_SAMPLE_DURATION;
    assert_eq!(
        trick.samples[0].duration, expected_dur_0,
        "first keyframe duration must fold in the following non-sync samples"
    );

    // Duration of keyframe 1 (index 4) spans indices 4..8 → 4 × 100 = 400.
    let expected_dur_1 = (SYNTHETIC_GOP_SIZE as u32) * SYNTHETIC_SAMPLE_DURATION;
    assert_eq!(
        trick.samples[1].duration, expected_dur_1,
        "second keyframe duration must fold in the tail non-sync samples"
    );

    // Total timeline is conserved.
    let derived_total: u64 = trick.samples.iter().map(|s| s.duration as u64).sum();
    assert_eq!(
        derived_total, source_total,
        "derived total duration must equal source total duration"
    );

    // Codec bytes are byte-identical to the source sync samples.
    assert_eq!(
        trick.samples[0].data, src.samples[0].data,
        "first kept sample data must be byte-identical to source sample[0]"
    );
    assert_eq!(
        trick.samples[1].data, src.samples[SYNTHETIC_SYNC2_INDEX].data,
        "second kept sample data must be byte-identical to source sample[{SYNTHETIC_SYNC2_INDEX}]"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — Real fixture: demux av_frag.mp4 → derive trick track
// ---------------------------------------------------------------------------

/// Verify derive_iframe_track on the video track of a real fMP4 fixture.
///
/// **Why it bites:**
/// - Asserts `output_count < source_count` — proves the track was actually
///   thinned (guards against a passthrough returning all samples).
/// - Asserts `sum(output.duration) == sum(source.duration)` — proves duration
///   conservation across a real GOP structure.
/// - Asserts every output sample `is_sync` — proves no non-sync samples leaked.
#[test]
fn real_fixture_video_trickplay() {
    let file = av_frag_fixture();
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("demux av_frag.mp4");

    // The fixture has video as the first track.
    let video = media
        .tracks
        .iter()
        .find(|t| {
            matches!(
                t.config(),
                CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
            )
        })
        .expect("fixture must have a video track");

    let source_count = video.samples.len();
    let source_sync_count = video.samples.iter().filter(|s| s.is_sync).count();
    let source_total: u64 = video.samples.iter().map(|s| s.duration as u64).sum();

    // Sanity: the fixture must have at least one sync sample for the test to
    // be meaningful.
    assert!(
        source_sync_count > 0,
        "fixture video track must contain at least one sync sample"
    );

    let trick = derive_iframe_track(video).expect("derive_iframe_track must succeed");

    // Output count equals the number of sync samples in the source.
    assert_eq!(
        trick.samples.len(),
        source_sync_count,
        "derived count must equal source sync-sample count ({source_sync_count})"
    );

    // The track was actually thinned (not a passthrough).
    assert!(
        trick.samples.len() < source_count,
        "derived count ({}) must be less than source count ({source_count})",
        trick.samples.len()
    );

    // Every output sample is a sync sample.
    for (i, s) in trick.samples.iter().enumerate() {
        assert!(s.is_sync, "output sample {i} must have is_sync=true");
    }

    // Total duration is conserved.
    let derived_total: u64 = trick.samples.iter().map(|s| s.duration as u64).sum();
    assert_eq!(
        derived_total, source_total,
        "derived total duration ({derived_total}) must equal source total ({source_total})"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — No-sync: empty sync set → Err
// ---------------------------------------------------------------------------

/// Verify that a track with no sync samples yields an error rather than an
/// empty (and useless) derived track.
#[test]
fn no_sync_samples_returns_error() {
    let samples: Vec<Sample> = (0u8..4)
        .map(|i| Sample {
            data: vec![i],
            duration: 100,
            is_sync: false,
            composition_offset: 0,
        })
        .collect();
    let src = Track::new(synthetic_spec(), samples);

    let err = derive_iframe_track(&src).expect_err("must fail when no sync samples");
    assert!(
        matches!(err, Error::InvalidInput(_)),
        "expected Error::InvalidInput, got {err:?}"
    );
}
