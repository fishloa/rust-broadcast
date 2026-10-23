#![cfg(feature = "std")]
//! HLS FRAME-RATE derivation from track codec config (Apple 9.15).

use std::num::NonZeroUsize;

use broadcast_hls::MasterPlaylist;
use hls_runtime::server::{Container, HlsOrigin};
use media_plane::trunk::{Trunk, TrunkConfig};
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
    let data = test_fixtures::require_bytes(fixture, "frame_rate")?;
    let media = TsDemux::new().demux(&data).ok()?;
    Some(media.tracks.into_iter().map(|t| t.spec).collect())
}

#[test]
fn h264_fixture_with_vui_timing_emits_frame_rate() {
    let Some(specs) = specs_from_fixture("ts/h264_aac.ts") else {
        return;
    };
    let origin = origin();
    origin.set_track_specs(&specs).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(
        parsed.variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(25_000))
    );
}

#[test]
fn audio_only_origin_has_no_frame_rate() {
    let origin = origin();
    origin.set_track_specs(&[]).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(parsed.variants[0].frame_rate, None);
}

#[test]
fn maximum_across_video_tracks_wins() {
    let specs25 = specs_from_fixture("ts/h264_aac.ts").unwrap_or_default();
    let specs30 =
        specs_from_fixture("scte35-ssai/ts/video_with_scte35_splice_insert.ts").unwrap_or_default();
    if specs25.is_empty() || specs30.is_empty() {
        return;
    }
    // Take the video spec from each fixture.
    let v25 = specs25
        .into_iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video spec");
    let v30 = specs30
        .into_iter()
        .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
        .expect("video spec");
    let origin = origin();
    // unsorted input: track ids 2 then 1
    let mut a = v30;
    a.track_id = transmux::TrackId::new(2);
    let mut b = v25;
    b.track_id = transmux::TrackId::new(1);
    origin.set_track_specs(&[a, b]).unwrap();
    let parsed = MasterPlaylist::parse(&render(&origin)).expect("parse master");
    assert_eq!(
        parsed.variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(30_000))
    );
}

#[test]
fn set_init_reset_clears_frame_rate() {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .container(Container::Fmp4)
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .build()
        .expect("build");
    let specs = specs_from_fixture("ts/h264_aac.ts").unwrap_or_default();
    if specs.is_empty() {
        return;
    }
    origin.set_track_specs(&specs).unwrap();
    assert_eq!(
        MasterPlaylist::parse(&render(&origin)).unwrap().variants[0].frame_rate,
        Some(broadcast_hls::FrameRate::from_millihertz(25_000))
    );
    origin.set_init(vec![0xAAu8; 16]);
    assert_eq!(
        MasterPlaylist::parse(&render(&origin)).unwrap().variants[0].frame_rate,
        None
    );
}
