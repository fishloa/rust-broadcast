//! tkhd presentation size (Phase 5): cropped output size rescaled by SAR.

use transmux::TrackSpec;
use transmux::ir::{AspectRatioIdc, CodecConfig};
use transmux::pipeline;
use transmux::ts_demux::TsDemux;

fn spec_from_fixture(fixture: &str) -> Option<TrackSpec> {
    let data = test_fixtures::require_bytes(fixture, "display_tkhd")?;
    let media = TsDemux::new().demux(&data).ok()?;
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .map(|t| t.spec)
}

fn build(spec: TrackSpec) -> Vec<u8> {
    pipeline::build_init_segment(&[spec], 90000).expect("build_init_segment")
}

fn tkhd_dims(data: &[u8]) -> (u32, u32) {
    let pos = data
        .windows(4)
        .position(|w| w == b"tkhd")
        .expect("tkhd box must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    let box_bytes = &data[start..start + size];
    let w = u32::from_be_bytes(box_bytes[size - 8..size - 4].try_into().unwrap());
    let h = u32::from_be_bytes(box_bytes[size - 4..size].try_into().unwrap());
    (w, h)
}

fn dims(spec: &TrackSpec) -> (u32, u32) {
    match &spec.config {
        CodecConfig::Avc { width, height, .. } => (*width as u32, *height as u32),
        CodecConfig::Hevc { width, height, .. } => (*width as u32, *height as u32),
        _ => (0, 0),
    }
}

#[test]
fn melodie_tkhd() {
    let Some(spec) = spec_from_fixture("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let data = build(spec.clone());
    assert_eq!(tkhd_dims(&data), (0x0400_0000, 0x0240_0000));
    // swapped SAR differs
    let mut g = spec.display.clone().expect("display");
    g.aspect_ratio = Some(AspectRatioIdc::ExtendedSar {
        sar_width: 45,
        sar_height: 64,
    });
    let swapped = build(spec.with_display(Some(g)));
    assert_ne!(tkhd_dims(&swapped).0, 0x0400_0000);
}

#[test]
fn krone_tkhd() {
    let Some(spec) = spec_from_fixture("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let data = build(spec.clone());
    assert_eq!(tkhd_dims(&data), (0x0417_45D1, 0x0240_0000));
    let mut g = spec.display.clone().expect("display");
    g.aspect_ratio = Some(AspectRatioIdc::ExtendedSar {
        sar_width: 11,
        sar_height: 16,
    });
    let swapped = build(spec.with_display(Some(g)));
    assert_ne!(tkhd_dims(&swapped).0, 0x0417_45D1);
}

#[test]
fn high_1080_cropped_tkhd() {
    let Some(spec) = spec_from_fixture("ts/h264/high_1080_cropped.ts") else {
        return;
    };
    let data = build(spec);
    assert_eq!(tkhd_dims(&data), (0x0780_0000, 0x0438_0000));
}

#[test]
fn display_none_keeps_config_dims() {
    let Some(spec) = spec_from_fixture("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let data = build(spec.with_display(None));
    assert_eq!(tkhd_dims(&data), (w << 16, h << 16));
}

#[test]
fn tkhd_rounds_to_nearest() {
    let Some(mut spec) = spec_from_fixture("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let mut g = spec.display.clone().expect("display");
    g.output_rect = transmux::ir::PictureRect::new(0, 0, 721, 576);
    g.aspect_ratio = Some(AspectRatioIdc::Sar16x11);
    spec = spec.with_display(Some(g));
    let data = build(spec);
    assert_eq!(tkhd_dims(&data).0, 0x0418_BA2F);
}
