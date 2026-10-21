#![cfg(feature = "std")]
//! `nominal_frame_rate_millihertz` — H.264 SPS VUI timing only.

use broadcast_common::Parse;
use transmux::CodecConfig;
use transmux::ts_demux::TsDemux;
use transmux::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord, decode_hevc_vui_timing, nominal_frame_rate};
use transmux::nalu_types::{HevcNalArray, HevcNalUnit};

// Re-exported? nominal_frame_rate is in transmux root.

// Re-exported? nominal_frame_rate is in transmux root.
const HEVC_SPS_NUT: u8 = 33;

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

fn extract_hevc_sps(path: &str) -> Vec<u8> {
    let file = std::fs::read(path).expect("fixture must exist");
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
    let sps_array = record
        .arrays
        .iter()
        .find(|a| a.nal_unit_type == HEVC_SPS_NUT)
        .expect("SPS array");
    sps_array.nalus[0].0.clone()
}

#[test]
fn hevc_vui_hrd_fields() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_hrd.mp4"
    ));
    let t = decode_hevc_vui_timing(&sps).unwrap().unwrap();
    assert_eq!(t.num_units_in_tick, 1);
    assert_eq!(t.time_scale, 25);
    assert!(!t.field_seq);
    assert!(!t.frame_field_info_present);
    assert_eq!(t.hrd_fixed_pic_rate, Some(1));
}

#[test]
fn hevc_vui_ntsc_fields() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_ntsc.mp4"
    ));
    let t = decode_hevc_vui_timing(&sps).unwrap().unwrap();
    assert_eq!(t.num_units_in_tick, 1001);
    assert_eq!(t.time_scale, 30000);
    assert!(!t.field_seq);
    assert!(!t.frame_field_info_present);
    assert_eq!(t.hrd_fixed_pic_rate, None);
}

#[test]
fn hevc_vui_field_fields() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_field.mp4"
    ));
    let t = decode_hevc_vui_timing(&sps).unwrap().unwrap();
    assert_eq!(t.num_units_in_tick, 1);
    assert_eq!(t.time_scale, 25);
    assert!(t.field_seq);
    assert!(t.frame_field_info_present);
    assert_eq!(t.hrd_fixed_pic_rate, None);
}

#[test]
fn hevc_vui_field_hrd_fields() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_field_hrd.mp4"
    ));
    let t = decode_hevc_vui_timing(&sps).unwrap().unwrap();
    assert_eq!(t.num_units_in_tick, 1);
    assert_eq!(t.time_scale, 25);
    assert!(t.field_seq);
    assert!(t.frame_field_info_present);
    assert_eq!(t.hrd_fixed_pic_rate, Some(1));
}

fn hevc_config(sps: Vec<u8>) -> CodecConfig {
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
        arrays: vec![HevcNalArray {
            array_completeness: true,
            nal_unit_type: 33,
            nalus: vec![HevcNalUnit(sps)],
        }],
    };
    CodecConfig::Hevc {
        config: HEVCConfigurationBox::new(record),
        width: 320,
        height: 240,
    }
}

#[test]
fn hevc_nominal_rate_hrd() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_hrd.mp4"
    ));
    assert_eq!(
        nominal_frame_rate(&hevc_config(sps)),
        Some((25_000, transmux::FrameRateBasis::Normative))
    );
}

#[test]
fn hevc_nominal_rate_ntsc() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_ntsc.mp4"
    ));
    assert_eq!(
        nominal_frame_rate(&hevc_config(sps)),
        Some((29_970, transmux::FrameRateBasis::Convention))
    );
}

#[test]
fn hevc_nominal_rate_field() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_field.mp4"
    ));
    assert_eq!(nominal_frame_rate(&hevc_config(sps)), None);
}

#[test]
fn hevc_nominal_rate_field_hrd() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_vui_field_hrd.mp4"
    ));
    assert_eq!(
        nominal_frame_rate(&hevc_config(sps)),
        Some((12_500, transmux::FrameRateBasis::Normative))
    );
}

#[test]
fn hevc_nominal_rate_main() {
    let sps = extract_hevc_sps(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_main.mp4"
    ));
    assert_eq!(
        nominal_frame_rate(&hevc_config(sps)),
        Some((25_000, transmux::FrameRateBasis::Convention))
    );
}

#[test]
fn real_h264_fixture_with_vui_timing() {
    let data = test_fixtures::require_bytes("ts/h264_aac.ts", "frame_rate").expect("fixture");
    let media = TsDemux::new().demux(&data).expect("demux");
    let spec = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    assert_eq!(
        transmux::nominal_frame_rate_millihertz(&spec.spec.config),
        Some(25_000)
    );
}

#[test]
fn h264_fixture_basis() {
    let data = test_fixtures::require_bytes("ts/h264_aac.ts", "frame_rate").expect("fixture");
    let media = TsDemux::new().demux(&data).expect("demux");
    let spec = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    // Report basis for the committed H.264 fixture:
    assert_eq!(
        transmux::nominal_frame_rate(&spec.spec.config),
        Some((25_000, transmux::FrameRateBasis::Convention))
    );
}
#[test]
fn missing_sps_yields_none() {
    use transmux::AVCConfigurationBox;
    let cfg = CodecConfig::Avc {
        config: AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
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
        }),
        width: 640,
        height: 360,
    };
    assert_eq!(transmux::nominal_frame_rate_millihertz(&cfg), None);
}
