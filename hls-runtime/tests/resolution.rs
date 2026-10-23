#![cfg(feature = "std")]
//! HLS RESOLUTION derivation from track display geometry (Q8).

use std::num::NonZeroUsize;

use broadcast_hls::MasterPlaylist;
use hls_runtime::server::{Container, HlsOrigin};
use media_plane::trunk::{Trunk, TrunkConfig};
use transmux::ir::AspectRatioIdc;
use transmux::ts_demux::TsDemux;
use transmux::{CodecConfig, TrackSpec};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
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

fn specs_from_fixture(fixture: &str) -> Option<Vec<TrackSpec>> {
    let data = test_fixtures::require_bytes(fixture, "resolution")?;
    let media = TsDemux::new().demux(&data).ok()?;
    Some(media.tracks.into_iter().map(|t| t.spec).collect())
}

#[test]
fn melodie_resolution_1024x576() {
    let Some(specs) = specs_from_fixture("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let origin = origin();
    origin.set_track_specs(&specs).unwrap();
    let body = render(&origin);
    let parsed = MasterPlaylist::parse(&body).expect("parse master");
    assert_eq!(parsed.variants[0].resolution, Some((1024, 576)));
}

#[test]
fn krone_resolution_1047x576() {
    let Some(specs) = specs_from_fixture("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let origin = origin();
    origin.set_track_specs(&specs).unwrap();
    let body = render(&origin);
    let parsed = MasterPlaylist::parse(&body).expect("parse master");
    assert_eq!(parsed.variants[0].resolution, Some((1047, 576)));
}

#[test]
fn square_pixel_fixture_resolution_is_coded_size() {
    let Some(specs) = specs_from_fixture("ts/h264_aac.ts") else {
        return;
    };
    let coded = specs
        .iter()
        .find_map(|s| match &s.config {
            CodecConfig::Avc { width, height, .. } => Some((*width as u32, *height as u32)),
            CodecConfig::Hevc { width, height, .. } => Some((*width as u32, *height as u32)),
            _ => None,
        })
        .expect("video track");
    let origin = origin();
    origin.set_track_specs(&specs).unwrap();
    let body = render(&origin);
    let parsed = MasterPlaylist::parse(&body).expect("parse master");
    assert_eq!(parsed.variants[0].resolution, Some(coded));
}

#[test]
fn audio_only_origin_has_no_resolution() {
    let origin = origin();
    origin.set_track_specs(&[]).unwrap();
    let body = render(&origin);
    let parsed = MasterPlaylist::parse(&body).expect("parse master");
    assert_eq!(parsed.variants[0].resolution, None);
}

#[test]
fn first_video_track_by_track_id_wins_resolution() {
    // Two video tracks: resolution from track 1.
    let s1 = TrackSpec::new(
        transmux::TrackId::new(1),
        90_000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
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
            }),
            width: 640,
            height: 360,
        },
    );
    let g = transmux::ir::DisplayGeometry::new(640, 360, Some(AspectRatioIdc::Square));
    // force coded size only; resolution should be 640x360
    let s1 = s1.with_display(Some(g));
    let s2 = TrackSpec::new(
        transmux::TrackId::new(2),
        90_000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
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
            }),
            width: 1280,
            height: 720,
        },
    );
    let origin = origin();
    // unsorted input
    origin.set_track_specs(&[s2, s1]).unwrap();
    let body = render(&origin);
    let parsed = MasterPlaylist::parse(&body).expect("parse master");
    assert_eq!(parsed.variants[0].resolution, Some((640, 360)));
}

#[test]
fn set_init_reset_clears_resolution() {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .container(Container::Fmp4)
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    let specs =
        specs_from_fixture("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts").unwrap_or_default();
    if specs.is_empty() {
        return;
    }
    origin.set_track_specs(&specs).unwrap();
    assert_eq!(
        MasterPlaylist::parse(&render(&origin)).unwrap().variants[0].resolution,
        Some((1024, 576))
    );
    origin.set_init(vec![0xAAu8; 8]);
    assert_eq!(
        MasterPlaylist::parse(&render(&origin)).unwrap().variants[0].resolution,
        None
    );
}
