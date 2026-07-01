//! Real-fixture round-trip tests for AVC/HEVC decoder configuration boxes.
//!
//! These tests scan for `avcC` and `hvcC` four-CCs in real fragmented MP4 files
//! and verify that parsing the box body and serializing it back produces
//! byte-identical output. The oracle bytes come from ffmpeg.
//!
//! Fixtures: `fixtures/mp4/h264_high.mp4` (avcC), `fixtures/mp4/hevc_main.mp4` (hvcC).

use broadcast_common::{Parse, Serialize};
use transmux::{AVCDecoderConfigurationRecord, HEVCDecoderConfigurationRecord};

/// Linear-scan for a box four-CC and return the box body (bytes after the 8-byte header).
fn find_box_body<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
    let needle = four_cc.as_slice();
    let pos = data
        .windows(4)
        .position(|w| w == needle)
        .expect("box four-CC must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start + 8..start + size]
}

// ---------------------------------------------------------------------------
// avcC from h264_high.mp4
// ---------------------------------------------------------------------------

#[test]
fn avcc_h264_high_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/h264_high.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let avcc_body = find_box_body(&data, b"avcC");

    let record = AVCDecoderConfigurationRecord::parse(avcc_body)
        .expect("should parse avcC from h264_high.mp4");

    // --- Value assertions against the oracle (brief AC 1) ---
    assert_eq!(record.configuration_version, 1, "configurationVersion");
    assert_eq!(
        record.profile_indication, 0x64,
        "AVCProfileIndication=100 (High)"
    );
    assert_eq!(record.level_indication, 0x0d, "AVCLevelIndication=13 (1.3)");
    assert_eq!(record.length_size_minus_one, 3, "4-byte NAL length");
    assert!(!record.sps.is_empty(), "≥1 SPS");
    assert!(!record.pps.is_empty(), "≥1 PPS");

    // High profile → extension present
    assert_eq!(record.chroma_format, Some(1), "chroma_format=1 (4:2:0)");
    assert_eq!(record.bit_depth_luma_minus8, Some(0), "8-bit luma");
    assert_eq!(record.bit_depth_chroma_minus8, Some(0), "8-bit chroma");

    // --- Byte-exact round-trip ---
    let serialized = record.to_bytes();
    assert_eq!(
        serialized.len(),
        avcc_body.len(),
        "avcC serialized length matches original"
    );
    assert_eq!(
        serialized, avcc_body,
        "avcC round-trip must be byte-identical"
    );
}

#[test]
fn avcc_h264_high_mutation_changes_bytes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/h264_high.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let avcc_body = find_box_body(&data, b"avcC");

    let mut record = AVCDecoderConfigurationRecord::parse(avcc_body).expect("should parse avcC");

    let original = record.to_bytes();
    record.level_indication = 50;
    let mutated = record.to_bytes();

    assert_ne!(
        mutated, original,
        "mutating level_indication must change serialized bytes (no raw passthrough)"
    );
    assert_eq!(mutated[3], 50, "mutated level appears at byte offset 3");
}

// ---------------------------------------------------------------------------
// hvcC from hevc_main.mp4
// ---------------------------------------------------------------------------

#[test]
fn hvcc_hevc_main_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/hevc_main.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let hvcc_body = find_box_body(&data, b"hvcC");

    let record = HEVCDecoderConfigurationRecord::parse(hvcc_body)
        .expect("should parse hvcC from hevc_main.mp4");

    // --- Value assertions against the oracle (brief AC 2) ---
    assert_eq!(record.configuration_version, 1, "configurationVersion");
    assert_eq!(record.general_profile_idc, 1, "Main profile");
    assert_eq!(record.general_profile_space, 0);
    assert!(!record.general_tier_flag, "main tier");
    assert_eq!(record.chroma_format_idc, 1, "chromaFormat=1 (4:2:0)");
    assert_eq!(record.bit_depth_luma_minus8, 0, "8-bit luma");
    assert_eq!(record.bit_depth_chroma_minus8, 0, "8-bit chroma");
    assert!(!record.arrays.is_empty(), "≥1 NAL array");
    assert_eq!(record.arrays.len(), 3, "VPS/SPS/PPS = 3 arrays");

    // --- Byte-exact round-trip ---
    let serialized = record.to_bytes();
    assert_eq!(
        serialized.len(),
        hvcc_body.len(),
        "hvcC serialized length matches original"
    );
    assert_eq!(
        serialized,
        hvcc_body,
        "hvcC round-trip must be byte-identical; first diff at offset {}",
        serialized
            .iter()
            .zip(hvcc_body.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0)
    );
}

#[test]
fn hvcc_hevc_main_mutation_changes_bytes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/hevc_main.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let hvcc_body = find_box_body(&data, b"hvcC");

    let mut record = HEVCDecoderConfigurationRecord::parse(hvcc_body).expect("should parse hvcC");

    let original = record.to_bytes();
    record.general_level_idc = 80;
    let mutated = record.to_bytes();

    assert_ne!(
        mutated, original,
        "mutating general_level_idc must change serialized bytes (no raw passthrough)"
    );
    // Level is byte 12 (0-indexed) in the body: configVersion(1) + profile_byte(1)
    // + compat_flags(4) + constraint_flags(6) = 12
    assert_eq!(mutated[12], 80, "mutated level appears at byte offset 12");
}
