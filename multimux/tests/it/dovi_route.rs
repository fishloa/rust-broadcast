//! Dolby Vision profiles 5, 8.1 and 8.4 over real MPEG-TS streams (issue
//! #1428): the multivariant playlist carries the CODECS / SUPPLEMENTAL-CODECS /
//! VIDEO-RANGE the Apple HLS authoring spec prescribes, the init segment carries
//! the `dvcC` / `dvvC` box, and external readers (ffprobe, GPAC, Apple
//! `mediastreamvalidator`) agree.
//!
//! Real artifacts: `private/fixtures/video/dolby-vision/` (PROVENANCE.md). Every test
//! drives the production path: `RouteHandle` + `ProgramSegmenter::pump` over a
//! `Trunk`, read back through the axum router. External tools skip loudly when
//! absent.

use crate::support::hdr_route::{
    Source, attribute, contains, init_bytes, joined_mp4, load, master_text, route_app,
    route_app_from, scratch, validator_comments, video_codec,
};
use test_fixtures::process::tool_available;

use broadcast_hls::MasterPlaylist;

const P81: &str = "video/dolby-vision/sollevante-dv-p8.1-1080p24-10s.ts";
const P5: &str = "video/dolby-vision/sollevante-dv-p5-1080p24-10s.ts";
const P84: &str = "video/dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.ts";
const HDR10: &str = "video/hdr10/meridian-hdr10-60f.ts";
const HLG: &str = "video/hlg/sollevante-hlg-derived-1080p24-10s.ts";
const SDR: &str = "ts/hevc/main10.ts";

fn without_dovi(mut sources: Vec<Source>) -> Vec<Source> {
    for src in &mut sources {
        if src.spec.dovi.is_some() {
            src.spec.dovi = None;
        }
    }
    assert!(sources.iter().all(|s| s.spec.dovi.is_none()));
    sources
}

use crate::support::shared;

struct Expected {
    codec: &'static str,
    supplemental: Option<&'static str>,
    range: &'static str,
}

async fn assert_master(fixture: &str, want: Expected) {
    if test_fixtures::require(fixture, "dovi_route").is_none() {
        return;
    }
    let app = route_app(fixture);
    let text = master_text(&app).await;
    MasterPlaylist::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(video_codec(&text), want.codec, "{text}");
    assert_eq!(
        attribute(&text, "SUPPLEMENTAL-CODECS").as_deref(),
        want.supplemental,
        "{text}"
    );
    assert_eq!(
        attribute(&text, "VIDEO-RANGE").as_deref(),
        Some(want.range),
        "{text}"
    );
}

#[tokio::test]
async fn p81_master_has_hdr10_base_codecs_supplemental_dvh1_db1p_and_pq() {
    assert_master(
        P81,
        Expected {
            codec: "hvc1.2.4.L153.B0",
            supplemental: Some("dvh1.08.03/db1p"),
            range: "PQ",
        },
    )
    .await;
}

#[tokio::test]
async fn p5_master_has_dvh1_codecs_no_supplemental_and_pq() {
    assert_master(
        P5,
        Expected {
            codec: "dvh1.05.03",
            supplemental: None,
            range: "PQ",
        },
    )
    .await;
}

#[tokio::test]
async fn p84_master_has_hlg_base_codecs_supplemental_dvh1_db4h_and_hlg() {
    assert_master(
        P84,
        Expected {
            codec: "hvc1.2.4.L150.B0",
            supplemental: Some("dvh1.08.07/db4h"),
            range: "HLG",
        },
    )
    .await;
}

#[tokio::test]
async fn every_video_variant_carries_the_measured_frame_rate() {
    for (fixture, expected) in [
        (P81, "24.000"),
        (P5, "24.000"),
        (P84, "30.000"),
        (HLG, "24.000"),
    ] {
        if test_fixtures::require(fixture, "dovi_route").is_none() {
            continue;
        }
        let text = master_text(&route_app(fixture)).await;
        assert_eq!(
            attribute(&text, "FRAME-RATE").as_deref(),
            Some(expected),
            "{fixture}\n{text}"
        );
    }
}

#[tokio::test]
async fn non_dolby_vision_routes_have_no_supplemental_codecs() {
    for fixture in [HDR10, HLG, SDR] {
        if test_fixtures::require(fixture, "dovi_route").is_none() {
            continue;
        }
        let text = master_text(&route_app(fixture)).await;
        assert!(!text.contains("SUPPLEMENTAL-CODECS"), "{fixture}\n{text}");
        assert!(!video_codec(&text).starts_with("dvh1"), "{fixture}\n{text}");
    }
}

#[tokio::test]
async fn p5_init_carries_dvh1_and_dvcc_and_p81_carries_hvc1_and_dvvc() {
    for (fixture, entry, config_box, other_box) in [
        (P5, b"dvh1", b"dvcC", b"dvvC"),
        (P81, b"hvc1", b"dvvC", b"dvcC"),
        (P84, b"hvc1", b"dvvC", b"dvcC"),
    ] {
        if test_fixtures::require(fixture, "dovi_route").is_none() {
            continue;
        }
        let init = init_bytes(&route_app(fixture)).await;
        assert!(contains(&init, entry), "{fixture}");
        assert!(contains(&init, config_box), "{fixture}");
        assert!(!contains(&init, other_box), "{fixture}");
    }
}

fn ffprobe_streams(file: &std::path::Path) -> String {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_entries",
            "stream_side_data",
        ])
        .arg(file)
        .output()
        .expect("ffprobe");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[tokio::test]
