//! Dummy codec configs and track specs shared by the transmux segmenter tests.

use transmux::pipeline::{CodecConfig, TrackSpec};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, DecoderConfigDescriptor,
    DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor, TrackId,
};

pub fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![transmux::AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

pub fn dummy_esds() -> EsdsBox {
    EsdsBox::new(ESDescriptor::new(
        1,
        0,
        Some(DecoderConfigDescriptor::new(
            0x40,
            0x05,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(vec![0x12, 0x10])),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ))
}

pub fn video_track() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(1),
        Timescale::from_nonzero(90_000),
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    )
}

pub fn audio_track() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(2),
        Timescale::from_nonzero(48_000),
        CodecConfig::Aac {
            esds: dummy_esds(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    )
}
