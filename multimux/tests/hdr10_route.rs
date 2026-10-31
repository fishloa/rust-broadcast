//! HDR10 over a real MPEG-TS stream (issue #1428): the multivariant playlist
//! says `VIDEO-RANGE=PQ`, the init segment carries `colr`/`mdcv`/`clli`, and
//! external readers (ffprobe, GPAC, Apple `mediastreamvalidator`) agree.
//!
//! Real artifact: `private/fixtures/hdr10/meridian-hdr10-60f.ts` (see its
//! PROVENANCE.md). Every test drives the production path: `RouteHandle` +
//! `ProgramSegmenter::pump` over a `Trunk`, read back through the axum router.
//! External tools skip loudly when absent.

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
const HDR_FIXTURE: &str = "hdr10/meridian-hdr10-60f.ts";
const SDR_FIXTURE: &str = "ts/hevc/main10.ts";

struct Source {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

const NAL_LENGTH_PREFIX: usize = 4;
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;

fn strip_sei_nals(data: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(data.len());
    let mut removed = 0;
    let mut at = 0;
    while at + NAL_LENGTH_PREFIX <= data.len() {
        let len = u32::from_be_bytes(data[at..at + NAL_LENGTH_PREFIX].try_into().unwrap()) as usize;
        let end = at + NAL_LENGTH_PREFIX + len;
        let nal_type = (data[at + NAL_LENGTH_PREFIX] >> HEVC_NAL_TYPE_SHIFT) & HEVC_NAL_TYPE_MASK;
        if nal_type == transmux::hdr::HEVC_PREFIX_SEI_NUT {
            removed += 1;
        } else {
            out.extend_from_slice(&data[at..end]);
        }
        at = end;
    }
    (out, removed)
}

fn without_in_band_sei(mut sources: Vec<Source>) -> Vec<Source> {
    for src in &mut sources {
        let transmux::CodecConfig::Hevc { config, .. } = &mut src.spec.config else {
            continue;
        };
        assert!(src.spec.hdr.is_some());
        config
            .config
            .arrays
            .retain(|a| a.nal_unit_type != transmux::hdr::HEVC_PREFIX_SEI_NUT);
        let mut removed = 0;
        for sample in &mut src.samples {
            let (data, n) = strip_sei_nals(&sample.data);
            removed += n;
            sample.data = data.into();
        }
        assert!(removed > 0, "the fixture carries in-band SEI to strip");
    }
    sources
}

fn load(fixture: &str) -> Vec<Source> {
    let bytes = test_fixtures::require_bytes(fixture, "hdr10_route").expect("fixture");
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

#[tokio::test]
async fn hdr10_route_master_says_video_range_pq_and_main10_codecs() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    let app = route_app(HDR_FIXTURE);
    let text = master_text(&app).await;
    let master = MasterPlaylist::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert!(text.contains("VIDEO-RANGE=PQ"), "{text}");
    let codecs = master.variants[0].codecs.clone().expect("CODECS");
    assert!(
        codecs.starts_with("hvc1.2.4.L"),
        "Main 10 from the SPS: {codecs}"
    );
}

#[tokio::test]
async fn sdr_hevc_route_master_has_no_hdr_video_range() {
    if test_fixtures::require(SDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    let app = route_app(SDR_FIXTURE);
    let text = master_text(&app).await;
    assert!(
        !text.contains("VIDEO-RANGE=PQ") && !text.contains("VIDEO-RANGE=HLG"),
        "{text}"
    );
    let init = init_bytes(&app).await;
    assert!(!contains(&init, b"mdcv") && !contains(&init, b"clli"));
}

#[tokio::test]
async fn hdr10_route_init_carries_colr_mdcv_clli() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    let init = init_bytes(&route_app(HDR_FIXTURE)).await;
    for fourcc in [b"colr", b"mdcv", b"clli"] {
        assert!(contains(&init, fourcc), "{fourcc:?}");
    }
}

#[tokio::test]
async fn ffprobe_reads_mastering_display_and_content_light_level() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    if !tool_available("ffprobe", "-version") {
        eprintln!(
            "SKIP ffprobe_reads_mastering_display_and_content_light_level: ffprobe not on PATH"
        );
        return;
    }
    let app = route_app_from(without_in_band_sei(load(HDR_FIXTURE)));
    let dir = scratch("hdr10-ffprobe");
    dump_route(&app, &dir).await;
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
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_frames",
            "-read_intervals",
            "%+#1",
        ])
        .arg(&file)
        .output()
        .expect("ffprobe");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    for needle in [
        "color_transfer=smpte2084",
        "color_primaries=bt2020",
        "side_data_type=Mastering display metadata",
        "side_data_type=Content light level metadata",
        "max_content=1000",
        "max_average=400",
        "max_luminance=10000000/10000",
        "min_luminance=1/10000",
    ] {
        assert!(
            text.contains(needle),
            "ffprobe output lacks {needle}:\n{text}"
        );
    }
}

#[tokio::test]
async fn gpac_dumps_the_mdcv_and_clli_boxes() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    if !tool_available("MP4Box", "-version") {
        eprintln!("SKIP gpac_dumps_the_mdcv_and_clli_boxes: MP4Box not on PATH");
        return;
    }
    let app = route_app(HDR_FIXTURE);
    let dir = scratch("hdr10-gpac");
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
    for needle in [
        "MasteringDisplayColourVolumeBox",
        "ContentLightLevelBox",
        "13250",
        "34500",
        "10000000",
    ] {
        assert!(xml.contains(needle), "MP4Box -diso lacks {needle}:\n{xml}");
    }
}

async fn validator_comments(fixture: &str, name: &str) -> Vec<String> {
    let app = route_app(fixture);
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
async fn mediastreamvalidator_reports_no_hdr_specific_finding() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none()
        || test_fixtures::require(SDR_FIXTURE, "hdr10_route").is_none()
    {
        return;
    }
    if !tool_available("mediastreamvalidator", "--version") {
        eprintln!(
            "SKIP mediastreamvalidator_reports_no_hdr_specific_finding: mediastreamvalidator not on PATH"
        );
        return;
    }
    let hdr = validator_comments(HDR_FIXTURE, "hdr10-validator").await;
    let sdr = validator_comments(SDR_FIXTURE, "sdr-validator").await;
    for c in &hdr {
        eprintln!("VALIDATOR hdr: {c}");
    }
    for c in &sdr {
        eprintln!("VALIDATOR sdr: {c}");
    }
    let only_hdr: Vec<_> = hdr.iter().filter(|c| !sdr.contains(c)).collect();
    assert!(
        only_hdr.is_empty(),
        "findings only the HDR10 route has: {only_hdr:?}"
    );
}
