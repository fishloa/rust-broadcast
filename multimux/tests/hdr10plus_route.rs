//! HDR10+ (SMPTE ST 2094-40) over the real Tears of Steel MPEG-TS stream (issue
//! #1428): the multivariant playlist carries `SUPPLEMENTAL-CODECS` with the
//! `cdm4` brand and `VIDEO-RANGE=PQ`, every fMP4 sample keeps its HDR10+ SEI
//! byte-identical, and external readers (ffprobe, Apple `mediastreamvalidator`)
//! agree.
//!
//! Real artifact: `private/fixtures/hdr10plus/` (PROVENANCE.md). Every test
//! drives the production path: `RouteHandle` + `ProgramSegmenter::pump` over a
//! `Trunk`, read back through the axum router. External tools skip loudly when
//! absent.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_common::mux::Unpackage;
use broadcast_hls::MasterPlaylist;
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use tower::ServiceExt;
use transmux::pipeline::{Sample, TrackSpec};
use transmux::ts_demux::TsDemux;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use multimux::source::segment::ProgramSegmenter;

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 200;
const WINDOW_SEGMENTS: usize = 64;
const RING: usize = 1 << 16;
const PUMP_EVERY: usize = 64;
const STREAM: &str = "cam";
const DECLARED_BPS: u64 = 4_000_000;
const HDR10PLUS: &str = "hdr10plus/tos-256x144-hdr10plus.ts";
const HDR10: &str = "hdr10/meridian-hdr10-60f.ts";
const HLG: &str = "hlg/sollevante-hlg-derived-1080p24-10s.ts";
const SDR: &str = "ts/hevc/main10.ts";
const FRAMES: usize = 259;

