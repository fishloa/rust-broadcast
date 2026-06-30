//! Real-fixture round-trip tests for AVC/HEVC decoder configuration records.
//!
//! These tests navigate the container hierarchy in real fragmented MP4 files:
//! `moov → trak → mdia → minf → stbl → stsd → avc1 → avcC` (and `hvc1 → hvcC`)
//! and verify that parsing an existing record and serializing it back produces
//! byte-identical output.

use broadcast_common::{Parse, Serialize};
use transmux::{AVCDecoderConfigurationRecord, HEVCDecoderConfigurationRecord};

/// Find the first box of a given type within `body` bytes.
fn find_box<'a>(body: &'a [u8], four_cc: &[u8; 4]) -> Option<(&'a [u8], usize)> {
    let mut offset = 0usize;
    while offset + 8 <= body.len() {
        let size = u32::from_be_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ]) as usize;
        let ty = &body[offset + 4..offset + 8];
        if size == 0 {
            break;
        }
        if size < 8 {
            break;
        }
        if ty == four_cc {
            // Return the body (after the 8-byte header)
            return Some((&body[offset + 8..offset + size], offset));
        }
        offset += size;
    }
    None
}

/// Navigate: stsd→avc1(or hvc1)→avcC(or hvcC) and return the config box body.
fn extract_config_body(
    file_data: &[u8],
    sample_entry_type: &[u8; 4],
    config_type: &[u8; 4],
) -> Vec<u8> {
    // Helper: open a container by type, get its children
    fn enter_box<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
        let mut offset = 0usize;
        while offset + 8 <= data.len() {
            let size = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            let ty = &data[offset + 4..offset + 8];
            if size == 0 || size < 8 {
                break;
            }
            if ty == four_cc {
                // FullBox: skip 4 extra bytes (version+flags)
                return &data[offset + 8..offset + size];
            }
            offset += size;
        }
        panic!("box {:?} not found", std::str::from_utf8(four_cc).unwrap());
    }

    // Navigate the hierarchy
    let moov = enter_box(file_data, b"moov");
    let trak = enter_box(moov, b"trak");
    let mdia = enter_box(trak, b"mdia");
    let minf = enter_box(mdia, b"minf");
    let stbl = enter_box(minf, b"stbl");
    let stsd_data = enter_box(stbl, b"stsd");

    // stsd is a FullBox. Skip version(1)+flags(3) to get to the entry count + entries.
    // stsd body: version(8) + flags(24) = 4 bytes, then entry_count(32), then entries.
    if stsd_data.len() < 4 + 4 {
        panic!("stsd too short");
    }
    let _version_flags = &stsd_data[0..4];
    let entry_count = u32::from_be_bytes([stsd_data[4], stsd_data[5], stsd_data[6], stsd_data[7]]);
    if entry_count == 0 {
        panic!("no stsd entries");
    }

    // stsd entries start at offset 8
    let mut entries_data = &stsd_data[8..];

    // Find the sample entry box matching our type
    loop {
        if entries_data.len() < 8 {
            break;
        }
        let size = u32::from_be_bytes([
            entries_data[0],
            entries_data[1],
            entries_data[2],
            entries_data[3],
        ]) as usize;
        let ty = &entries_data[4..8];
        if size == 0 || size < 8 {
            break;
        }
        if ty == sample_entry_type {
            // Sample entry body starts at offset 8 (after box header 8 bytes).
            // VisualSampleEntry has 78 bytes of fixed fields before config boxes.
            // So we skip 78 bytes to get to the config box data.
            let entry_body = &entries_data[8..size];
            // After the VisualSampleEntry fixed fields (78 bytes),
            // the config box(es) start.
            let config_region = if entry_body.len() > 78 {
                &entry_body[78..]
            } else {
                panic!("sample entry too short for VisualSampleEntry fields");
            };

            // Find the config box in the config region
            if let Some((config_body, _)) = find_box(config_region, config_type) {
                return config_body.to_vec();
            }
            panic!(
                "config box {:?} not found inside sample entry",
                std::str::from_utf8(config_type).unwrap()
            );
        }
        entries_data = &entries_data[size..];
    }

    panic!(
        "sample entry {:?} not found in stsd",
        std::str::from_utf8(sample_entry_type).unwrap()
    );
}

