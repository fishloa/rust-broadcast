//! `TrackSpec::display` (sample aspect ratio) on the demo
//! (`docs/display-geometry/DESIGN.md` Phase 1a).
//!
//! Two real anamorphic SD captures carry the two ways broadcasters signal a
//! 16:9 picture in a 720x576 frame:
//!
//! - `melodie-anamorphic-sar-64-45-zenith-tv.ts`: VUI `aspect_ratio_idc` 255
//!   (`Extended_SAR`, `sar_width` 64, `sar_height` 45).
//! - `krone-anamorphic-sar-16-11-zenith-tv.ts`: `aspect_ratio_idc` 4, i.e. the
//!   Table E-1 row `16:11` ("720x576 16:9 frame with horizontal overscan").
//!
//! Each is checked through both TS demux entry points (batch [`TsDemux`] and
//! the streaming [`StreamingTsDemux`], whose `TrackAdded` / `TrackUpdated`
//! carry a `TrackSpec`). Fixtures are loaded through `test_fixtures`; the
//! private submodule is skipped, not failed, when it is not checked out.

use transmux::ir::{AspectRatioIdc, CodecConfig};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};

/// 720x576: the SD frame size of both zenith-tv captures (no frame cropping:
/// `frame_cropping_flag` 0 in both SPS).
const SD_WIDTH: u32 = 720;
const SD_HEIGHT: u32 = 576;

fn read(fixture: &str) -> Option<Vec<u8>> {
    let path = test_fixtures::require(fixture, "display_geometry")?;
    Some(std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())))
}

/// The `(coded size, aspect_ratio, SAR)` of every AVC `TrackSpec` the streaming
/// demuxer announces (each `TrackAdded` / `TrackUpdated`).
fn streaming_video_specs(data: &[u8]) -> Vec<transmux::ir::TrackSpec> {
    let mut demux = StreamingTsDemux::new();
    let mut specs = Vec::new();
    for chunk in data.chunks(188 * 512) {
        demux.feed(chunk);
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    if matches!(spec.config, CodecConfig::Avc { .. }) {
                        specs.push(spec);
                    }
                }
                _ => {}
            }
        }
    }
    demux.finish();
    while let Some(ev) = demux.poll_event() {
        if let DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) = ev {
            if matches!(spec.config, CodecConfig::Avc { .. }) {
                specs.push(spec);
            }
        }
    }
    specs
}

fn batch_video_specs(data: &[u8]) -> Vec<transmux::ir::TrackSpec> {
    let media = TsDemux::new().demux(data).expect("batch demux");
    media
        .tracks
        .iter()
        .filter(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .map(|t| t.spec.clone())
        .collect()
}

fn check(fixture: &str, aspect_ratio: AspectRatioIdc, sar: (u16, u16)) {
    let Some(data) = read(fixture) else {
        return;
    };
    // Batch: the video track's spec carries the geometry of its codec config.
    let batch = batch_video_specs(&data);
    assert!(!batch.is_empty(), "{fixture}: no AVC track");
    for spec in &batch {
        let g = spec
            .display
            .as_ref()
            .unwrap_or_else(|| panic!("{fixture}: batch TrackSpec.display is None"));
        assert_eq!((g.coded_width, g.coded_height), (SD_WIDTH, SD_HEIGHT));
        assert_eq!(g.aspect_ratio, Some(aspect_ratio.clone()), "{fixture}");
        assert_eq!(g.sample_aspect_ratio(), Some(sar), "{fixture}");
    }
    // Streaming: the same, from the announced spec (nothing in this capture
    // changes its geometry, so this is the `TrackAdded` spec).
    let streaming = streaming_video_specs(&data);
    assert!(!streaming.is_empty(), "{fixture}: no AVC TrackAdded");
    for spec in &streaming {
        let g = spec
            .display
            .as_ref()
            .unwrap_or_else(|| panic!("{fixture}: streaming TrackSpec.display is None"));
        assert_eq!((g.coded_width, g.coded_height), (SD_WIDTH, SD_HEIGHT));
        assert_eq!(g.aspect_ratio, Some(aspect_ratio.clone()), "{fixture}");
        assert_eq!(g.sample_aspect_ratio(), Some(sar), "{fixture}");
    }
}

#[test]
fn melodie_extended_sar_64_45() {
    check(
        "ts/melodie-anamorphic-sar-64-45-zenith-tv.ts",
        AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        },
        (64, 45),
    );
}

#[test]
fn krone_table_sar_16_11() {
    check(
        "ts/krone-anamorphic-sar-16-11-zenith-tv.ts",
        AspectRatioIdc::Sar16x11,
        (16, 11),
    );
}