async fn ffprobe_reads_dolby_vision_configuration_only_from_our_box() {
    if !tool_available("ffprobe", "-version") {
        eprintln!(
            "SKIP ffprobe_reads_dolby_vision_configuration_only_from_our_box: ffprobe not on PATH"
        );
        return;
    }
    for (fixture, profile, level, compat) in [(P81, 8, 3, 1), (P5, 5, 3, 0), (P84, 8, 7, 4)] {
        if test_fixtures::require(fixture, "dovi_route").is_none() {
            continue;
        }
        let name = format!("dovi-ffprobe-{profile}-{compat}");
        let with = joined_mp4(&route_app(fixture), &scratch(&name)).await;
        let text = ffprobe_streams(&with);
        for needle in [
            "side_data_type=DOVI configuration record".to_string(),
            format!("dv_profile={profile}"),
            format!("dv_level={level}"),
            format!("dv_bl_signal_compatibility_id={compat}"),
            "rpu_present_flag=1".to_string(),
            "bl_present_flag=1".to_string(),
        ] {
            assert!(text.contains(&needle), "{fixture}: lacks {needle}\n{text}");
        }
        let control = joined_mp4(
            &route_app_from(without_dovi(load(fixture))),
            &scratch(&format!("{name}-control")),
        )
        .await;
        let control_text = ffprobe_streams(&control);
        assert!(
            !control_text.contains("dv_profile"),
            "{fixture}: the control route has no dvcC/dvvC yet ffprobe reports Dolby Vision\n{control_text}"
        );
    }
}

#[tokio::test]
async fn gpac_dumps_the_dolby_vision_configuration_box() {
    if !tool_available("MP4Box", "-version") {
        eprintln!("SKIP gpac_dumps_the_dolby_vision_configuration_box: MP4Box not on PATH");
        return;
    }
    for (fixture, box_name) in [(P81, "dvvC"), (P5, "dvcC"), (P84, "dvvC")] {
        if test_fixtures::require(fixture, "dovi_route").is_none() {
            continue;
        }
        let app = route_app(fixture);
        let dir = scratch(&format!("dovi-gpac-{box_name}-{}", fixture.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let init = dir.join("init.mp4");
        std::fs::write(&init, init_bytes(&app).await).unwrap();
        let out = std::process::Command::new("MP4Box")
            .arg("-diso")
            .arg(&init)
            .current_dir(&dir)
            .output()
            .expect("MP4Box");
        let xml = std::fs::read_to_string(dir.join("init_info.xml"))
            .unwrap_or_else(|_| String::from_utf8_lossy(&out.stdout).to_string());
        assert!(
            xml.contains("DOLBY") || xml.contains("Dolby") || xml.contains("DOVI"),
            "{fixture}\n{xml}"
        );
        assert!(
            xml.contains(box_name) || xml.contains("DOVIConfigurationBox"),
            "{fixture}\n{xml}"
        );
    }
}

const ISSUE_1602_FINDINGS: [&str; 2] = [
    "Playlist vs segment duration mismatch",
    "Playlist vs partial segment duration mismatch",
];

fn has_b_frame_reordering(fixture: &str) -> bool {
    load(fixture)
        .iter()
        .flat_map(|s| s.samples.iter())
        .any(|s| s.timing.map(|t| t.pts) != s.timing.map(|t| t.dts))
}

#[tokio::test]
async fn mediastreamvalidator_reports_no_dolby_vision_specific_finding() {
    if !tool_available("mediastreamvalidator", "--version") {
        eprintln!(
            "SKIP mediastreamvalidator_reports_no_dolby_vision_specific_finding: mediastreamvalidator not on PATH"
        );
        return;
    }
    let present = |fixtures: &[(&'static str, &'static str)]| -> Vec<(&'static str, &'static str)> {
        fixtures
            .iter()
            .copied()
            .filter(|(fixture, _)| test_fixtures::require(fixture, "dovi_route").is_some())
            .collect()
    };
    let baseline_runs = present(&[
        (SDR, "dovi-base-sdr"),
        (HDR10, "dovi-base-hdr10"),
        (HLG, "dovi-base-hlg"),
    ]);
    let value_runs = present(&[
        (P81, "dovi-val-p81"),
        (P5, "dovi-val-p5"),
        (P84, "dovi-val-p84"),
    ]);
    let mut all = baseline_runs.clone();
    all.extend(value_runs.iter().copied());
    let mut results = std::thread::scope(|scope| {
        let handles: Vec<_> = all
            .iter()
            .map(|&(fixture, name)| {
                scope.spawn(move || shared::run_async(validator_comments(route_app(fixture), name)))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("validator thread"))
            .collect::<Vec<_>>()
            .into_iter()
    });
    let mut baseline = Vec::new();
    for _ in &baseline_runs {
        baseline.extend(results.next().expect("baseline result"));
    }
    for &(fixture, name) in &value_runs {
        let comments = results.next().expect("value result");
        for c in &comments {
            eprintln!("VALIDATOR {name}: {c}");
        }
        assert!(
            !comments.iter().any(|c| c.contains("FRAME-RATE")),
            "{fixture}: {comments:?}"
        );
        let issue_1602 = has_b_frame_reordering(fixture);
        let only: Vec<_> = comments
            .iter()
            .filter(|c| !baseline.contains(c))
            .filter(|c| !(issue_1602 && ISSUE_1602_FINDINGS.contains(&c.as_str())))
            .collect();
        assert!(
            only.is_empty(),
            "{fixture}: findings beyond the SDR/HDR10/HLG baseline: {only:?}"
        );
    }
}
