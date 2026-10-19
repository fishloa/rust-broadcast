//! Mux-side display geometry (Phase 4a-1): `build_trak` writes `pasp`/`colr`
//! from `TrackSpec::display`.
//!
//! Tests (a) unit-build a `TrackSpec` with `with_display` / a constructed
//! `DisplayGeometry`, build an init segment, parse it back with transmux's own
//! box parsers, and assert exact values, order and absence. Tests (b) run real
//! captures through `TsDemux` -> `build_init_segment` and cross-check with
//! ffprobe.

use broadcast_common::Parse;
use transmux::ir::{
    AspectRatioIdc, CodecConfig, ColourDescription, DisplayGeometry, VideoFormat, VideoSignalType,
};
use transmux::pipeline;
use transmux::{
    AVCDecoderConfigurationRecord, AVCSampleEntry, HEVCDecoderConfigurationRecord, HEVCSampleEntry,
    TrackId, TrackSpec,
};

fn find_box_body<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == four_cc.as_slice())
        .expect("box four-CC must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start + 8..start + size]
}

fn avc_config() -> AVCDecoderConfigurationRecord {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/h264_high.mp4"
    ))
    .expect("fixture");
    let body = find_box_body(&data, b"avcC");
    AVCDecoderConfigurationRecord::parse(body).expect("parse avcC")
}

fn hevc_config() -> HEVCDecoderConfigurationRecord {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/hevc_main.mp4"
    ))
    .expect("fixture");
    let hvc_box = find_full_box(&data, b"hvcC");
    HEVCDecoderConfigurationRecord::parse(&hvc_box[8..]).expect("parse hvcC")
}

fn build(spec: TrackSpec) -> Vec<u8> {
    pipeline::build_init_segment(&[spec], 90000).expect("build_init_segment")
}

fn find_full_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == fourcc)
        .unwrap_or_else(|| panic!("box {:?} not found", std::str::from_utf8(fourcc).unwrap()));
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start..start + size]
}

fn avc_entry(data: &[u8]) -> AVCSampleEntry {
    let full = find_full_box(data, b"avc1");
    AVCSampleEntry::bare_parse(full).expect("avc1 parse")
}

fn hevc_entry(data: &[u8]) -> HEVCSampleEntry {
    let full = find_full_box(data, b"hvc1");
    HEVCSampleEntry::bare_parse(full).expect("hvc1 parse")
}

fn pasp_of(entry: &AVCSampleEntry) -> Option<(u32, u32)> {
    entry
        .extra_boxes
        .iter()
        .find(|b| b.box_type == *b"pasp")
        .map(|b| {
            let p = transmux::visual_ext::PixelAspectRatioBox::parse(&b.data).unwrap();
            (p.h_spacing, p.v_spacing)
        })
}

fn colr_of(entry: &AVCSampleEntry) -> Option<(u16, u16, u16, bool)> {
    entry
        .extra_boxes
        .iter()
        .find(|b| b.box_type == *b"colr")
        .map(|b| {
            let c = transmux::visual_ext::ColourInformationBox::parse(&b.data).unwrap();
            let n = c.nclx.unwrap();
            (
                n.colour_primaries,
                n.transfer_characteristics,
                n.matrix_coefficients,
                n.full_range_flag,
            )
        })
}

fn box_types(entry: &AVCSampleEntry) -> Vec<[u8; 4]> {
    entry.extra_boxes.iter().map(|b| b.box_type).collect()
}

// ---- (a) unit tests ----

#[test]
fn sar_64_45_writes_pasp() {
    let g = DisplayGeometry::new(
        720,
        576,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        }),
    );
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert_eq!(pasp_of(&e), Some((64, 45)));
    assert!(colr_of(&e).is_none());
}

#[test]
fn sar_16_11_writes_pasp() {
    let g = DisplayGeometry::new(720, 576, Some(AspectRatioIdc::Sar16x11));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert_eq!(pasp_of(&e), Some((16, 11)));
}

#[test]
fn sar_1_1_writes_no_pasp() {
    let g = DisplayGeometry::new(720, 576, Some(AspectRatioIdc::Square));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert!(pasp_of(&e).is_none());
}

