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

use broadcast_common::{Package, Unpackage};
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
        if let DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) = ev
            && matches!(spec.config, CodecConfig::Avc { .. })
        {
            specs.push(spec);
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
        assert_eq!(g.aspect_ratio, Some(aspect_ratio), "{fixture}");
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
        assert_eq!(g.aspect_ratio, Some(aspect_ratio), "{fixture}");
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

/// One event of interest, in emission order (`Spec` boxes the rare side).
#[derive(Debug)]
enum Obs {
    Sample {
        track_id: transmux::ir::TrackId,
        width: u16,
        dts: Option<i64>,
    },
    Spec(Box<transmux::ir::TrackSpec>),
}

/// Width of the AVC config last announced for `track_id` (the fixture's first
/// clip is 320x240, the second 640x480).
fn demux_config(sequence: &[Obs], track_id: transmux::ir::TrackId) -> CodecConfig {
    sequence
        .iter()
        .rev()
        .find_map(|o| match o {
            Obs::Spec(spec) if spec.track_id == track_id => Some(spec.config.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no config for track {track_id:?}"))
}

/// The absolute DTS (90 kHz) of the access unit that first carries the second
/// 640x480 SPS/PPS pair (README oracle: keyframe 306000, 3.4 s).
const TWO_RESOLUTIONS_KEYFRAME_DTS: i64 = 306_000;

/// `TrackUpdated` lands between the samples it separates (design §2 / §6.5).
///
/// `h264-two-resolutions.ts` (public fixture, README oracle) concatenates a
/// 320x240 clip and a 640x480 one on one video PID: four `keyint=25` GOPs, so
/// the second SPS/PPS set first travels in the keyframe access unit whose
/// absolute DTS is 306000 ticks (90 kHz: 3.4 s, i.e. the third keyframe).
/// Video samples are emitted one access unit behind, so the event must arrive
/// after every 320x240 `Sample` and before the first 640x480 one.
#[test]
fn track_updated_lands_after_the_last_old_config_sample() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ts/h264-two-resolutions.ts");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut demux = StreamingTsDemux::new();
    let mut sequence = Vec::new();
    for chunk in data.chunks(188 * 512) {
        demux.feed(chunk);
        while let Some(ev) = demux.poll_event() {
            match ev {
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    let CodecConfig::Avc { width, .. } = demux_config(&sequence, track_id) else {
                        continue;
                    };
                    {
                        sequence.push(Obs::Sample {
                            track_id,
                            width,
                            dts: sample.dts,
                        });
                    }
                }
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    sequence.push(Obs::Spec(Box::new(spec)));
                }
                _ => {}
            }
        }
    }
    demux.finish();
    // `sequence` carries every track's samples; the AVC one is the only
    // spec whose config names dimensions.
    let track = sequence
        .iter()
        .find_map(|o| match o {
            Obs::Spec(spec) if matches!(spec.config, CodecConfig::Avc { .. }) => {
                Some(spec.track_id)
            }
            _ => None,
        })
        .expect("the fixture announces an AVC track");
    let video: Vec<&Obs> = sequence
        .iter()
        .filter(|o| match o {
            Obs::Spec(spec) => spec.track_id == track,
            Obs::Sample { track_id, .. } => *track_id == track,
        })
        .collect();
    // `TrackAdded` announces 320x240; the mid-file `TrackUpdated` announces
    // 640x480. Phase 2 parks that update on the access unit that first
    // carries the new SPS (DTS 306000) and releases it one access unit later,
    // with the last 320x240 sample (DTS 302400) — the Phase 1a ordering — so
    // the update is the *last* 320x240 sample before the first 640x480 one.
    let first_640 = video
        .iter()
        .position(|o| matches!(o, Obs::Sample { width: 640, .. }))
        .expect("the update lands before the first 640x480 sample");
    assert!(matches!(video[first_640], Obs::Sample { width: 640, .. }));
    let update_at = first_640;
    let before = &video[..update_at];
    let after = &video[update_at + 1..];
    assert!(
        matches!(before.last(), Some(Obs::Spec(spec)) if matches!(spec.config, CodecConfig::Avc { width: 640, .. })),
        "the event must close the 320x240 era: {before:?}"
    );
    let last_320_before = before[..before.len() - 1]
        .iter()
        .rposition(|o| matches!(o, Obs::Sample { width: 320, .. }))
        .expect("320x240 samples precede the event");
    assert!(
        matches!(
            before[last_320_before],
            Obs::Sample {
                width: 320,
                dts: Some(d),
                ..
            } if *d < TWO_RESOLUTIONS_KEYFRAME_DTS
        ),
        "the event must follow the last 320x240 sample: {before:?}"
    );
    assert_eq!(
        before[last_320_before + 1..].len(),
        1,
        "the event immediately follows that sample: {before:?}"
    );
    let Some(Obs::Sample {
        width: 640,
        dts: Some(dts),
        ..
    }) = after.first()
    else {
        panic!("the event must precede the first 640x480 sample: {after:?}");
    };
    assert_eq!(*dts, TWO_RESOLUTIONS_KEYFRAME_DTS);
}

/// The geometry survives an fMP4 (CMAF) round trip: anamorphic TS input,
/// `CmafMux` then `Fmp4Demux`. Both `TrackSpec`s derive it from the same
/// `avcC` (the muxer rewrites the init segment, so this is not a pass-through
/// of the demuxed spec).
#[test]
fn cmaf_round_trip_keeps_the_sample_aspect_ratio() {
    let Some(data) = read("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let media = TsDemux::new().demux(&data).expect("TS demux");
    let before: Vec<Option<(u16, u16)>> = media
        .tracks
        .iter()
        .map(|t| {
            t.spec
                .display
                .as_ref()
                .and_then(|g| g.sample_aspect_ratio())
        })
        .collect();
    assert!(
        before.contains(&Some((16, 11))),
        "TS input must carry 16:11: {before:?}"
    );
    let fmp4 = transmux::CmafMux::new(1).package(&media).expect("CMAF mux");
    let back = transmux::Fmp4Demux::new()
        .unpackage(&fmp4)
        .expect("fMP4 demux");
    let after: Vec<Option<(u16, u16)>> = back
        .tracks
        .iter()
        .map(|t| {
            t.spec
                .display
                .as_ref()
                .and_then(|g| g.sample_aspect_ratio())
        })
        .collect();
    assert_eq!(after, before);
    let video = back
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video track");
    assert_eq!(
        video.spec.display.as_ref().map(|g| g.aspect_ratio),
        Some(Some(AspectRatioIdc::Sar16x11))
    );
}
