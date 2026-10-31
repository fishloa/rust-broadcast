//! Dolby Vision profiles 5, 8.1 and 8.4 over real MPEG-TS streams (issue
//! #1428): the multivariant playlist carries the CODECS / SUPPLEMENTAL-CODECS /
//! VIDEO-RANGE the Apple HLS authoring spec prescribes, the init segment carries
//! the `dvcC` / `dvvC` box, and external readers (ffprobe, GPAC, Apple
//! `mediastreamvalidator`) agree.
//!
//! Real artifacts: `private/fixtures/dolby-vision/` (PROVENANCE.md). Every test
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
use broadcast_hls::{MasterPlaylist, MediaPlaylist};
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
const P81: &str = "dolby-vision/sollevante-dv-p8.1-1080p24-10s.ts";
const P5: &str = "dolby-vision/sollevante-dv-p5-1080p24-10s.ts";
const P84: &str = "dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.ts";
const HDR10: &str = "hdr10/meridian-hdr10-60f.ts";
const HLG: &str = "hlg/sollevante-hlg-derived-1080p24-10s.ts";
const SDR: &str = "ts/hevc/main10.ts";

struct Source {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

fn without_dovi(mut sources: Vec<Source>) -> Vec<Source> {
    for src in &mut sources {
        if src.spec.dovi.is_some() {
            src.spec.dovi = None;
        }
    }
    assert!(sources.iter().all(|s| s.spec.dovi.is_none()));
    sources
}

fn load(fixture: &str) -> Vec<Source> {
    let bytes = test_fixtures::require_bytes(fixture, "dovi_route").expect("fixture");
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

fn route_app(fixture: &str) -> Router {
    route_app_from(load(fixture))
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

async fn init_bytes(app: &Router) -> Vec<u8> {
    let text = String::from_utf8(ok(app, "media.m3u8").await).expect("utf8");
    let playlist =
        MediaPlaylist::parse(&text).unwrap_or_else(|e| panic!("media must parse: {e}\n{text}"));
    let map = playlist.segments[0]
        .map
        .as_ref()
        .expect("EXT-X-MAP")
        .uri
        .clone();
    ok(app, &map).await
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
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
async fn mediastreamvalidator_reports_no_dolby_vision_specific_finding() {
    if !tool_available("mediastreamvalidator", "--version") {
        eprintln!(
            "SKIP mediastreamvalidator_reports_no_dolby_vision_specific_finding: mediastreamvalidator not on PATH"
        );
        return;
    }
    let mut baseline = Vec::new();
    for (fixture, name) in [
        (SDR, "dovi-base-sdr"),
        (HDR10, "dovi-base-hdr10"),
        (HLG, "dovi-base-hlg"),
    ] {
        if test_fixtures::require(fixture, "dovi_route").is_some() {
            baseline.extend(validator_comments(route_app(fixture), name).await);
        }
    }
    for (fixture, name) in [
        (P81, "dovi-val-p81"),
        (P5, "dovi-val-p5"),
        (P84, "dovi-val-p84"),
    ] {
        if test_fixtures::require(fixture, "dovi_route").is_none() {
            continue;
        }
        baseline.extend(
            validator_comments(
                route_app_from(without_dovi(load(fixture))),
                &format!("{name}-control"),
            )
            .await,
        );
        let comments = validator_comments(route_app(fixture), name).await;
        for c in &comments {
            eprintln!("VALIDATOR {name}: {c}");
        }
        let only: Vec<_> = comments.iter().filter(|c| !baseline.contains(c)).collect();
        assert!(
            only.is_empty(),
            "{fixture}: findings beyond the SDR/HDR10/HLG baseline: {only:?}"
        );
    }
}
