//! Demux-side display geometry (Phase 4a-2): an fMP4 / MP4 sample entry's
//! `pasp` and `colr nclx` are applied to `TrackSpec::display`.

use broadcast_common::Unpackage;
use transmux::TrackSpec;
use transmux::ir::{
    AspectRatioIdc, CodecConfig, ColourDescription, DisplayGeometry, VideoFormat, VideoSignalType,
};
use transmux::media::Fmp4Demux;
use transmux::pipeline;
use transmux::ts_demux::TsDemux;

fn avc_spec(fixture: &str) -> Option<TrackSpec> {
    let data = test_fixtures::require_bytes(fixture, "display_demux")?;
    let media = TsDemux::new().demux(&data).ok()?;
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .map(|t| t.spec)
}

fn dims(spec: &TrackSpec) -> (u32, u32) {
    match &spec.config {
        CodecConfig::Avc { width, height, .. } => (*width as u32, *height as u32),
        CodecConfig::Hevc { width, height, .. } => (*width as u32, *height as u32),
        _ => (0, 0),
    }
}

fn demux_display(bytes: &[u8]) -> Option<DisplayGeometry> {
    let mut d = Fmp4Demux::new();
    let media = d.unpackage(bytes).ok()?;
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))?
        .spec
        .display
}

fn build(spec: TrackSpec) -> Vec<u8> {
    pipeline::build_init_segment(&[spec], 90000).expect("build_init_segment")
}
#[test]
fn pasp_fills_unspecified_sar() {
    let Some(mut spec) = avc_spec("tsduck/input/test-062.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

#[test]
fn pasp_conflict_keeps_vui_sar() {
    let Some(mut spec) = avc_spec("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((16u16, 11u16)));
}

#[test]
fn pasp_agreement_unchanged() {
    let Some(mut spec) = avc_spec("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

#[test]
fn colr_overrides_colour() {
    let Some(mut spec) = avc_spec("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let mut g = DisplayGeometry::new(w, h, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Unspecified,
        true,
        Some(ColourDescription::from_wire(1, 13, 6)),
    ));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    let vs = got.video_signal.expect("video_signal");
    assert!(vs.full_range);
    let c = vs.colour.expect("colour");
    assert_eq!(c.primaries.code(), 1);
    assert_eq!(c.transfer.code(), 13);
    assert_eq!(c.matrix.code(), 6);
}

#[test]
fn colr_codepoint_over_255_ignored() {
    let Some(mut spec) = avc_spec("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let mut g = DisplayGeometry::new(w, h, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Unspecified,
        true,
        Some(ColourDescription::from_wire(1, 13, 6)),
    ));
    spec = spec.with_display(Some(g));
    let mut bytes = build(spec);
    // patch colr primaries to 257 (>255)
    let i = bytes.windows(4).position(|w| w == b"colr").expect("colr");
    bytes[i + 8] = 0x01;
    bytes[i + 9] = 0x01;
    let got = demux_display(&bytes).expect("display");
    let vs = got.video_signal.expect("video_signal");
    let c = vs.colour.expect("colour");
    // VUI colour 5/5/5 kept
    assert_eq!(c.primaries.code(), 5);
    assert_eq!(c.transfer.code(), 5);
    assert_eq!(c.matrix.code(), 5);
}

#[test]
fn neither_box_display_unchanged() {
    let data = match test_fixtures::require_bytes("ts/gulli-opengop.ts", "display_demux") {
        Some(d) => d,
        None => return,
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc");
    let orig = track.spec.display.clone();
    let spec = track.spec.clone();
    let ts = spec.timescale;
    let bytes = pipeline::build_init_segment(&[spec], ts).expect("build");
    let got = demux_display(&bytes).expect("display");
    assert_eq!(Some(got), orig);
}

#[test]
fn display_none_with_pasp_creates_display() {
    let data = match test_fixtures::require_bytes("ts/gulli-opengop.ts", "display_demux") {
        Some(d) => d,
        None => return,
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc");
    let mut spec = track.spec.clone();
    // corrupt the SPS so TrackSpec::new yields no display
    if let CodecConfig::Avc { config, .. } = &mut spec.config
        && let Some(sps) = config.config.sps.first_mut()
    {
        sps.0.fill(0);
    }
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let ts = spec.timescale;
    let bytes = pipeline::build_init_segment(&[spec], ts).expect("build");
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.coded_width, w);
    assert_eq!(got.coded_height, h);
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

#[test]
fn ffmpeg_melodie_pasp_colr() {
    let fixture = "/Volumes/External/Projects/rust-broadcast/private/fixtures/ts/melodie-anamorphic-sar-64-45-zenith-tv.ts";
    if !std::path::Path::new(fixture).exists() {
        println!("SKIP: private fixture missing");
        return;
    }
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_err()
    {
        println!("SKIP: ffmpeg absent");
        return;
    }
    let tmp = std::env::temp_dir().join("phase4a2_m.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-i",
            fixture,
            "-t",
            "3",
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+write_colr",
        ])
        .arg(&tmp)
        .status()
        .expect("ffmpeg");
    if !status.success() {
        println!("SKIP: ffmpeg failed");
        return;
    }
    let bytes = std::fs::read(&tmp).expect("read mp4");
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
    let vs = got.video_signal.expect("video_signal");
    assert!(!vs.full_range);
    let c = vs.colour.expect("colour");
    assert_eq!(c.primaries.code(), 5);
    assert_eq!(c.transfer.code(), 5);
    assert_eq!(c.matrix.code(), 5);
}

#[test]
fn ffmpeg_krone_pasp() {
    let fixture = "/Volumes/External/Projects/rust-broadcast/private/fixtures/ts/krone-anamorphic-sar-16-11-zenith-tv.ts";
    if !std::path::Path::new(fixture).exists() {
        println!("SKIP: private fixture missing");
        return;
    }
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_err()
    {
        println!("SKIP: ffmpeg absent");
        return;
    }
    let tmp = std::env::temp_dir().join("phase4a2_k.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-i",
            fixture,
            "-t",
            "3",
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+write_colr",
        ])
        .arg(&tmp)
        .status()
        .expect("ffmpeg");
    if !status.success() {
        println!("SKIP: private fixture missing");
        return;
    }
    let bytes = std::fs::read(&tmp).expect("read mp4");
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((16u16, 11u16)));
}