#[test]
fn sar_unspecified_writes_no_pasp() {
    let g = DisplayGeometry::new(720, 576, None);
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert!(pasp_of(&e).is_none());
}

#[test]
fn colour_5_5_5_limited_writes_colr() {
    let mut g = DisplayGeometry::new(720, 576, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Unspecified,
        false,
        Some(ColourDescription::from_wire(5, 5, 5)),
    ));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert_eq!(colr_of(&e), Some((5, 5, 5, false)));
    assert!(pasp_of(&e).is_none());
}

#[test]
fn colour_bt709_full_range_writes_colr() {
    let mut g = DisplayGeometry::new(720, 576, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Component,
        true,
        Some(ColourDescription::from_wire(1, 1, 1)),
    ));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert_eq!(colr_of(&e), Some((1, 1, 1, true)));
}

#[test]
fn video_signal_present_colour_none_writes_no_colr() {
    let mut g = DisplayGeometry::new(720, 576, None);
    g.video_signal = Some(VideoSignalType::new(VideoFormat::Component, true, None));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert!(colr_of(&e).is_none());
}

#[test]
fn no_display_writes_neither() {
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    );
    let data = build(spec);
    let e = avc_entry(&data);
    assert!(pasp_of(&e).is_none());
    assert!(colr_of(&e).is_none());
}

#[test]
fn neither_is_byte_identical_to_display_none() {
    // display None vs display Some with no effective pasp/colr -> identical bytes.
    let spec_none = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    );
    let mut g = DisplayGeometry::new(720, 576, Some(AspectRatioIdc::Square));
    g.video_signal = Some(VideoSignalType::new(VideoFormat::Component, true, None));
    let spec_neither = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    assert_eq!(build(spec_none), build(spec_neither));
}

#[test]
fn order_pasp_then_colr_after_config() {
    let mut g = DisplayGeometry::new(
        720,
        576,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        }),
    );
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Unspecified,
        false,
        Some(ColourDescription::from_wire(5, 5, 5)),
    ));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox {
                config: avc_config(),
            },
            width: 720,
            height: 576,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = avc_entry(&data);
    assert_eq!(box_types(&e), vec![*b"pasp", *b"colr"]);
}

#[test]
fn hevc_writes_pasp_and_colr() {
    let mut g = DisplayGeometry::new(
        1920,
        1080,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        }),
    );
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Component,
        true,
        Some(ColourDescription::from_wire(1, 1, 1)),
    ));
    let spec = TrackSpec::new(
        TrackId::new(1),
        90000,
        CodecConfig::Hevc {
            config: transmux::HEVCConfigurationBox {
                config: hevc_config(),
            },
            width: 1920,
            height: 1080,
        },
    )
    .with_display(Some(g));
    let data = build(spec);
    let e = hevc_entry(&data);
    // extra_boxes for HEVC: inspect via pasp/colr using raw box types
    let types: Vec<[u8; 4]> = e.extra_boxes.iter().map(|b| b.box_type).collect();
    assert_eq!(types, vec![*b"pasp", *b"colr"]);
    let pasp = transmux::visual_ext::PixelAspectRatioBox::parse(&e.extra_boxes[0].data).unwrap();
    assert_eq!((pasp.h_spacing, pasp.v_spacing), (64, 45));
    let colr = transmux::visual_ext::ColourInformationBox::parse(&e.extra_boxes[1].data).unwrap();
    let n = colr.nclx.unwrap();
    assert_eq!(
        (
            n.colour_primaries,
            n.transfer_characteristics,
            n.matrix_coefficients,
            n.full_range_flag
        ),
        (1, 1, 1, true)
    );
}

// ---- (b) real captures ----

fn read(fixture: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(fixture, "display_mux")
}

