//! HDR10+ (ST 2094-40) dynamic metadata over the real Tears of Steel stream
//! `private/fixtures/hdr10plus/` (see its PROVENANCE.md): typed parse of every
//! SEI against the x265/hdr10plus_tool JSON, byte-exact round trip of all 259
//! SEI NAL units, and the multi-window / Bezier / peak-luminance branches the
//! stream does not exercise, built from values chosen from the semantics (no
//! real capture carries them).
//!
//! Spec inputs: `private/specs-verified/smpte/hdr10plus/`.
#![cfg(feature = "std")]

use broadcast_common::mux::{Package, Unpackage};
use broadcast_common::{Parse, Serialize};
use transmux::annexb::{iter_annexb_nals, iter_length_prefixed_nals};
use transmux::ir::{CodecConfig, Media};
use transmux::ts_demux::TsDemux;
use transmux::{
    AdditionalWindow, BezierToneMapping, ColorVolumeTransform, DistributionPoint,
    Hdr10PlusMetadata, OverlapProcessOption, PeakLuminanceTable, WindowRegion,
};

const HEVC: &str = "hdr10plus/tos-256x144-hdr10plus.hevc";
const TS: &str = "hdr10plus/tos-256x144-hdr10plus.ts";
const JSON: &str = "hdr10plus/tos-256x144-hdr10plus-metadata.json";
const FRAMES: usize = 259;

fn hdr10plus_sei_nals(es: &[u8]) -> Vec<Vec<u8>> {
    iter_annexb_nals(es)
        .filter(|n| Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(n))
        .map(<[u8]>::to_vec)
        .collect()
}

fn fixture_es() -> Option<Vec<u8>> {
    test_fixtures::require_bytes(HEVC, "hdr10plus")
}

#[test]
fn every_sei_in_the_stream_round_trips_byte_exact() {
    let Some(es) = fixture_es() else { return };
    let nals = hdr10plus_sei_nals(&es);
    assert_eq!(nals.len(), FRAMES);
    for (i, nal) in nals.iter().enumerate() {
        let payloads = Hdr10PlusMetadata::hevc_sei_nal_payloads(nal);
        assert_eq!(payloads.len(), 1, "SEI {i}");
        let meta =
            Hdr10PlusMetadata::parse(&payloads[0]).unwrap_or_else(|e| panic!("SEI {i}: {e}"));
        assert_eq!(meta.try_to_bytes().unwrap(), payloads[0], "payload {i}");
        assert_eq!(meta.to_hevc_prefix_sei_nal().unwrap(), *nal, "NAL {i}");
        let again = Hdr10PlusMetadata::parse(&meta.try_to_bytes().unwrap()).unwrap();
        assert_eq!(again, meta, "serialize then parse {i}");
    }
}

#[test]
fn first_sei_decodes_to_the_values_ffprobe_reports() {
    let Some(es) = fixture_es() else { return };
    let nal = &hdr10plus_sei_nals(&es)[0];
    let meta = Hdr10PlusMetadata::from_hevc_sei_nal(nal).unwrap().unwrap();
    assert_eq!(meta.application_version, 1);
    assert_eq!(meta.num_windows(), 1);
    assert_eq!(meta.targeted_system_display_maximum_luminance, 0);
    assert!(meta.targeted_system_display_actual_peak_luminance.is_none());
    assert!(meta.mastering_display_actual_peak_luminance.is_none());
    let w = &meta.primary_window;
    assert_eq!(w.max_scl, [17830, 16895, 14252]);
    assert_eq!(w.average_max_rgb, 1037);
    let dist: Vec<(u8, u32)> = w
        .distribution_max_rgb
        .iter()
        .map(|p| (p.percentage, p.percentile))
        .collect();
    assert_eq!(
        dist,
        [
            (1, 3),
            (5, 14024),
            (10, 43),
            (25, 56),
            (50, 219),
            (75, 1036),
            (90, 2714),
            (95, 4668),
            (99, 14445)
        ]
    );
    assert_eq!(w.fraction_bright_pixels, 0);
    assert!(w.tone_mapping.is_none());
    assert!(w.color_saturation_weight.is_none());
    assert_eq!(meta.alignment_padding, 0);
}

