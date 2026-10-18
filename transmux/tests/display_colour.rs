//! `TrackSpec::display` colour signalling on real captures
//! (`docs/display-geometry/DESIGN.md` Phase 1b).
//!
//! Each fixture was read with an independent oracle before its expectation was
//! written here: `ffmpeg -bsf:v trace_headers` over the video PID (it prints
//! `overscan_info_present_flag`, `video_signal_type_present_flag`,
//! `video_format`, `video_full_range_flag`,
//! `colour_description_present_flag`, `colour_primaries`,
//! `transfer_characteristics` and `matrix_coefficients` field by field):
//!
//! | fixture | video_format | full range | colour primaries/transfer/matrix |
//! |---|---|---|---|
//! | `ts/melodie-anamorphic-sar-64-45-zenith-tv.ts` (private) | 5 | 0 | 5/5/5 |
//! | `ts/krone-anamorphic-sar-16-11-zenith-tv.ts` (private) | absent (`video_signal_type_present_flag` 0) | | |
//! | `ts/transformers-eia608-h264.ts` (private) | 0 | 1 | absent (`colour_description_present_flag` 0) |
//! | `ts/gulli-opengop.ts` (public) | 0 | 0 | 1/1/1 (BT.709) |
//! | `tsduck/input/test-084.ts` (private) | 0 | 0 | 5/5/5 |
//!
//! Every capture was also scanned with the same oracle: the other H.264
//! fixtures either carry `video_signal_type_present_flag` 0 or one of the
//! combinations above. Checked through both TS demux entry points (batch
//! [`TsDemux`] and the streaming [`StreamingTsDemux`]). Fixtures are loaded
//! through `test_fixtures` and SKIP, never fail, when the private submodule is
//! not checked out.
//!
//! HEVC colour has **no** real fixture in this worktree
//! (`private/fixtures/tsduck/input/` holds no `test-074.hevc.ts`, and no
//! private HEVC capture signals a non-default colour description), so HEVC
//! colour is covered by the spec vectors in `src/sps.rs` only.

use broadcast_common::{Package, Unpackage};
use transmux::ir::{
    CodecConfig, ColourDescription, ColourPrimaries, MatrixCoefficients, TransferCharacteristics,
    VideoFormat, VideoSignalType,
};
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};

fn read(fixture: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(fixture, "display_colour")
}

/// The `display` of every AVC `TrackSpec` the streaming demuxer announces.
fn streaming_display(data: &[u8]) -> Vec<Option<transmux::ir::DisplayGeometry>> {
    let mut demux = StreamingTsDemux::new();
    let mut out = Vec::new();
    let mut take = |demux: &mut StreamingTsDemux, out: &mut Vec<_>| {
        while let Some(ev) = demux.poll_event() {
            if let DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) = ev
                && matches!(spec.config, CodecConfig::Avc { .. })
            {
                out.push(spec.display);
            }
        }
    };
    for chunk in data.chunks(188 * 512) {
        demux.feed(chunk);
        take(&mut demux, &mut out);
    }
    demux.finish();
    take(&mut demux, &mut out);
    out
}

