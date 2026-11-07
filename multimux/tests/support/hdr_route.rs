//! Shared helpers for the multimux HEVC HDR route tests (`hdr10_route`, `hdr10plus_route`, `dovi_route`, `heaac_override`): one `RouteHandle` + `ProgramSegmenter` over a `Trunk`, read back through the axum router.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_common::mux::Unpackage;
use broadcast_hls::MediaPlaylist;
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use test_fixtures::process::msv;
use tower::ServiceExt;
use transmux::pipeline::{Sample, TrackSpec};
use transmux::ts_demux::TsDemux;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use multimux::source::segment::ProgramSegmenter;

use crate::support::shared;

pub const TARGET_DURATION_SECS: f64 = 1.0;
pub const PART_TARGET_MS: u32 = 200;
pub const WINDOW_SEGMENTS: usize = 64;
pub const RING: usize = 1 << 16;
pub const PUMP_EVERY: usize = 64;
pub const STREAM: &str = "cam";
pub const DECLARED_BPS: u64 = 4_000_000;

pub struct Source {
    pub spec: TrackSpec,
    pub samples: Vec<Sample>,
}

pub fn load(fixture: &str) -> Vec<Source> {
    let bytes = test_fixtures::require_bytes(fixture, "hevc_route").expect("fixture");
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

pub fn new_trunk() -> Arc<Trunk> {
    let nz = |n: usize| NonZeroUsize::new(n).expect("non-zero");
    Trunk::new(TrunkConfig::new(
        nz(RING),
        nz(64),
        nz(WINDOW_SEGMENTS),
        nz(64),
        nz(RING),
    ))
}

/// One route per fixture, built once per process. The first request to a
/// freshly built route populates its playlist state, and two tests making that
/// first request concurrently see an empty playlist on one of them, so the
/// builder makes it before the route is shared.
pub fn route_app(fixture: &str) -> Router {
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

pub fn route_app_from(sources: Vec<Source>) -> Router {
    route_app_with_profiles(sources, Vec::new())
}

pub fn route_app_with_profiles(
    sources: Vec<Source>,
    profiles: Vec<multimux::config::AudioProfileOverride>,
) -> Router {
    let trunk = new_trunk();
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(sources.iter().map(|s| s.spec.clone()).collect());
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(Some(DECLARED_BPS))
            .with_audio_profiles(profiles),
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

pub async fn get(app: &Router, path: &str) -> (StatusCode, Vec<u8>) {
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

pub async fn ok(app: &Router, path: &str) -> Vec<u8> {
    let (status, body) = get(app, path).await;
    assert_eq!(status, StatusCode::OK, "GET {path}");
    body
}

pub async fn master_text(app: &Router) -> String {
    String::from_utf8(ok(app, "master.m3u8").await).expect("utf8")
}

pub async fn dump_route(app: &Router, dir: &std::path::Path) {
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

pub fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

pub fn attribute(text: &str, name: &str) -> Option<String> {
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

pub fn video_codec(master: &str) -> String {
    attribute(master, "CODECS")
        .expect("CODECS")
        .split(',')
        .next()
        .unwrap()
        .to_string()
}

pub async fn joined_mp4(app: &Router, dir: &std::path::Path) -> std::path::PathBuf {
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

pub async fn validator_comments(app: Router, name: &str) -> Vec<String> {
    let dir = scratch(name);
    dump_route(&app, &dir).await;
    let report = msv::run_report(&dir, "master.m3u8", std::time::Duration::from_secs(35));
    msv::error_comments(&report)
}

pub async fn init_bytes(app: &Router) -> Vec<u8> {
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

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
