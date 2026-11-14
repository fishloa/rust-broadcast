//! HDR10+ (SMPTE ST 2094-40) over the real Tears of Steel MPEG-TS stream (issue
//! #1428): the multivariant playlist carries `SUPPLEMENTAL-CODECS` with the
//! `cdm4` brand and `VIDEO-RANGE=PQ`, every fMP4 sample keeps its HDR10+ SEI
//! byte-identical, and external readers (ffprobe, Apple `mediastreamvalidator`)
//! agree.
//!
//! Real artifact: `private/fixtures/video/hdr10plus/` (PROVENANCE.md). Every test
//! drives the production path: `RouteHandle` + `ProgramSegmenter::pump` over a
//! `Trunk`, read back through the axum router. External tools skip loudly when
//! absent.

use crate::support::hdr_route::{
    Source, attribute, joined_mp4, load, master_text, ok, route_app, route_app_from, scratch,
    validator_comments, video_codec,
};
use test_fixtures::process::tool_available;

use broadcast_common::mux::Unpackage;
use broadcast_hls::MasterPlaylist;
use transmux::ts_demux::TsDemux;

const HDR10PLUS: &str = "video/hdr10plus/tos-256x144-hdr10plus.ts";
const HDR10: &str = "video/hdr10/meridian-hdr10-60f.ts";
const HLG: &str = "video/hlg/sollevante-hlg-derived-1080p24-10s.ts";
const SDR: &str = "ts/hevc/main10.ts";
const FRAMES: usize = 259;

use crate::support::shared;

const HDR10PLUS_SIDE_DATA: &str = "HDR Dynamic Metadata SMPTE2094-40 (HDR10+)";
const NAL_LENGTH_PREFIX: usize = 4;

fn strip_hdr10plus_sei(sources: Vec<Source>) -> Vec<Source> {
    sources
        .into_iter()
        .map(|mut src| {
            src.spec.hdr10plus = false;
            for sample in &mut src.samples {
                let mut out = Vec::with_capacity(sample.data.len());
                let mut at = 0;
                while at + NAL_LENGTH_PREFIX <= sample.data.len() {
                    let len = u32::from_be_bytes(
                        sample.data[at..at + NAL_LENGTH_PREFIX].try_into().unwrap(),
                    ) as usize;
                    let end = at + NAL_LENGTH_PREFIX + len;
                    let nal = &sample.data[at + NAL_LENGTH_PREFIX..end];
                    if !transmux::Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(nal) {
                        out.extend_from_slice(&sample.data[at..end]);
                    }
                    at = end;
                }
                sample.data = out.into();
            }
            src
        })
        .collect()
}

#[tokio::test]
async fn master_announces_hdr10plus_with_the_cdm4_brand_and_pq() {
    if test_fixtures::require(HDR10PLUS, "hdr10plus_route").is_none() {
        return;
    }
    let text = master_text(&route_app(HDR10PLUS)).await;
    MasterPlaylist::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(video_codec(&text), "hvc1.2.4.H153.90", "{text}");
    assert_eq!(
        attribute(&text, "SUPPLEMENTAL-CODECS").as_deref(),
        Some("hvc1.2.4.H153.90/cdm4"),
        "{text}"
    );
    assert_eq!(
        attribute(&text, "VIDEO-RANGE").as_deref(),
        Some("PQ"),
        "{text}"
    );
}

#[tokio::test]
async fn routes_without_hdr10plus_carry_no_supplemental_codecs() {
    for fixture in [HDR10, HLG, SDR] {
        if test_fixtures::require(fixture, "hdr10plus_route").is_none() {
            continue;
        }
        let text = master_text(&route_app(fixture)).await;
        assert!(!text.contains("SUPPLEMENTAL-CODECS"), "{fixture}\n{text}");
        assert!(!text.contains("cdm4"), "{fixture}\n{text}");
    }
    if test_fixtures::require(HDR10PLUS, "hdr10plus_route").is_some() {
        let stripped = route_app_from(strip_hdr10plus_sei(load(HDR10PLUS)));
        let text = master_text(&stripped).await;
        assert!(!text.contains("SUPPLEMENTAL-CODECS"), "{text}");
    }
}

fn hdr10plus_seis_by_pts(media: &transmux::Media) -> Vec<Vec<u8>> {
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Hevc { .. }))
        .expect("HEVC track");
    let mut samples: Vec<_> = track.samples.iter().collect();
    samples.sort_by_key(|s| s.timing.map(|t| t.pts));
    samples
        .iter()
        .flat_map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .filter(|n| transmux::Hdr10PlusMetadata::hevc_sei_nal_carries_hdr10plus(n))
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn every_fmp4_sample_of_the_route_keeps_its_hdr10plus_sei_byte_identical() {
    if test_fixtures::require(HDR10PLUS, "hdr10plus_route").is_none() {
        return;
    }
    let source = {
        let bytes = test_fixtures::require_bytes(HDR10PLUS, "hdr10plus_route").unwrap();
        hdr10plus_seis_by_pts(&TsDemux::new().unpackage(&bytes).unwrap())
    };
    assert_eq!(source.len(), FRAMES);
    let file = joined_mp4(&route_app(HDR10PLUS), &scratch("hdr10plus-passthrough")).await;
    let served = transmux::Fmp4Demux::new()
        .unpackage(&std::fs::read(file).unwrap())
        .expect("demux the served segments");
    let seis = hdr10plus_seis_by_pts(&served);
    assert_eq!(seis.len(), FRAMES);
    assert_eq!(seis, source);
}

