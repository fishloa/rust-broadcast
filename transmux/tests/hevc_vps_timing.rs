#![cfg(feature = "std")]
//! HEVC VPS timing extraction (transmux/docs/codec/hevc-vps-timing.md).

use broadcast_common::Parse;
use transmux::CodecConfig;
use transmux::nalu_types::{HevcNalArray, HevcNalUnit};
use transmux::{
    HEVCConfigurationBox, HEVCDecoderConfigurationRecord, decode_hevc_vps_timing,
    nominal_frame_rate,
};

fn find_box<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        if size < 8 {
            break;
        }
        let ty = &data[offset + 4..offset + 8];
        let body = &data[offset + 8..offset + size];
        if ty == four_cc {
            return Some(body);
        }
        offset += size;
    }
    None
}

fn extract_nal(path_rel: &str, nut: u8) -> Vec<u8> {
    let file = test_fixtures::read(path_rel);
    let moov = find_box(&file, b"moov").expect("moov");
    let trak = find_box(moov, b"trak").expect("trak");
    let mdia = find_box(trak, b"mdia").expect("mdia");
    let minf = find_box(mdia, b"minf").expect("minf");
    let stbl = find_box(minf, b"stbl").expect("stbl");
    let stsd_body = find_box(stbl, b"stsd").expect("stsd");
    let entries = &stsd_body[8..];
    let hvc1_body = find_box(entries, b"hev1").expect("hev1");
    let config_region = &hvc1_body[78..];
    let hvcc_body = find_box(config_region, b"hvcC").expect("hvcC");
    let record = HEVCDecoderConfigurationRecord::parse(hvcc_body).expect("hvcC parse");
    record
        .arrays
        .iter()
        .find(|a| a.nal_unit_type == nut)
        .expect("array")
        .nalus[0]
        .0
        .clone()
}

#[test]
fn hevc_vps_timing_fixture_decodes() {
    let vps = extract_nal("mp4/hevc_vps_timing.mp4", 32);
    let t = decode_hevc_vps_timing(&vps)
        .expect("parse")
        .expect("timing present");
    assert_eq!(t.num_units_in_tick, 1001);
    assert_eq!(t.time_scale, 30000);
}

#[test]
fn hevc_vps_timing_fixture_nominal_rate() {
    let sps = extract_nal("mp4/hevc_vps_timing.mp4", 33);
    let vps = extract_nal("mp4/hevc_vps_timing.mp4", 32);
    let record = HEVCDecoderConfigurationRecord {
        configuration_version: 1,
        general_profile_space: 0,
        general_tier_flag: false,
        general_profile_idc: 1,
        general_profile_compatibility_flags: 0,
        general_constraint_indicator_flags: 0,
        general_level_idc: 93,
        min_spatial_segmentation_idc: 0,
        parallelism_type: 0,
        chroma_format_idc: 1,
        bit_depth_luma_minus8: 0,
        bit_depth_chroma_minus8: 0,
        avg_frame_rate: 0,
        constant_frame_rate: 0,
        num_temporal_layers: 1,
        temporal_id_nested: true,
        length_size_minus_one: 3,
        arrays: vec![
            HevcNalArray {
                array_completeness: true,
                nal_unit_type: 32,
                nalus: vec![HevcNalUnit(vps)],
            },
            HevcNalArray {
                array_completeness: true,
                nal_unit_type: 33,
                nalus: vec![HevcNalUnit(sps)],
            },
        ],
    };
    let cfg = CodecConfig::Hevc {
        config: HEVCConfigurationBox::new(record),
        width: 320,
        height: 240,
    };
    assert_eq!(
        nominal_frame_rate(&cfg),
        Some((29970, transmux::FrameRateBasis::Convention))
    );
}

#[test]
fn hevc_vps_timing_other_fixtures_none() {
    for name in [
        "mp4/hevc_main.mp4",
        "mp4/hevc_vui_hrd.mp4",
        "mp4/hevc_vui_ntsc.mp4",
        "mp4/hevc_vui_field.mp4",
        "mp4/hevc_vui_field_hrd.mp4",
    ] {
        if let Some(vps) = extract_vps_optional(name) {
            let r = decode_hevc_vps_timing(&vps).expect("parse");
            assert!(r.is_none(), "{}: expected None, got {:?}", name, r);
        }
    }
}

fn extract_vps_optional(path_rel: &str) -> Option<Vec<u8>> {
    let file = std::fs::read(test_fixtures::path(path_rel)).ok()?;
    let moov = find_box(&file, b"moov")?;
    let trak = find_box(moov, b"trak")?;
    let mdia = find_box(trak, b"mdia")?;
    let minf = find_box(mdia, b"minf")?;
    let stbl = find_box(minf, b"stbl")?;
    let stsd_body = find_box(stbl, b"stsd")?;
    let entries = &stsd_body[8..];
    let hvc1_body = find_box(entries, b"hev1")?;
    let config_region = &hvc1_body[78..];
    let hvcc_body = find_box(config_region, b"hvcC")?;
    let record = HEVCDecoderConfigurationRecord::parse(hvcc_body).ok()?;
    record
        .arrays
        .iter()
        .find(|a| a.nal_unit_type == 32)?
        .nalus
        .first()
        .map(|n| n.0.clone())
}

#[test]
fn hevc_vps_timing_truncated_never_panics() {
    let vps = extract_nal("mp4/hevc_vps_timing.mp4", 32);
    for i in 0..=vps.len() {
        let r = std::panic::catch_unwind(|| decode_hevc_vps_timing(&vps[..i]));
        assert!(r.is_ok(), "panicked at prefix len {}", i);
    }
}
