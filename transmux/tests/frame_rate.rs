#![cfg(feature = "std")]
//! `nominal_frame_rate_millihertz` — H.264 SPS VUI timing only.

use transmux::CodecConfig;
use transmux::ts_demux::TsDemux;

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