fn ffprobe_frames(file: &std::path::Path) -> String {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_frames", "-select_streams", "v"])
        .arg(file)
        .output()
        .expect("ffprobe");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[tokio::test]
async fn ffprobe_reports_hdr10plus_side_data_on_every_frame_of_our_output() {
    if test_fixtures::require(HDR10PLUS, "hdr10plus_route").is_none() {
        return;
    }
    if !tool_available("ffprobe", "-version") {
        eprintln!(
            "SKIP ffprobe_reports_hdr10plus_side_data_on_every_frame_of_our_output: ffprobe not on PATH"
        );
        return;
    }
    let with = joined_mp4(&route_app(HDR10PLUS), &scratch("hdr10plus-ffprobe")).await;
    let text = ffprobe_frames(&with);
    assert_eq!(text.matches("[FRAME]").count(), FRAMES, "{text}");
    assert_eq!(text.matches(HDR10PLUS_SIDE_DATA).count(), FRAMES);
    let control = joined_mp4(
        &route_app_from(strip_hdr10plus_sei(load(HDR10PLUS))),
        &scratch("hdr10plus-ffprobe-control"),
    )
    .await;
    let control_text = ffprobe_frames(&control);
    assert_eq!(control_text.matches("[FRAME]").count(), FRAMES);
    assert_eq!(
        control_text.matches(HDR10PLUS_SIDE_DATA).count(),
        0,
        "the control route has no HDR10+ SEI yet ffprobe reports it"
    );
}

const PART_FLOOR_FINDING: &str = "at least 85% of PART-TARGET";

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
async fn mediastreamvalidator_reports_no_hdr10plus_specific_finding() {
    if !tool_available("mediastreamvalidator", "--version") {
        eprintln!(
            "SKIP mediastreamvalidator_reports_no_hdr10plus_specific_finding: mediastreamvalidator not on PATH"
        );
        return;
    }
    if test_fixtures::require(HDR10PLUS, "hdr10plus_route").is_none() {
        return;
    }
    let base_runs: Vec<_> = [(SDR, "hdr10plus-base-sdr"), (HDR10, "hdr10plus-base-hdr10")]
        .into_iter()
        .filter(|(fixture, _)| test_fixtures::require(fixture, "hdr10plus_route").is_some())
        .collect();
    let (baseline, comments) = std::thread::scope(|scope| {
        let bases: Vec<_> = base_runs
            .iter()
            .map(|&(fixture, name)| {
                scope.spawn(move || shared::run_async(validator_comments(route_app(fixture), name)))
            })
            .collect();
        let stripped = scope.spawn(|| {
            shared::run_async(validator_comments(
                route_app_from(strip_hdr10plus_sei(load(HDR10PLUS))),
                "hdr10plus-base-same-stream-without-sei",
            ))
        });
        let value = scope
            .spawn(|| shared::run_async(validator_comments(route_app(HDR10PLUS), "hdr10plus-val")));
        let mut baseline = Vec::new();
        for h in bases {
            baseline.extend(h.join().expect("validator thread"));
        }
        baseline.extend(stripped.join().expect("validator thread"));
        (baseline, value.join().expect("validator thread"))
    });
    for c in &comments {
        eprintln!("VALIDATOR hdr10plus: {c}");
    }
    for c in baseline.iter().chain(&comments) {
        assert!(
            !c.contains(PART_FLOOR_FINDING),
            "validator reports the 85% part floor: {c}"
        );
    }
    let issue_1602 = has_b_frame_reordering(HDR10PLUS);
    let only: Vec<_> = comments
        .iter()
        .filter(|c| !baseline.contains(c))
        .filter(|c| !(issue_1602 && ISSUE_1602_FINDINGS.contains(&c.as_str())))
        .collect();
    assert!(
        only.is_empty(),
        "findings beyond the SDR/HDR10 baseline: {only:?}"
    );
}

#[tokio::test]
async fn parts_at_23_976_fps_sit_on_the_frame_grid() {
    if test_fixtures::require(HDR10PLUS, "hdr10plus_route").is_none() {
        return;
    }
    let text = String::from_utf8(ok(&route_app(HDR10PLUS), "media.m3u8").await).expect("utf8");
    let value = |tag: &str, key: &str| -> f64 {
        let line = text
            .lines()
            .find(|l| l.starts_with(tag))
            .unwrap_or_else(|| panic!("{tag} missing:\n{text}"));
        let start = line.find(key).expect(key) + key.len();
        line[start..]
            .split(',')
            .next()
            .and_then(|v| v.parse().ok())
            .expect("number")
    };
    let target = value("#EXT-X-PART-INF:", "PART-TARGET=");
    let hold_back = value("#EXT-X-SERVER-CONTROL:", "PART-HOLD-BACK=");
    assert!(target > 0.2, "200 ms holds 4.8 frames; got {target}");
    assert!(hold_back >= 3.0 * target - 1e-9, "{hold_back} vs {target}");
    let mut segments: Vec<Vec<(f64, bool)>> = vec![Vec::new()];
    for line in text.lines() {
        if line.starts_with("#EXT-X-PART:") {
            let start = line.find("DURATION=").expect("duration") + "DURATION=".len();
            let duration: f64 = line[start..]
                .split(',')
                .next()
                .and_then(|v| v.parse().ok())
                .expect("number");
            let exempt = line.contains("INDEPENDENT=YES") || line.contains("GAP=YES");
            segments
                .last_mut()
                .expect("segment")
                .push((duration, exempt));
        } else if line.starts_with("#EXTINF") {
            segments.push(Vec::new());
        }
    }
    let mut checked = 0;
    for parts in &segments {
        for (i, (duration, exempt)) in parts.iter().enumerate() {
            assert!(*duration <= target + 1e-9, "{duration} > {target}");
            if i + 1 < parts.len() && !exempt {
                assert!(
                    *duration >= 0.85 * target - 1e-9,
                    "{duration} < 85% of {target}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 10, "only {checked} non-terminal parts checked");
}