struct Source {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

fn load(fixture: &str) -> Vec<Source> {
    let bytes = test_fixtures::require_bytes(fixture, "hdr10plus_route").expect("fixture");
    let media = TsDemux::new().unpackage(&bytes).expect("demux fixture");
    media
        .tracks
        .into_iter()
        .map(|t| Source {
            spec: t.spec,
            samples: t
                .samples
                .into_iter()
                .map(|mut s| {
                    s.dts = s.dts.or(s.pts);
                    s
                })
                .collect(),
        })
        .collect()
}

fn new_trunk() -> Arc<Trunk> {
    let nz = |n: usize| NonZeroUsize::new(n).expect("non-zero");
    Trunk::new(TrunkConfig::new(
        nz(RING),
        nz(64),
        nz(WINDOW_SEGMENTS),
        nz(64),
        nz(RING),
    ))
}

#[path = "support/shared.rs"]
mod shared;

/// One route per fixture, built once per process. The first request to a
/// freshly built route populates its playlist state, and two tests making that
/// first request concurrently see an empty playlist on one of them, so the
/// builder makes it before the route is shared.
fn route_app(fixture: &str) -> Router {
    shared::shared(&format!("route:{fixture}"), || {
        let app = route_app_from(load(fixture));
        shared::run_async(async {
            let _ = get(&app, "media.m3u8").await;
            let _ = get(&app, "master.m3u8").await;
        });
        app
    })
    .clone()
}

fn route_app_from(sources: Vec<Source>) -> Router {
    let trunk = new_trunk();
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(sources.iter().map(|s| s.spec.clone()).collect());
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(Some(DECLARED_BPS)),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    route.set_track_specs(ProgramId(0), trunk.tracks().to_vec());
    let mut segmenter =
        ProgramSegmenter::try_new(&trunk, &route, TARGET_DURATION_SECS, PART_TARGET_MS)
            .expect("segmenter");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }
    let mut order: Vec<(f64, usize, usize)> = Vec::new();
    for (ti, src) in sources.iter().enumerate() {
        if !src.spec.config.is_muxable_in_bmff() {
            continue;
        }
        let scale = f64::from(src.spec.timescale);
        for (si, s) in src.samples.iter().enumerate() {
            order.push((s.dts.unwrap_or(0) as f64 / scale, ti, si));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (n, (_, ti, si)) in order.into_iter().enumerate() {
        writer.publish(
            sources[ti].spec.track_id,
            RetentionClass::Timed,
            sources[ti].samples[si].clone(),
        );
        if n % PUMP_EVERY == 0 {
            segmenter.pump(ProgramId(0), &trunk, &route);
        }
    }
    segmenter.pump(ProgramId(0), &trunk, &route);
    segmenter.flush();
    let mut streams = HashMap::new();
    streams.insert(
        STREAM.to_string(),
        (
            route,
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    router(Arc::new(AppState::new(streams)))
}

async fn get(app: &Router, path: &str) -> (StatusCode, Vec<u8>) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/{STREAM}/{path}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    (status, body)
}

async fn ok(app: &Router, path: &str) -> Vec<u8> {
    let (status, body) = get(app, path).await;
    assert_eq!(status, StatusCode::OK, "GET {path}");
    body
}

fn tool_available(tool: &str, arg: &str) -> bool {
    std::process::Command::new(tool)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn master_text(app: &Router) -> String {
    String::from_utf8(ok(app, "master.m3u8").await).expect("utf8")
}

async fn dump_route(app: &Router, dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        std::fs::write(dir.join(name), ok(app, name).await).unwrap();
    }
    let mut lines = Vec::new();
    for name in ["media.m3u8", "iframe.m3u8"] {
        lines.extend(
            std::fs::read_to_string(dir.join(name))
                .unwrap()
                .lines()
                .map(str::to_string),
        );
    }
    for line in &lines {
        let uri = match line.find("URI=\"") {
            Some(s) => line[s + 5..].split('"').next().map(str::to_string),
            None if !line.trim().is_empty() && !line.starts_with('#') => {
                Some(line.trim().to_string())
            }
            None => None,
        };
        if let Some(uri) = uri {
            let (status, body) = get(app, &uri).await;
            if status == StatusCode::OK {
                std::fs::write(dir.join(&uri), body).unwrap();
            }
        }
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn attribute(text: &str, name: &str) -> Option<String> {
    let line = text.lines().find(|l| l.starts_with("#EXT-X-STREAM-INF:"))?;
    let key = format!("{name}=");
    let mut rest = line.strip_prefix("#EXT-X-STREAM-INF:")?;
    while !rest.is_empty() {
        let (attr_name, after) = rest.split_once('=')?;
        let (value, remainder) = if let Some(quoted) = after.strip_prefix('"') {
            let (v, r) = quoted.split_once('"')?;
            (v, r.strip_prefix(',').unwrap_or(r))
        } else {
            after.split_once(',').unwrap_or((after, ""))
        };
        if format!("{attr_name}=") == key {
            return Some(value.to_string());
        }
        rest = remainder;
    }
    None
}

fn video_codec(master: &str) -> String {
    attribute(master, "CODECS")
        .expect("CODECS")
        .split(',')
        .next()
        .unwrap()
        .to_string()
}

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

async fn joined_mp4(app: &Router, dir: &std::path::Path) -> std::path::PathBuf {
    dump_route(app, dir).await;
    let media = std::fs::read_to_string(dir.join("media.m3u8")).unwrap();
    let mut joined = Vec::new();
    for line in media.lines() {
        if line.starts_with("#EXT-X-MAP") {
            let s = line.find("URI=\"").unwrap();
            let uri = line[s + 5..].split('"').next().unwrap();
            joined.extend(std::fs::read(dir.join(uri)).unwrap());
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            joined.extend(std::fs::read(dir.join(line.trim())).unwrap());
        }
    }
    let file = dir.join("joined.mp4");
    std::fs::write(&file, joined).unwrap();
    file
}

fn hdr10plus_seis_by_pts(media: &transmux::Media) -> Vec<Vec<u8>> {
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Hevc { .. }))
        .expect("HEVC track");
    let mut samples: Vec<_> = track.samples.iter().collect();
    samples.sort_by_key(|s| s.pts);
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
        .any(|s| s.pts != s.dts)
}

async fn validator_comments(app: Router, name: &str) -> Vec<String> {
    let dir = scratch(name);
    dump_route(&app, &dir).await;
    let report = dir.join("report.json");
    let mut cmd = std::process::Command::new("mediastreamvalidator");
    cmd.current_dir(&dir)
        .arg("--quiet")
        .arg("-t")
        .arg("5")
        .arg("-O")
        .arg(&report)
        .arg("master.m3u8");
    test_bounded::output_bounded(&mut cmd, std::time::Duration::from_secs(35)).expect("run");
    let text = std::fs::read_to_string(&report).expect("validator report");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut comments = Vec::new();
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(c) = m.get("errorComment").and_then(|c| c.as_str()) {
                    out.push(c.to_string());
                }
                m.values().for_each(|c| walk(c, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    walk(&json, &mut comments);
    comments.sort();
    comments.dedup();
    comments
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
    let (mut baseline, comments) = std::thread::scope(|scope| {
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