fn display_order_sample_seis(ts: &[u8]) -> Vec<Vec<u8>> {
    let media = TsDemux::new().unpackage(ts).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("HEVC track");
    let mut samples: Vec<_> = track.samples.iter().collect();
    samples.sort_by_key(|s| s.pts);
    samples
        .iter()
        .map(|s| {
            let nals: Vec<Vec<u8>> = iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .filter(|n| Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(n))
                .map(<[u8]>::to_vec)
                .collect();
            assert_eq!(nals.len(), 1, "exactly one HDR10+ SEI per access unit");
            nals.into_iter().next().unwrap()
        })
        .collect()
}

#[test]
fn all_259_seis_match_the_x265_json_in_display_order() {
    let Some(ts) = test_fixtures::require_bytes(TS, "hdr10plus") else {
        return;
    };
    let Some(json) = test_fixtures::require_bytes(JSON, "hdr10plus") else {
        return;
    };
    let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let scenes = json["SceneInfo"].as_array().unwrap();
    assert_eq!(scenes.len(), FRAMES);
    let nals = display_order_sample_seis(&ts);
    assert_eq!(nals.len(), FRAMES);
    let ints = |v: &serde_json::Value| -> Vec<u64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect()
    };
    for (i, (nal, scene)) in nals.iter().zip(scenes).enumerate() {
        let meta = Hdr10PlusMetadata::from_hevc_sei_nal(nal).unwrap().unwrap();
        let lum = &scene["LuminanceParameters"];
        let w = &meta.primary_window;
        assert_eq!(
            w.max_scl.iter().map(|&v| u64::from(v)).collect::<Vec<_>>(),
            ints(&lum["MaxScl"]),
            "frame {i} MaxScl"
        );
        assert_eq!(
            u64::from(w.average_max_rgb),
            lum["AverageRGB"].as_u64().unwrap(),
            "frame {i} AverageRGB"
        );
        let dists = &lum["LuminanceDistributions"];
        assert_eq!(
            w.distribution_max_rgb
                .iter()
                .map(|p| u64::from(p.percentage))
                .collect::<Vec<_>>(),
            ints(&dists["DistributionIndex"]),
            "frame {i} index"
        );
        assert_eq!(
            w.distribution_max_rgb
                .iter()
                .map(|p| u64::from(p.percentile))
                .collect::<Vec<_>>(),
            ints(&dists["DistributionValues"]),
            "frame {i} values"
        );
        assert_eq!(
            meta.num_windows() as u64,
            scene["NumberOfWindows"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(meta.targeted_system_display_maximum_luminance),
            scene["TargetedSystemDisplayMaximumLuminance"]
                .as_u64()
                .unwrap()
        );
    }
}

#[test]
fn the_stream_violates_table_1_and_is_still_accepted() {
    let Some(es) = fixture_es() else { return };
    let meta = Hdr10PlusMetadata::from_hevc_sei_nal(&hdr10plus_sei_nals(&es)[0])
        .unwrap()
        .unwrap();
    let d = &meta.primary_window.distribution_max_rgb;
    assert_eq!((d[1].percentage, d[2].percentage), (5, 10));
    assert_ne!(d[1].percentile, 0);
    assert_ne!(d[2].percentile, 255);
}

fn transform(seed: u32) -> ColorVolumeTransform {
    ColorVolumeTransform {
        max_scl: [seed, seed + 1, seed + 2],
        average_max_rgb: seed * 3,
        distribution_max_rgb: (0..6u32)
            .map(|k| DistributionPoint {
                percentage: (k * 20) as u8,
                percentile: seed * 100 + k,
            })
            .collect(),
        fraction_bright_pixels: 77,
        tone_mapping: None,
        color_saturation_weight: None,
    }
}

fn region(base: u16) -> WindowRegion {
    WindowRegion {
        upper_left_corner_x: base,
        upper_left_corner_y: base + 1,
        lower_right_corner_x: base + 200,
        lower_right_corner_y: base + 120,
        center_of_ellipse_x: base + 100,
        center_of_ellipse_y: base + 60,
        rotation_angle: 180,
        semimajor_axis_internal_ellipse: 40,
        semimajor_axis_external_ellipse: 65535,
        semiminor_axis_external_ellipse: 30,
        overlap_process_option: OverlapProcessOption::Layering,
    }
}

