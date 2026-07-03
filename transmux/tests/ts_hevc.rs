//! `TsDemux` HEVC gate — H.265-in-TS → hub `Media` IR (issue #467).
//!
//! Ground truth is ffprobe on the committed fixtures:
//! - `fixtures/ts/hevc/main.ts`   — HEVC Main   profile, 320x240, 8-bit 4:2:0.
//! - `fixtures/ts/hevc/main10.ts` — HEVC Main 10 profile, 320x240, 10-bit 4:2:0.
//!
//! (`ffprobe -v error -select_streams v -show_entries
//!   stream=codec_name,width,height,profile,pix_fmt fixtures/ts/hevc/main.ts`)
//!
//! Every test is written to *bite*: a stub returning `None` fails "track present";
//! the dims / bit-depth / hvcC contents are checked against the external oracle,
//! not hardcoded to whatever the demuxer happens to emit.

use std::path::PathBuf;

use broadcast_common::{Package, Parse, Unpackage};
use transmux::TsDemux;
use transmux::hevc_config::HEVCDecoderConfigurationRecord;
use transmux::media::{CmafMux, Media};
use transmux::pipeline::CodecConfig;
use transmux::validate::{Severity, validate_init_segment, validate_media_segment};

// ── Fixture loading ─────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/hevc")
}

fn load_ts(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    let data = std::fs::read(&path).unwrap_or_else(|_| panic!("{name} fixture must exist"));
    assert_eq!(
        data.len() % 188,
        0,
        "TS file must be whole 188-byte packets"
    );
    data
}

/// The single HEVC video track from a demuxed Media (there is exactly one).
fn hevc_track(media: &Media) -> &transmux::media::Track {
    let hevc: Vec<&transmux::media::Track> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .collect();
    assert_eq!(
        hevc.len(),
        1,
        "must demux exactly one HEVC video track (was 0 before #467)"
    );
    hevc[0]
}

/// HEVC NAL unit type from the 2-byte header: `(byte0 >> 1) & 0x3F`.
fn hevc_nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| (b >> 1) & 0x3F)
}

// ── Test 1: real HEVC-in-TS demux, dims + well-formed hvcC ───────────────────

/// A track is produced (previously HEVC was skipped → None), its dimensions
/// match ffprobe (320x240), and the `hvcC` carries VPS/SPS/PPS NAL arrays.
#[test]
fn demuxes_hevc_track_with_ffprobe_dims_and_wellformed_hvcc() {
    let ts = load_ts("main.ts");
    let media: Media = TsDemux::new().unpackage(&ts).expect("demux must succeed");

    let track = hevc_track(&media);
    let (config, width, height) = match &track.spec.config {
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => (config, *width, *height),
        other => panic!("expected HEVC config, got {other:?}"),
    };

    // ffprobe ground truth: 320x240.
    assert_eq!(
        (width, height),
        (320, 240),
        "HEVC dimensions must be decoded from the in-band SPS (ffprobe: 320x240)"
    );

    // The hvcC must round-trip through the record parser and carry the parameter
    // sets. Serialize the box body, re-parse it, and inspect the NAL arrays.
    let record = &config.config;
    assert_eq!(
        record.configuration_version, 1,
        "hvcC configurationVersion must be 1"
    );

    // Collect NAL-array types present.
    let types: Vec<u8> = record.arrays.iter().map(|a| a.nal_unit_type).collect();
    assert!(types.contains(&32), "hvcC must carry a VPS array (type 32)");
    assert!(
        types.contains(&33),
        "hvcC must carry an SPS array (type 33)"
    );
    assert!(types.contains(&34), "hvcC must carry a PPS array (type 34)");

    // Each array carries at least one NAL, and the NAL header type matches the
    // array type (proves they were sorted into the correct arrays, not dumped).
    for arr in &record.arrays {
        assert!(
            !arr.nalus.is_empty(),
            "NAL array (type {}) must carry at least one NAL",
            arr.nal_unit_type
        );
        for nal in &arr.nalus {
            assert_eq!(
                hevc_nal_type(&nal.0),
                Some(arr.nal_unit_type),
                "NAL header type must match its hvcC array type"
            );
        }
    }

    // Byte-serialize the record and re-parse it: proves the built hvcC is
    // well-formed (parseable), not an arbitrary blob.
    let bytes = {
        use broadcast_common::Serialize;
        let mut buf = vec![0u8; record.serialized_len()];
        record.serialize_into(&mut buf).expect("serialize hvcC");
        buf
    };
    let reparsed = HEVCDecoderConfigurationRecord::parse(&bytes).expect("hvcC must re-parse");
    assert_eq!(&reparsed, record, "hvcC must round-trip byte-identically");

    // Main profile → general_profile_idc = 1 (ffprobe: profile=Main), 8-bit.
    assert_eq!(
        record.general_profile_idc, 1,
        "Main profile → profile_idc 1"
    );
    assert_eq!(
        record.bit_depth_luma_minus8, 0,
        "Main profile is 8-bit → bit_depth_luma_minus8 = 0"
    );
}

