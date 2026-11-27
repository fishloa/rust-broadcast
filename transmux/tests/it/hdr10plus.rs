//! HDR10+ (ST 2094-40) through the container layer: the real Tears of Steel
//! stream `private/fixtures/video/hdr10plus/` (see its PROVENANCE.md) demuxed,
//! carried through TS / fMP4 / TS-HLS / DASH output, and compared with the
//! x265/hdr10plus_tool JSON per access unit. The typed parse/serialize of the
//! message itself is `broadcast-bitstream`'s own test.
//!
//! Spec inputs: `private/specs-verified/smpte/hdr10plus/`.
#![cfg(feature = "std")]

use crate::ts_hls_stage;
use crate::ts_mux_stage;
use broadcast_bitstream::Hdr10PlusMetadata;
use broadcast_bitstream::annexb::iter_length_prefixed_nals;
use test_harness::collect::Collected as Media;
use transmux::ir::CodecConfig;

const TS: &str = "video/hdr10plus/tos-256x144-hdr10plus.ts";
const JSON: &str = "video/hdr10plus/tos-256x144-hdr10plus-metadata.json";
const FRAMES: usize = 259;

fn display_order_sample_seis(ts: &[u8]) -> Vec<Vec<u8>> {
    let media = test_harness::collect::media_from_ts(ts).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("HEVC track");
    let mut samples: Vec<_> = track.samples.iter().collect();
    samples.sort_by_key(|s| s.timing.and_then(|t| t.pts()));
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
        let meta =
            Hdr10PlusMetadata::from_hevc_sei_nal(nal, &mut broadcast_bitstream::Findings::new())
                .unwrap();
        let lum = &scene["LuminanceParameters"];
        let w = meta.primary_window();
        assert_eq!(
            w.max_scl()
                .iter()
                .map(|&v| u64::from(v))
                .collect::<Vec<_>>(),
            ints(&lum["MaxScl"]),
            "frame {i} MaxScl"
        );
        assert_eq!(
            u64::from(w.average_max_rgb()),
            lum["AverageRGB"].as_u64().unwrap(),
            "frame {i} AverageRGB"
        );
        let dists = &lum["LuminanceDistributions"];
        assert_eq!(
            w.distribution_max_rgb()
                .iter()
                .map(|p| u64::from(p.percentage()))
                .collect::<Vec<_>>(),
            ints(&dists["DistributionIndex"]),
            "frame {i} index"
        );
        assert_eq!(
            w.distribution_max_rgb()
                .iter()
                .map(|p| u64::from(p.percentile()))
                .collect::<Vec<_>>(),
            ints(&dists["DistributionValues"]),
            "frame {i} values"
        );
        assert_eq!(
            meta.num_windows() as u64,
            scene["NumberOfWindows"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(meta.targeted_system_display_maximum_luminance()),
            scene["TargetedSystemDisplayMaximumLuminance"]
                .as_u64()
                .unwrap()
        );
    }
}

fn hevc_track(media: &Media) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("HEVC track")
}

fn seis_in_display_order(media: &Media) -> Vec<Vec<u8>> {
    let mut samples: Vec<_> = hevc_track(media).samples.iter().collect();
    samples.sort_by_key(|s| s.timing.and_then(|t| t.pts()));
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
    let media = test_harness::collect::media_from_ts(&ts).expect("demux");
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
        hdr.content_light_level().unwrap().max_content_light_level,
        1000
    );
    assert!(hdr.mastering_display().is_some());
}

#[test]
fn track_flag_is_clear_for_streams_without_hdr10plus() {
    for fixture in ["video/hdr10/meridian-hdr10-60f.ts", "ts/hevc/main10.ts"] {
        let Some(bytes) = test_fixtures::require_bytes(fixture, "hdr10plus") else {
            continue;
        };
        let media = test_harness::collect::media_from_ts(&bytes).expect("demux");
        assert!(!hevc_track(&media).spec.hdr10plus, "{fixture}");
    }
}

#[test]
fn fmp4_output_keeps_every_sei_byte_identical_and_keeps_the_flag() {
    let Some((media, source)) = source_media() else {
        return;
    };
    let bytes = crate::cmaf::package(&media).expect("cmaf");
    let back =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
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
    let bytes = ts_mux_stage::mux(&media);
    let back = test_harness::collect::media_from_ts(&bytes).expect("ts demux");
    assert_eq!(seis_in_display_order(&back).len(), FRAMES);
    assert_eq!(seis_in_display_order(&back), source);
    assert!(hevc_track(&back).spec.hdr10plus);
}

#[test]
fn ts_hls_segments_keep_every_sei_byte_identical() {
    let Some((media, source)) = source_media() else {
        return;
    };
    let out = ts_hls_stage::hls(&media, 6);
    let joined: Vec<u8> = out.segments.concat();
    let back = test_harness::collect::media_from_ts(&joined).expect("ts demux");
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
    crate::packagers::dash(&packager, media).expect("dash")
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
    for fixture in ["video/hdr10/meridian-hdr10-60f.ts", "ts/hevc/main10.ts"] {
        let Some(bytes) = test_fixtures::require_bytes(fixture, "hdr10plus") else {
            continue;
        };
        let media = test_harness::collect::media_from_ts(&bytes).expect("demux");
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
    std::fs::write(&file, ts_mux_stage::mux(&media)).unwrap();
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
    std::fs::write(&control_file, ts_mux_stage::mux(&control)).unwrap();
    let control_text = ffprobe_frames(&control_file).expect("ffprobe");
    assert_eq!(control_text.matches("[FRAME]").count(), FRAMES);
    assert_eq!(control_text.matches(FFPROBE_SIDE_DATA).count(), 0);
}
