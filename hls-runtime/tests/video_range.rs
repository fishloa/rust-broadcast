#![cfg(feature = "std")]
//! HLS VIDEO-RANGE derivation from track display colour (Phase 4b).

use std::num::NonZeroUsize;

use hls_runtime::server::{Container, HlsOrigin};
use media_plane::trunk::{Trunk, TrunkConfig};
use transmux::ir::{ColourDescription, DisplayGeometry, VideoFormat, VideoSignalType};
use transmux::ts_demux::TsDemux;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, TrackId,
    TrackSpec,
};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

fn origin() -> HlsOrigin {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build")
}

fn render(origin: &HlsOrigin) -> String {
    match origin.master_playlist("media.m3u8") {
        Ok(s) => s,
        Err(e) => panic!("master_playlist: {e}"),
    }
}

fn spec_with(track_id: u32, code: Option<u8>, colour: bool, display: bool) -> TrackSpec {
    let mut spec = TrackSpec::new(
        TrackId::new(track_id),
        90_000,
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    );
    if !display {
        return spec;
    }
    let mut g = DisplayGeometry::new(320, 240, None);
    if colour {
        if let Some(c) = code {
            g.video_signal = Some(VideoSignalType::new(
                VideoFormat::Component,
                false,
                Some(ColourDescription::from_wire(1, c, 1)),
            ));
        } else {
            g.video_signal = Some(VideoSignalType::new(VideoFormat::Component, false, None));
        }
    }
    spec = spec.with_display(Some(g));
    spec
}

#[test]
fn pq_transfer_code_renders_video_range_pq() {
    let origin = origin();
    let spec = spec_with(1, Some(16), true, true);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(body.contains("VIDEO-RANGE=PQ"), "body: {body}");
}

#[test]
fn hlg_transfer_code_renders_video_range_hlg() {
    let origin = origin();
    let spec = spec_with(1, Some(18), true, true);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(body.contains("VIDEO-RANGE=HLG"), "body: {body}");
}

#[test]
fn sdr_transfer_code_omits_video_range() {
    let origin = origin();
    let spec = spec_with(1, Some(1), true, true);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn unspecified_transfer_code_2_omits_video_range() {
    let origin = origin();
    let spec = spec_with(1, Some(2), true, true);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn no_colour_description_omits_video_range() {
    let origin = origin();
    let spec = spec_with(1, None, false, true);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn no_display_omits_video_range() {
    let origin = origin();
    let spec = spec_with(1, None, false, false);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn colour_without_transfer_mapping_omits_video_range() {
    // colour description present but transfer code 2 (unspecified) -> no mapping
    let origin = origin();
    let spec = spec_with(1, Some(2), true, true);
    origin.set_track_specs(&[spec]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn first_video_track_by_track_id_wins_pq_then_hlg() {
    let origin = origin();
    let s1 = spec_with(1, Some(16), true, true); // PQ
    let s2 = spec_with(2, Some(18), true, true); // HLG
    // unsorted input
    origin.set_track_specs(&[s2, s1]).unwrap();
    let body = render(&origin);
    assert!(body.contains("VIDEO-RANGE=PQ"), "body: {body}");
    assert!(!body.contains("VIDEO-RANGE=HLG"), "body: {body}");
}

#[test]
fn first_video_track_by_track_id_sdr_masks_later_pq() {
    let origin = origin();
    let s1 = spec_with(1, Some(1), true, true); // SDR -> omitted
    let s2 = spec_with(2, Some(16), true, true); // PQ
    origin.set_track_specs(&[s1, s2]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn first_colour_track_with_unmapped_code_masks_later_pq() {
    let origin = origin();
    let s1 = spec_with(1, Some(2), true, true); // unmapped
    let s2 = spec_with(2, Some(16), true, true); // PQ
    origin.set_track_specs(&[s1, s2]).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn set_init_reset_clears_video_range() {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .container(Container::Fmp4)
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    let spec = spec_with(1, Some(16), true, true);
    origin.set_track_specs(&[spec]).unwrap();
    assert!(render(&origin).contains("VIDEO-RANGE=PQ"));
    // garbage init -> derived None -> reset
    origin.set_init(vec![0xAAu8; 8]);
    assert!(!render(&origin).contains("VIDEO-RANGE"));
}

#[test]
fn real_sdr_capture_omits_video_range() {
    let Some(data) = test_fixtures::require_bytes(
        "ts/melodie-anamorphic-sar-64-45-zenith-tv.ts",
        "video_range",
    ) else {
        return;
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|t| t.spec).collect();
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    origin.set_track_specs(&specs).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}

#[test]
fn real_sdr_krone_omits_video_range() {
    let Some(data) =
        test_fixtures::require_bytes("ts/krone-anamorphic-sar-16-11-zenith-tv.ts", "video_range")
    else {
        return;
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|t| t.spec).collect();
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    origin.set_track_specs(&specs).unwrap();
    let body = render(&origin);
    assert!(!body.contains("VIDEO-RANGE"), "body: {body}");
}