/// The `display` of every AVC track in the batch `Media`.
fn batch_display(data: &[u8]) -> Vec<Option<transmux::ir::DisplayGeometry>> {
    let media = TsDemux::new().demux(data).expect("batch demux");
    media
        .tracks
        .iter()
        .filter(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .map(|t| t.spec.display.clone())
        .collect()
}

fn colour(p: u8, t: u8, m: u8) -> ColourDescription {
    ColourDescription::from_wire(p, t, m)
}

/// Assert `video_signal` through batch and streaming.
fn check(fixture: &str, want: Option<VideoSignalType>) {
    let Some(data) = read(fixture) else {
        return;
    };
    let batch = batch_display(&data);
    assert!(!batch.is_empty(), "{fixture}: no AVC track");
    for display in &batch {
        let g = display
            .as_ref()
            .unwrap_or_else(|| panic!("{fixture}: batch TrackSpec.display is None"));
        assert_eq!(g.video_signal, want, "{fixture} (batch)");
        assert_eq!(g.overscan_appropriate, None, "{fixture} (batch)");
    }
    for display in streaming_display(&data) {
        let g = display
            .as_ref()
            .unwrap_or_else(|| panic!("{fixture}: streaming TrackSpec.display is None"));
        assert_eq!(g.video_signal, want, "{fixture} (streaming)");
    }
}

/// Melodie TV: BT.601-ish SD, `video_format` 5, limited range, colour 5/5/5
/// (BT.470 System B,G primaries / transfer, BT.601 625 matrix).
#[test]
fn melodie_signals_bt601_sd_colour() {
    check(
        "ts/melodie-anamorphic-sar-64-45-zenith-tv.ts",
        Some(VideoSignalType::new(
            VideoFormat::Unspecified,
            false,
            Some(colour(5, 5, 5)),
        )),
    );
}

/// Krone: `video_signal_type_present_flag` 0, so nothing is recorded and every
/// accessor answers with the spec inference (format 5, limited, 2/2/2).
#[test]
fn krone_signals_no_video_signal_type() {
    let fixture = "ts/krone-anamorphic-sar-16-11-zenith-tv.ts";
    let Some(data) = read(fixture) else {
        return;
    };
    let batch = batch_display(&data);
    assert!(!batch.is_empty(), "{fixture}: no AVC track");
    for display in &batch {
        let g = display.as_ref().expect("display");
        assert_eq!(g.video_signal, None, "{fixture}");
        assert!(!g.full_range());
        assert_eq!(g.video_format(), VideoFormat::Unspecified);
        assert_eq!(g.colour_description(), colour(2, 2, 2));
    }
    for display in streaming_display(&data) {
        let g = display.as_ref().expect("display");
        assert_eq!(g.video_signal, None, "{fixture} (streaming)");
    }
}

/// Transformers (full-range computer-graphics source): `video_format` 0
/// (component), full range 1, `colour_description_present_flag` 0.
#[test]
fn transformers_is_full_range_component_without_colour_description() {
    let fixture = "ts/transformers-eia608-h264.ts";
    check(
        fixture,
        Some(VideoSignalType::new(VideoFormat::Component, true, None)),
    );
    let Some(data) = read(fixture) else {
        return;
    };
    for display in batch_display(&data) {
        let g = display.as_ref().expect("display");
        assert!(g.full_range(), "{fixture}");
        assert_eq!(g.video_format(), VideoFormat::Component);
        // No colour description: the inference (matrix 2), never BT.601.
        assert_eq!(
            g.colour_description().matrix,
            MatrixCoefficients::Unspecified
        );
    }
}

/// A real BT.709 capture (public fixture, France TNT Gulli): format 0
/// (component), limited range, colour 1/1/1.
#[test]
fn gulli_signals_bt709_colour() {
    check(
        "ts/gulli-opengop.ts",
        Some(VideoSignalType::new(
            VideoFormat::Component,
            false,
            Some(colour(1, 1, 1)),
        )),
    );
}

/// A second broadcaster coding the same 5/5/5 BT.601 SD signal
/// (tsduck `test-084.ts`, SAR 12:11), proving the read is not tied to one
/// capture's SPS.
#[test]
fn tsduck_084_signals_bt601_sd_colour() {
    check(
        "tsduck/input/test-084.ts",
        Some(VideoSignalType::new(
            VideoFormat::Component,
            false,
            Some(colour(5, 5, 5)),
        )),
    );
}

fn fmp4_round_trip(fixture: &str) {
    let Some(data) = read(fixture) else {
        return;
    };
    let media = TsDemux::new().demux(&data).expect("TS demux");
    let want = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .and_then(|t| t.spec.display.as_ref())
        .map(|g| g.video_signal)
        .unwrap_or_else(|| panic!("{fixture}: TS input has no DisplayGeometry"));
    // The fixture must really signal a colour description (else this proves
    // nothing about carrying it through the muxer).
    let signalled = want.expect("TS input signals a video signal type");
    assert!(
        signalled.colour.is_some(),
        "{fixture}: no colour description"
    );
    let fmp4 = transmux::CmafMux::new(1).package(&media).expect("CMAF mux");
    let back = transmux::Fmp4Demux::new()
        .unpackage(&fmp4)
        .expect("fMP4 demux");
    let video = back
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video track");
    let g = video
        .spec
        .display
        .as_ref()
        .expect("the muxed avcC still yields geometry");
    assert_eq!(g.video_signal, want, "{fixture}");
    // The typed label survives too (the point of the typed enums).
    let d = g.colour_description();
    assert!(matches!(
        d.matrix,
        MatrixCoefficients::Bt709 | MatrixCoefficients::Bt470SystemB
    ));
    assert_ne!(d.primaries, ColourPrimaries::Reserved(0));
    assert_ne!(d.transfer, TransferCharacteristics::Reserved(19));
}

/// The colour description survives an fMP4 (CMAF) mux + demux: the fields live
/// in the SPS carried by `avcC`, so a rewritten init segment round-trips them.
#[test]
fn cmaf_round_trip_keeps_the_colour_description() {
    fmp4_round_trip("ts/gulli-opengop.ts");
    fmp4_round_trip("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts");
}