// ---------------------------------------------------------------------------
// AVC (H.264) fixture tests
// ---------------------------------------------------------------------------

#[test]
fn test_avcc_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    let avcc_body = extract_config_body(&data, b"avc1", b"avcC");

    let record = AVCDecoderConfigurationRecord::parse(&avcc_body)
        .expect("should parse avcC from real fixture");

    // Verify key fields
    assert_eq!(record.configuration_version, 1, "avcC version");
    assert_eq!(record.profile_indication, 100, "High profile");
    assert_eq!(record.level_indication, 13, "level 1.3");
    assert_eq!(record.length_size_minus_one, 3, "4-byte NAL length");

    // High profile (100) → chroma/bit-depth ext present
    assert_eq!(record.chroma_format, Some(1), "chroma_format=1 (4:2:0)");
    assert_eq!(record.bit_depth_luma_minus8, Some(0), "8-bit luma");
    assert_eq!(record.bit_depth_chroma_minus8, Some(0), "8-bit chroma");

    // SPS: 1, PPS: 1, SPSExt: 0
    assert_eq!(record.sps.len(), 1, "one SPS");
    assert_eq!(record.pps.len(), 1, "one PPS");
    assert_eq!(record.sps_ext.len(), 0, "no SPSExt");

    // Round-trip: serialize → byte-identical
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
fn test_avcc_mutation_changes_bytes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let avcc_body = extract_config_body(&data, b"avc1", b"avcC");

    let mut record = AVCDecoderConfigurationRecord::parse(&avcc_body).expect("should parse avcC");

    let original = record.to_bytes();
    record.level_indication = 50;
    let mutated = record.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating level_indication must change serialized bytes"
    );
    assert_eq!(mutated[3], 50, "level at offset 3");
}

// ---------------------------------------------------------------------------
// HEVC (H.265) fixture tests
// ---------------------------------------------------------------------------

#[test]
fn test_hvcc_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/hevc_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    let hvcc_body = extract_config_body(&data, b"hvc1", b"hvcC");

    let record = HEVCDecoderConfigurationRecord::parse(&hvcc_body)
        .expect("should parse hvcC from real fixture");

    // Verify key fields from the fixture
    assert_eq!(record.configuration_version, 1, "hvcC version");
    assert_eq!(record.general_profile_space, 0);
    assert!(!record.general_tier_flag, "main tier");
    assert_eq!(record.general_profile_idc, 1, "Main profile");
    assert_eq!(record.general_level_idc, 60, "level 6.0");
    assert_eq!(record.length_size_minus_one, 3, "4-byte NAL length");

    // Check arrays — should have VPS, SPS, PPS at minimum
    assert!(record.arrays.len() >= 3, "at least VPS/SPS/PPS arrays");

    // Round-trip: serialize → byte-identical
    let serialized = record.to_bytes();
    assert_eq!(
        serialized.len(),
        hvcc_body.len(),
        "hvcC serialized length matches original ({} vs {})",
        serialized.len(),
        hvcc_body.len()
    );
    assert_eq!(
        serialized,
        hvcc_body,
        "hvcC round-trip must be byte-identical; first differing byte at offset {}",
        serialized
            .iter()
            .zip(hvcc_body.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0)
    );
}

#[test]
fn test_hvcc_mutation_changes_bytes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/hevc_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let hvcc_body = extract_config_body(&data, b"hvc1", b"hvcC");

    let mut record = HEVCDecoderConfigurationRecord::parse(&hvcc_body).expect("should parse hvcC");

    let original = record.to_bytes();
    record.general_level_idc = 90;
    let mutated = record.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating general_level_idc must change serialized bytes"
    );
}