fn round_trip(meta: &Hdr10PlusMetadata) -> Hdr10PlusMetadata {
    let bytes = meta.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), meta.serialized_len());
    assert_eq!(&bytes[..6], &[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
    let parsed = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(&parsed, meta);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    let nal = meta.to_hevc_prefix_sei_nal().unwrap();
    assert_eq!(
        Hdr10PlusMetadata::from_hevc_sei_nal(&nal).unwrap().unwrap(),
        *meta
    );
    parsed
}

#[test]
fn constructed_three_windows_with_bezier_and_tables_round_trip() {
    let mut primary = transform(10);
    primary.tone_mapping = Some(BezierToneMapping {
        knee_point_x: 4095,
        knee_point_y: 1,
        anchors: (0..9u16).map(|k| k * 100 + 3).collect(),
    });
    primary.color_saturation_weight = Some(63);
    let mut second = transform(20);
    second.tone_mapping = Some(BezierToneMapping {
        knee_point_x: 0,
        knee_point_y: 0,
        anchors: Vec::new(),
    });
    let mut third = transform(30);
    third.color_saturation_weight = Some(0);
    let meta = Hdr10PlusMetadata {
        application_version: 1,
        targeted_system_display_maximum_luminance: (1 << 27) - 1,
        targeted_system_display_actual_peak_luminance: Some(PeakLuminanceTable {
            rows: 2,
            columns: 3,
            values: vec![0, 15, 7, 8, 1, 14],
        }),
        primary_window: primary,
        additional_windows: vec![
            AdditionalWindow {
                region: region(5),
                color_volume: second,
            },
            AdditionalWindow {
                region: WindowRegion {
                    overlap_process_option: OverlapProcessOption::WeightedAverage,
                    ..region(9)
                },
                color_volume: third,
            },
        ],
        mastering_display_actual_peak_luminance: Some(PeakLuminanceTable {
            rows: 25,
            columns: 25,
            values: (0..625u32).map(|k| (k % 16) as u8).collect(),
        }),
        alignment_padding: 0,
    };
    let parsed = round_trip(&meta);
    assert_eq!(parsed.num_windows(), 3);
    assert_eq!(parsed.additional_windows[0].region.rotation_angle, 180);
    assert_eq!(
        parsed.additional_windows[1].region.overlap_process_option,
        OverlapProcessOption::WeightedAverage
    );
    assert_eq!(
        parsed
            .primary_window
            .tone_mapping
            .as_ref()
            .unwrap()
            .anchors
            .len(),
        9
    );
    assert_eq!(
        parsed
            .mastering_display_actual_peak_luminance
            .unwrap()
            .values
            .len(),
        625
    );
}

#[test]
fn version_zero_with_fifteen_anchors_and_fifteen_percentiles_round_trips() {
    let mut primary = transform(5);
    primary.distribution_max_rgb = (0..15u32)
        .map(|k| DistributionPoint {
            percentage: (k * 7) as u8,
            percentile: (1 << 17) - 1 - k,
        })
        .collect();
    primary.tone_mapping = Some(BezierToneMapping {
        knee_point_x: 2048,
        knee_point_y: 2047,
        anchors: (0..15u16).map(|k| 1023 - k).collect(),
    });
    let meta = Hdr10PlusMetadata {
        application_version: 0,
        targeted_system_display_maximum_luminance: 1000,
        targeted_system_display_actual_peak_luminance: None,
        primary_window: primary,
        additional_windows: Vec::new(),
        mastering_display_actual_peak_luminance: None,
        alignment_padding: 0,
    };
    round_trip(&meta);
}

#[test]
fn alignment_padding_bits_are_preserved() {
    let meta = Hdr10PlusMetadata {
        application_version: 1,
        targeted_system_display_maximum_luminance: 0,
        targeted_system_display_actual_peak_luminance: None,
        primary_window: transform(1),
        additional_windows: Vec::new(),
        mastering_display_actual_peak_luminance: None,
        alignment_padding: 0,
    };
    let mut bytes = meta.try_to_bytes().unwrap();
    let last = bytes.len() - 1;
    bytes[last] |= 0b11;
    let parsed = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(parsed.alignment_padding, 0b11);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn malformed_payloads_are_rejected() {
    let meta = Hdr10PlusMetadata {
        application_version: 1,
        targeted_system_display_maximum_luminance: 0,
        targeted_system_display_actual_peak_luminance: None,
        primary_window: transform(1),
        additional_windows: Vec::new(),
        mastering_display_actual_peak_luminance: None,
        alignment_padding: 0,
    };
    let good = meta.try_to_bytes().unwrap();
    for cut in 0..good.len() {
        assert!(Hdr10PlusMetadata::parse(&good[..cut]).is_err(), "cut {cut}");
    }
    let mut extra = good.clone();
    extra.push(0);
    assert!(Hdr10PlusMetadata::parse(&extra).is_err());
    let mut zero_windows = good.clone();
    zero_windows[7] &= 0b0011_1111;
    assert!(Hdr10PlusMetadata::parse(&zero_windows).is_err());
    let mut wrong_provider = good.clone();
    wrong_provider[2] = 0x3D;
    assert!(Hdr10PlusMetadata::parse(&wrong_provider).is_err());
    let mut wrong_app = good.clone();
    wrong_app[5] = 5;
    assert!(Hdr10PlusMetadata::parse(&wrong_app).is_err());
    let mut wrong_version = good;
    wrong_version[6] = 2;
    assert!(Hdr10PlusMetadata::parse(&wrong_version).is_err());
}

#[test]
fn serializing_inconsistent_values_is_an_error_not_a_misframed_payload() {
    let base = Hdr10PlusMetadata {
        application_version: 1,
        targeted_system_display_maximum_luminance: 0,
        targeted_system_display_actual_peak_luminance: None,
        primary_window: transform(1),
        additional_windows: Vec::new(),
        mastering_display_actual_peak_luminance: None,
        alignment_padding: 0,
    };
    let mut too_wide = base.clone();
    too_wide.primary_window.max_scl[0] = 1 << 17;
    assert!(too_wide.try_to_bytes().is_err());
    let mut bad_table = base.clone();
    bad_table.mastering_display_actual_peak_luminance = Some(PeakLuminanceTable {
        rows: 2,
        columns: 2,
        values: vec![0; 3],
    });
    assert!(bad_table.try_to_bytes().is_err());
    let mut too_many_windows = base.clone();
    for k in 0..3 {
        too_many_windows.additional_windows.push(AdditionalWindow {
            region: region(k),
            color_volume: transform(2),
        });
    }
    assert!(too_many_windows.try_to_bytes().is_err());
    let mut too_many_points = base;
    too_many_points.primary_window.distribution_max_rgb = (0..16u32)
        .map(|k| DistributionPoint {
            percentage: k as u8,
            percentile: k,
        })
        .collect();
    assert!(too_many_points.try_to_bytes().is_err());
}

fn hevc_track(media: &Media) -> &transmux::ir::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("HEVC track")
}

fn seis_in_display_order(media: &Media) -> Vec<Vec<u8>> {
    let mut samples: Vec<_> = hevc_track(media).samples.iter().collect();
    samples.sort_by_key(|s| s.pts);
    samples
        .iter()
        .flat_map(|s| {
            iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .filter(|n| Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(n))
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn source_media() -> Option<(Media, Vec<Vec<u8>>)> {
    let ts = test_fixtures::require_bytes(TS, "hdr10plus")?;
    let media = TsDemux::new().unpackage(&ts).expect("demux");
    let seis = seis_in_display_order(&media);
    assert_eq!(seis.len(), FRAMES);
    Some((media, seis))
}

#[test]
fn track_flag_is_set_from_the_first_access_unit_alongside_hdr10_static_metadata() {
    let Some((media, _)) = source_media() else {
        return;
    };
    let spec = &hevc_track(&media).spec;
    assert!(spec.hdr10plus);
    let hdr = spec.hdr.expect("HDR10 static metadata");
    assert_eq!(
        hdr.content_light_level.unwrap().max_content_light_level,
        1000
    );
    assert!(hdr.mastering_display.is_some());
}

#[test]
fn track_flag_is_clear_for_streams_without_hdr10plus() {
    for fixture in ["hdr10/meridian-hdr10-60f.ts", "ts/hevc/main10.ts"] {
        let Some(bytes) = test_fixtures::require_bytes(fixture, "hdr10plus") else {
            continue;
        };
        let media = TsDemux::new().unpackage(&bytes).expect("demux");
        assert!(!hevc_track(&media).spec.hdr10plus, "{fixture}");
    }
}

#[test]
fn fmp4_output_keeps_every_sei_byte_identical_and_keeps_the_flag() {
    let Some((media, source)) = source_media() else {
        return;
    };
    let bytes = transmux::CmafMux::default().package(&media).expect("cmaf");
    let back = transmux::media::Fmp4Demux::default()
        .unpackage(&bytes)
        .expect("fmp4 demux");
    assert_eq!(seis_in_display_order(&back).len(), FRAMES);
    assert_eq!(seis_in_display_order(&back), source);
    assert!(hevc_track(&back).spec.hdr10plus);
}

#[test]
fn ts_output_keeps_every_sei_byte_identical_and_keeps_the_flag() {
    let Some((media, source)) = source_media() else {
        return;
    };
    let bytes = transmux::ts_mux::TsMux::default()
        .package(&media)
        .expect("ts mux");
    let back = TsDemux::new().unpackage(&bytes).expect("ts demux");
    assert_eq!(seis_in_display_order(&back).len(), FRAMES);
    assert_eq!(seis_in_display_order(&back), source);
    assert!(hevc_track(&back).spec.hdr10plus);
}

#[test]
fn ts_hls_segments_keep_every_sei_byte_identical() {
    let Some((media, source)) = source_media() else {
        return;
    };
    let out = transmux::ts_hls::TsHlsPackager::default()
        .package(&media)
        .expect("ts hls");
    let joined: Vec<u8> = out.segments.concat();
    let back = TsDemux::new().unpackage(&joined).expect("ts demux");
    assert_eq!(seis_in_display_order(&back), source);
}

const DASHIF_PROPERTY: &str =
    r#"<SupplementalProperty schemeIdUri="http://dashif.org/metadata/hdr" value="SMPTE2094-40"/>"#;
const DVB_PROPERTY: &str =
    r#"<SupplementalProperty schemeIdUri="urn:dvb:dash:hdr-dmi" value="ST2094-40"/>"#;
const DVB_PROFILE: &str = "urn:dvb:dash:profile:dvb-dash:2014";

fn mpd(media: &Media, profiles: Option<&str>) -> String {
    let mut packager = transmux::DashPackager::default();
    if let Some(p) = profiles {
        packager.profiles = p.to_string();
    }
    packager.package(media).expect("dash")
}

#[test]
fn dash_if_supplemental_property_is_emitted_once_on_the_video_adaptation_set() {
    let Some((media, _)) = source_media() else {
        return;
    };
    let text = mpd(&media, None);
    assert_eq!(text.matches(DASHIF_PROPERTY).count(), 1, "{text}");
    assert!(!text.contains("urn:dvb:dash:hdr-dmi"), "{text}");
}

#[test]
fn dvb_dash_profile_adds_the_hdr_dmi_property() {
    let Some((media, _)) = source_media() else {
        return;
    };
    let text = mpd(&media, Some(DVB_PROFILE));
    assert_eq!(text.matches(DASHIF_PROPERTY).count(), 1, "{text}");
    assert_eq!(text.matches(DVB_PROPERTY).count(), 1, "{text}");
}

#[test]
fn dash_without_hdr10plus_carries_no_hdr_metadata_property() {
    for fixture in ["hdr10/meridian-hdr10-60f.ts", "ts/hevc/main10.ts"] {
        let Some(bytes) = test_fixtures::require_bytes(fixture, "hdr10plus") else {
            continue;
        };
        let media = TsDemux::new().unpackage(&bytes).expect("demux");
        let text = mpd(&media, Some(DVB_PROFILE));
        assert!(!text.contains("hdr"), "{fixture}\n{text}");
    }
}

const FFPROBE_SIDE_DATA: &str = "HDR Dynamic Metadata SMPTE2094-40 (HDR10+)";

fn ffprobe_frames(file: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_frames", "-select_streams", "v"])
        .arg(file)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

#[test]
fn ffprobe_reports_hdr10plus_side_data_on_every_frame_of_our_ts_output() {
    let Some((media, _)) = source_media() else {
        return;
    };
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/hdr10plus-ts");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.ts");
    std::fs::write(
        &file,
        transmux::ts_mux::TsMux::default()
            .package(&media)
            .expect("ts mux"),
    )
    .unwrap();
    let Some(text) = ffprobe_frames(&file) else {
        eprintln!(
            "SKIP ffprobe_reports_hdr10plus_side_data_on_every_frame_of_our_ts_output: ffprobe not usable"
        );
        return;
    };
    assert_eq!(text.matches("[FRAME]").count(), FRAMES, "{text}");
    assert_eq!(text.matches(FFPROBE_SIDE_DATA).count(), FRAMES);

    let mut control = media.clone();
    for track in &mut control.tracks {
        for sample in &mut track.samples {
            let mut kept = Vec::new();
            for nal in iter_length_prefixed_nals(&sample.data).unwrap() {
                if !Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(nal) {
                    kept.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    kept.extend_from_slice(nal);
                }
            }
            sample.data = kept.into();
        }
    }
    let control_file = dir.join("control.ts");
    std::fs::write(
        &control_file,
        transmux::ts_mux::TsMux::default()
            .package(&control)
            .expect("ts mux"),
    )
    .unwrap();
    let control_text = ffprobe_frames(&control_file).expect("ffprobe");
    assert_eq!(control_text.matches("[FRAME]").count(), FRAMES);
    assert_eq!(control_text.matches(FFPROBE_SIDE_DATA).count(), 0);
}

struct Bits {
    bytes: Vec<u8>,
    used: usize,
}

impl Bits {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            used: 0,
        }
    }

    fn put(&mut self, value: u64, width: usize) -> &mut Self {
        assert!(
            width == 64 || value < (1 << width),
            "{value} wider than {width}"
        );
        for shift in (0..width).rev() {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((value >> shift) & 1) as u8;
            *self.bytes.last_mut().unwrap() |= bit << (7 - self.used % 8);
            self.used += 1;
        }
        self
    }

    fn window_color_volume(&mut self, seed: u64) -> &mut Self {
        self.put(100 + seed, 17)
            .put(200 + seed, 17)
            .put(300 + seed, 17);
        self.put(400 + seed, 17);
        self.put(2, 4);
        self.put(1, 7).put(1000 + seed, 17);
        self.put(99, 7).put(131071, 17);
        self.put(777 + seed, 10)
    }
}

fn payload_with(bits: Bits) -> Vec<u8> {
    let mut out = vec![0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04, 0x01];
    out.extend_from_slice(&bits.bytes);
    out
}

#[test]
fn a_payload_assembled_independently_field_by_field_decodes_to_those_fields() {
    let mut b = Bits::new();
    b.put(1, 2);
    b.put(1000, 27);
    b.put(1, 1).put(2, 5).put(3, 5);
    for v in [0, 1, 2, 3, 14, 15] {
        b.put(v, 4);
    }
    b.window_color_volume(0);
    b.put(1, 1).put(1, 5).put(2, 5).put(9, 4).put(15, 4);
    b.put(1, 1)
        .put(4095, 12)
        .put(1234, 12)
        .put(2, 4)
        .put(1023, 10)
        .put(5, 10);
    b.put(1, 1).put(63, 6);
    let bytes = payload_with(b);
    let meta = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(meta.targeted_system_display_maximum_luminance, 1000);
    let lut = meta
        .targeted_system_display_actual_peak_luminance
        .as_ref()
        .unwrap();
    assert_eq!((lut.rows, lut.columns), (2, 3));
    assert_eq!(lut.values, [0, 1, 2, 3, 14, 15]);
    let w = &meta.primary_window;
    assert_eq!(w.max_scl, [100, 200, 300]);
    assert_eq!(w.average_max_rgb, 400);
    assert_eq!(
        (
            w.distribution_max_rgb[0].percentage,
            w.distribution_max_rgb[0].percentile
        ),
        (1, 1000)
    );
    assert_eq!(
        (
            w.distribution_max_rgb[1].percentage,
            w.distribution_max_rgb[1].percentile
        ),
        (99, 131071)
    );
    assert_eq!(w.fraction_bright_pixels, 777);
    let mastering = meta
        .mastering_display_actual_peak_luminance
        .as_ref()
        .unwrap();
    assert_eq!((mastering.rows, mastering.columns), (1, 2));
    assert_eq!(mastering.values, [9, 15]);
    let tone = w.tone_mapping.as_ref().unwrap();
    assert_eq!((tone.knee_point_x, tone.knee_point_y), (4095, 1234));
    assert_eq!(tone.anchors, [1023, 5]);
    assert_eq!(w.color_saturation_weight, Some(63));
    assert_eq!(meta.try_to_bytes().unwrap(), bytes);
}

#[test]
fn a_three_window_payload_assembled_independently_decodes_to_those_fields() {
    let mut b = Bits::new();
    b.put(3, 2);
    for base in [10u64, 20] {
        for v in 0..6 {
            b.put(base + v, 16);
        }
        b.put(90 + base, 8);
        b.put(1000 + base, 16)
            .put(2000 + base, 16)
            .put(3000 + base, 16);
        b.put(base % 2, 1);
    }
    b.put(5, 27).put(0, 1);
    b.window_color_volume(0)
        .window_color_volume(1)
        .window_color_volume(2);
    b.put(0, 1);
    b.put(0, 1).put(0, 1);
    b.put(1, 1).put(0, 12).put(0, 12).put(0, 4).put(0, 1);
    b.put(0, 1).put(1, 1).put(8, 6);
    let bytes = payload_with(b);
    let meta = Hdr10PlusMetadata::parse(&bytes).unwrap();
    assert_eq!(meta.num_windows(), 3);
    let first = &meta.additional_windows[0].region;
    assert_eq!(
        (
            first.upper_left_corner_x,
            first.upper_left_corner_y,
            first.lower_right_corner_x,
            first.lower_right_corner_y,
            first.center_of_ellipse_x,
            first.center_of_ellipse_y,
            first.rotation_angle,
            first.semimajor_axis_internal_ellipse,
            first.semimajor_axis_external_ellipse,
            first.semiminor_axis_external_ellipse,
        ),
        (10, 11, 12, 13, 14, 15, 100, 1010, 2010, 3010)
    );
    assert_eq!(
        first.overlap_process_option,
        OverlapProcessOption::WeightedAverage
    );
    let second = &meta.additional_windows[1].region;
    assert_eq!(second.semiminor_axis_external_ellipse, 3020);
    assert_eq!(
        second.overlap_process_option,
        OverlapProcessOption::WeightedAverage
    );
    assert_eq!(
        meta.additional_windows[1].color_volume.max_scl,
        [102, 202, 302]
    );
    assert_eq!(
        meta.additional_windows[0]
            .color_volume
            .fraction_bright_pixels,
        778
    );
    assert!(meta.primary_window.tone_mapping.is_none());
    assert_eq!(
        meta.additional_windows[0]
            .color_volume
            .tone_mapping
            .as_ref()
            .unwrap()
            .anchors
            .len(),
        0
    );
    assert_eq!(
        meta.additional_windows[1]
            .color_volume
            .color_saturation_weight,
        Some(8)
    );
    assert_eq!(meta.try_to_bytes().unwrap(), bytes);
}

#[test]
fn sample_walk_rejects_a_maximal_nal_length_without_overflow() {
    let sample = [0xFF, 0xFF, 0xFF, 0xFF, 0x4E, 0x01];
    assert!(!transmux::Hdr10PlusMetadata::sample_carries_hdr10plus(
        &sample
    ));
}