#[test]
fn melodie_pasp_and_colr() {
    let Some(data) = read("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let media = transmux::ts_demux::TsDemux::new()
        .demux(&data)
        .expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc track");
    let spec = track.spec.clone();
    let out = build(spec);
    let e = avc_entry(&out);
    assert_eq!(pasp_of(&e), Some((64, 45)));
    assert_eq!(colr_of(&e), Some((5, 5, 5, false)));
}

#[test]
fn krone_pasp_no_colr() {
    let Some(data) = read("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let media = transmux::ts_demux::TsDemux::new()
        .demux(&data)
        .expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc track");
    let out = build(track.spec.clone());
    let e = avc_entry(&out);
    assert_eq!(pasp_of(&e), Some((16, 11)));
    assert!(colr_of(&e).is_none());
}

#[test]
fn transformers_neither() {
    let Some(data) = read("ts/transformers-eia608-h264.ts") else {
        return;
    };
    let media = transmux::ts_demux::TsDemux::new()
        .demux(&data)
        .expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc track");
    let out = build(track.spec.clone());
    let e = avc_entry(&out);
    assert!(pasp_of(&e).is_none());
    assert!(colr_of(&e).is_none());
}

#[test]
fn gulli_colr_no_pasp() {
    // public fixture
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/gulli-opengop.ts");
    let Ok(data) = std::fs::read(&path) else {
        return;
    };
    let media = transmux::ts_demux::TsDemux::new()
        .demux(&data)
        .expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc track");
    let out = build(track.spec.clone());
    let e = avc_entry(&out);
    assert!(pasp_of(&e).is_none());
    assert_eq!(colr_of(&e), Some((1, 1, 1, false)));
}
#[test]
fn ffprobe_oracle_real_captures() {
    if std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("SKIP ffprobe_oracle_real_captures: ffprobe not installed");
        return;
    }
    type Case = (&'static str, Option<&'static str>, Option<&'static str>, Option<&'static str>);
    let cases: &[Case] = &[
        (
            "ts/melodie-anamorphic-sar-64-45-zenith-tv.ts",
            Some("64:45"),
            Some("bt470bg"),
            Some("tv"),
        ),
        (
            "ts/krone-anamorphic-sar-16-11-zenith-tv.ts",
            Some("16:11"),
            None,
            None,
        ),
        ("ts/transformers-eia608-h264.ts", None, None, None),
    ];
    for (fixture, sar, colour, range) in cases {
        let Some(data) = read(fixture) else {
            continue;
        };
        let media = transmux::ts_demux::TsDemux::new()
            .demux(&data)
            .expect("demux");
        let track = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("avc track");
        let out = build(track.spec.clone());
        let tmp = std::env::temp_dir().join(format!("{}_oracle.mp4", fixture.replace('/', "_")));
        std::fs::write(&tmp, &out).expect("write");
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=sample_aspect_ratio,color_range,color_space,color_transfer,color_primaries",
                "-of",
                "default=nw=1",
            ])
            .arg(&tmp)
            .output()
            .expect("ffprobe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(s) = sar {
            assert!(
                stdout.contains(&format!("sample_aspect_ratio={}", s)),
                "{}: {:?}\n{}",
                fixture,
                sar,
                stdout
            );
        } else {
            assert!(
                stdout.contains("sample_aspect_ratio=N/A"),
                "{}: {:?}\n{}",
                fixture,
                sar,
                stdout
            );
        }
        if let Some(c) = colour {
            assert!(
                stdout.contains(&format!("color_space={}", c)),
                "{}: {:?}\n{}",
                fixture,
                colour,
                stdout
            );
        } else {
            assert!(
                stdout.contains("color_space=unknown"),
                "{}: {:?}\n{}",
                fixture,
                colour,
                stdout
            );
        }
        if let Some(r) = range {
            assert!(
                stdout.contains(&format!("color_range={}", r)),
                "{}: {:?}\n{}",
                fixture,
                range,
                stdout
            );
        }
    }
    // gulli public fixture
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/gulli-opengop.ts");
    if let Ok(data) = std::fs::read(&path) {
        let media = transmux::ts_demux::TsDemux::new()
            .demux(&data)
            .expect("demux");
        let track = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("avc track");
        let out = build(track.spec.clone());
        let tmp = std::env::temp_dir().join("gulli_oracle.mp4");
        std::fs::write(&tmp, &out).expect("write");
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=sample_aspect_ratio,color_range,color_space,color_transfer,color_primaries",
                "-of",
                "default=nw=1",
            ])
            .arg(&tmp)
            .output()
            .expect("ffprobe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("sample_aspect_ratio=N/A"),
            "gulli sar\n{}",
            stdout
        );
        assert!(
            stdout.contains("color_space=bt709"),
            "gulli colour\n{}",
            stdout
        );
    }
}