// ── Test 2: keyframe flags ───────────────────────────────────────────────────

/// The first access unit is IRAP → is_sync; not every AU is a sync sample
/// (there are TRAIL frames between IRAPs).
#[test]
fn first_au_is_irap_and_not_all_sync() {
    let ts = load_ts("main.ts");
    let media = TsDemux::new().unpackage(&ts).expect("demux");
    let track = hevc_track(&media);

    assert!(!track.samples.is_empty(), "HEVC track must carry samples");
    assert!(
        track.samples[0].is_sync,
        "first access unit must be an IRAP keyframe (is_sync)"
    );
    let sync_count = track.samples.iter().filter(|s| s.is_sync).count();
    assert!(
        sync_count >= 1,
        "at least one sync sample (the IRAP): got {sync_count}"
    );
    assert!(
        sync_count < track.samples.len(),
        "not every AU can be a sync sample — there must be TRAIL frames \
         (samples={}, sync={sync_count})",
        track.samples.len()
    );
}

// ── Test 3: TS → IR → CMAF, validated ────────────────────────────────────────

/// Mux the demuxed HEVC Media to CMAF and run the conformance validator: zero
/// `Severity::Error`, and the init segment's video sample entry is `hvc1` with
/// an `hvcC` box.
#[test]
fn ts_to_cmaf_validates_with_hvc1_sample_entry() {
    let ts = load_ts("main.ts");
    let media = TsDemux::new().unpackage(&ts).expect("demux");

    let cmaf = CmafMux::default().package(&media).expect("package to CMAF");

    // Validator: the whole CMAF blob carries ftyp+moov (init) and moof+mdat
    // (media). Both validators must report zero errors (warnings are allowed).
    let init_issues = validate_init_segment(&cmaf);
    let init_errors: Vec<_> = init_issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(
        init_errors.is_empty(),
        "init segment must have zero errors, got {init_errors:?}"
    );

    let media_issues = validate_media_segment(&cmaf);
    let media_errors: Vec<_> = media_issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(
        media_errors.is_empty(),
        "media segment must have zero errors, got {media_errors:?}"
    );

    // The init segment's video sample entry must be `hvc1` and contain an `hvcC`.
    assert!(
        contains_ascii(&cmaf, b"hvc1"),
        "CMAF init must carry an hvc1 sample entry"
    );
    assert!(
        contains_ascii(&cmaf, b"hvcC"),
        "CMAF init must carry an hvcC config box"
    );

    // Round-trip through our own Fmp4Demux: the recovered config is HEVC with the
    // same dims — proves the IR is identical whether HEVC came from TS or fMP4.
    let round: Media = transmux::Fmp4Demux::new()
        .unpackage(&cmaf)
        .expect("re-parse our CMAF");
    let round_hevc = hevc_track(&round);
    match &round_hevc.spec.config {
        CodecConfig::Hevc { width, height, .. } => {
            assert_eq!((*width, *height), (320, 240), "round-trip dims preserved");
        }
        other => panic!("round-trip track must be HEVC, got {other:?}"),
    }
    assert_eq!(
        round_hevc.samples.len(),
        hevc_track(&media).samples.len(),
        "round-trip sample count preserved"
    );
}

/// Whether `haystack` contains the 4-byte ASCII tag `needle`.
fn contains_ascii(haystack: &[u8], needle: &[u8; 4]) -> bool {
    haystack.windows(4).any(|w| w == needle)
}

// ── Test 4: 10-bit variant ───────────────────────────────────────────────────

/// `main10.ts` (Main 10 profile) → the decoded SPS / hvcC reflects 10-bit depth.
#[test]
fn main10_reflects_ten_bit_depth() {
    let ts = load_ts("main10.ts");
    let media = TsDemux::new().unpackage(&ts).expect("demux");
    let track = hevc_track(&media);

    let record = match &track.spec.config {
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => {
            assert_eq!(
                (*width, *height),
                (320, 240),
                "main10 dims (ffprobe 320x240)"
            );
            &config.config
        }
        other => panic!("expected HEVC config, got {other:?}"),
    };

    // ffprobe: profile=Main 10, pix_fmt=yuv420p10le → 10-bit luma & chroma.
    assert_eq!(
        record.bit_depth_luma_minus8, 2,
        "Main 10 profile → bit_depth_luma_minus8 = 2 (10-bit)"
    );
    assert_eq!(
        record.bit_depth_chroma_minus8, 2,
        "Main 10 profile → bit_depth_chroma_minus8 = 2 (10-bit)"
    );
    assert_eq!(
        record.general_profile_idc, 2,
        "Main 10 profile → general_profile_idc = 2"
    );
}
