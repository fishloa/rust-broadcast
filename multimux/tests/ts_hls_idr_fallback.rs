//! `ts_hls` on a stream without IDR pictures (round-9 F1). Open-GOP H.264
//! (recovery-point SEI, zero type-5 NALs) used to serve an empty playlist
//! forever under the `idr_only` default. Real captures: the zenith live
//! captures (private) and the public `gulli-opengop.ts`; `france2.ts` is the
//! IDR-carrying control. Segments are fetched back over real HTTP and
//! demuxed; status and metrics come from the admin API and `/metrics`.
#![cfg(feature = "test-seams")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use broadcast_common::{Package, Unpackage};
use multimux::config::{
    AdminSpec, Config, InputSpec, OutputAuthSpec, Route, TsHlsCutPolicy, TsHlsIdrFallback,
};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on_admin;
use transmux::media::{Media, Track};
use transmux::pipeline::{CodecConfig, Sample};
use transmux::{NalCodec, TsDemux, TsMux, access_unit_is_idr};

#[path = "support/listener.rs"]
mod listener;
use listener::bind_tcp;

const ADMIN_TOKEN: &str = "ts-hls-idr-fallback-admin";
const HANG_GUARD: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const TARGET_SECS: f64 = 1.0;
const TS_PACKET: usize = 188;
const HTTP_CHUNK_PACKETS: usize = 56;
const HTTP_CHUNK_INTERVAL: Duration = Duration::from_millis(10);
const AUTO_BOUND_SECS: f64 = 6.0;
const SHORT_BOUND_TARGETS: u32 = 2;
const PACED_BOUND_TARGETS: u32 = 5;
const FRAME_SLACK_SECS: f64 = 0.2;
const LATE_SLACK_SECS: f64 = 6.0;
const MIN_SEGMENTS: usize = 2;
const MOVIE_TIMESCALE: u32 = 90_000;
const FRANCE2_FIRST_NON_IDR_SYNC: usize = 20;
const FRANCE2_LOOP_REPS: usize = 4;
const GULLI_LOOP_REPS: usize = 3;

fn file_input(path: &Path) -> InputSpec {
    InputSpec::File {
        path: path.to_string_lossy().into_owned(),
        loop_file: false,
    }
}

async fn http_input(path: &Path) -> (InputSpec, tokio::task::JoinHandle<()>) {
    use axum::Router;
    use axum::body::Body;
    use axum::response::IntoResponse;
    use axum::routing::get;

    async fn handler(body: axum::extract::State<Vec<u8>>) -> axum::response::Response {
        let chunks: Vec<Vec<u8>> = body
            .0
            .chunks(HTTP_CHUNK_PACKETS * TS_PACKET)
            .map(<[u8]>::to_vec)
            .collect();
        let stream = futures_util::stream::unfold(chunks.into_iter(), |mut rest| async move {
            tokio::time::sleep(HTTP_CHUNK_INTERVAL).await;
            rest.next()
                .map(|chunk| (Ok::<_, std::io::Error>(chunk), rest))
        });
        (
            [(axum::http::header::CONTENT_TYPE, "video/mp2t")],
            Body::from_stream(stream),
        )
            .into_response()
    }
    let app = Router::new()
        .route("/stream.ts", get(handler))
        .with_state(std::fs::read(path).expect("fixture"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (
        InputSpec::TsHttp {
            url: format!("http://{addr}/stream.ts"),
            auth: None,
        },
        handle,
    )
}

fn route(name: &str, input: InputSpec, fallback: TsHlsIdrFallback) -> Route {
    Route {
        name: name.to_string(),
        input,
        outputs: vec![OutputKind::TsHls],
        dvr: DvrConfig::default(),
        ts_hls_cut: TsHlsCutPolicy::IdrOnly,
        ts_hls_idr_fallback: fallback,
        declared_bandwidth_bps: None,
        audio_renditions: Vec::new(),
        audio_rendition_sources: Vec::new(),
        closed_caption_overrides: Vec::new(),
        audio_profiles: Vec::new(),
    }
}

struct Server {
    media: SocketAddr,
    admin: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
}

fn start(route: Route) -> Server {
    let (media, media_listener) = bind_tcp();
    let (admin, admin_listener) = bind_tcp();
    let cfg = Config {
        bind: media.to_string(),
        target_duration_secs: TARGET_SECS,
        part_target_ms: 200,
        window_segments: 64,
        routes: vec![route],
        admin: Some(AdminSpec {
            bind: admin.to_string(),
            auth: OutputAuthSpec::Bearer {
                token: ADMIN_TOKEN.to_string(),
            },
        }),
        ..Config::default()
    };
    let handle = tokio::spawn(async move {
        let _ = serve_with_registry_on_admin(
            media_listener,
            admin_listener,
            cfg,
            SchemeRegistry::new(),
        )
        .await;
    });
    Server {
        media,
        admin,
        handle,
        client: reqwest::Client::new(),
    }
}

impl Server {
    async fn text(&self, url: &str) -> Option<String> {
        let resp = self.client.get(url).send().await.ok()?;
        resp.status().is_success().then_some(())?;
        resp.text().await.ok()
    }

    async fn bytes(&self, url: &str) -> Vec<u8> {
        let resp = self.client.get(url).send().await.expect("GET");
        assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET {url}");
        resp.bytes().await.expect("body").to_vec()
    }

    async fn status(&self, name: &str) -> serde_json::Value {
        let resp = self
            .client
            .get(format!("http://{}/admin/routes/{name}", self.admin))
            .bearer_auth(ADMIN_TOKEN)
            .send()
            .await
            .expect("admin GET");
        resp.json().await.expect("admin json")
    }

    async fn metrics(&self) -> String {
        self.text(&format!("http://{}/metrics", self.media))
            .await
            .expect("metrics")
    }

    async fn wait_for_idr_status(&self, name: &str, want: &str) {
        let deadline = tokio::time::Instant::now() + HANG_GUARD;
        loop {
            if self.status(name).await["ts_hls_idr"].as_str() == Some(want) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "ts_hls_idr never became {want:?}: {}",
                self.status(name).await
            );
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn segments(&self, name: &str, want: usize) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + HANG_GUARD;
        loop {
            if let Some(body) = self
                .text(&format!("http://{}/{name}/media.m3u8", self.media))
                .await
            {
                let uris: Vec<String> = body
                    .lines()
                    .filter(|l| !l.starts_with('#') && l.ends_with(".ts"))
                    .map(str::to_owned)
                    .collect();
                if uris.len() >= want {
                    return uris;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "route {name} served fewer than {want} segments within the hang guard"
            );
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn playlist(&self, name: &str) -> String {
        self.text(&format!("http://{}/{name}/media.m3u8", self.media))
            .await
            .unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn video_samples(bytes: &[u8]) -> Vec<Sample> {
    let ir: Media = TsDemux::new().unpackage(bytes).expect("demux");
    ir.tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video")
        .samples
}

fn streamed_video(bytes: &[u8]) -> Vec<Sample> {
    use transmux::{DemuxEvent, StreamingTsDemux};
    let mut demux = StreamingTsDemux::new();
    let mut video = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut drain = |demux: &mut StreamingTsDemux| {
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) if matches!(spec.config, CodecConfig::Avc { .. }) => {
                    video.insert(spec.track_id);
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } if video.contains(&track_id) => out.push(sample),
                _ => {}
            }
        }
    };
    for chunk in bytes.chunks(HTTP_CHUNK_PACKETS * TS_PACKET) {
        demux.feed(chunk);
        drain(&mut demux);
    }
    demux.finish();
    drain(&mut demux);
    out
}

fn metric_value(metrics: &str, name: &str, route: &str, mode: &str) -> Option<f64> {
    metrics.lines().find_map(|l| {
        (l.starts_with(name)
            && l.contains(&format!("route=\"{route}\""))
            && l.contains(&format!("mode=\"{mode}\"")))
        .then(|| l.rsplit(' ').next()?.parse().ok())
        .flatten()
    })
}

const COUNTER: &str = "multimux_ts_hls_non_idr_total";

#[derive(Clone, Copy, PartialEq)]
enum Via {
    File,
    Http,
}

async fn assert_falls_back(
    tag: &str,
    path: &Path,
    via: Via,
    fallback: TsHlsIdrFallback,
    bound_secs: f64,
    want_segments: usize,
) {
    let source = std::fs::read(path).expect("fixture");
    let source_video = video_samples(&source);
    assert!(
        !source_video
            .iter()
            .any(|s| access_unit_is_idr(NalCodec::Avc, &s.data)),
        "{tag}: capture must carry no IDR for this test to mean anything"
    );
    let name = format!("fallback-{tag}");
    let (input, _origin) = match via {
        Via::File => (file_input(path), None),
        Via::Http => {
            let (input, handle) = http_input(path).await;
            (input, Some(handle))
        }
    };
    let server = start(route(&name, input, fallback));
    let started = std::time::Instant::now();
    let uris = server.segments(&name, want_segments).await;
    let first = server
        .bytes(&format!("http://{}/{name}/{}", server.media, uris[0]))
        .await;
    let opening = &video_samples(&first)[0];
    let streamed = streamed_video(&source);
    let opens_on_sync = streamed
        .iter()
        .any(|s| s.flags.is_sync && s.data == opening.data);
    assert!(
        opens_on_sync,
        "{tag}: the first segment must open on a non-IDR sync sample of the capture"
    );
    assert!(!access_unit_is_idr(NalCodec::Avc, &opening.data));
    let seen = started.elapsed().as_secs_f64();
    if via == Via::File {
        assert!(
            seen + FRAME_SLACK_SECS >= bound_secs,
            "{tag}: first segment served after {seen:.2}s, before the {bound_secs}s bound"
        );
        assert!(
            seen <= bound_secs + LATE_SLACK_SECS,
            "{tag}: first segment served only after {seen:.2}s (bound {bound_secs}s)"
        );
    }
    assert_eq!(
        server.status(&name).await["ts_hls_idr"].as_str(),
        Some("fell_back")
    );
    assert_eq!(
        metric_value(&server.metrics().await, COUNTER, &name, "fell_back"),
        Some(1.0),
        "{tag}: one fell_back event must be counted"
    );
}

fn short_bound() -> (TsHlsIdrFallback, f64) {
    (
        TsHlsIdrFallback::Targets(SHORT_BOUND_TARGETS),
        f64::from(SHORT_BOUND_TARGETS) * TARGET_SECS,
    )
}

#[tokio::test]
async fn open_gop_public_capture_falls_back_after_the_bound() {
    let fallback = TsHlsIdrFallback::Targets(PACED_BOUND_TARGETS);
    let bound = f64::from(PACED_BOUND_TARGETS) * TARGET_SECS;
    assert_falls_back(
        "gulli",
        &test_fixtures::path("ts/gulli-opengop.ts"),
        Via::File,
        fallback,
        bound,
        MIN_SEGMENTS,
    )
    .await;
}

#[tokio::test]
async fn zenith_arte_falls_back_by_default() {
    let Some(path) = test_fixtures::require("zenith-live/arte-8MB.ts", "ts_hls_idr_fallback")
    else {
        return;
    };
    assert_falls_back(
        "arte",
        &path,
        Via::Http,
        TsHlsIdrFallback::Auto,
        AUTO_BOUND_SECS,
        1,
    )
    .await;
}

#[tokio::test]
async fn zenith_tf1_falls_back_by_default() {
    let Some(path) = test_fixtures::require("zenith-live/tf-1-8MB.ts", "ts_hls_idr_fallback")
    else {
        return;
    };
    assert_falls_back(
        "tf1",
        &path,
        Via::Http,
        TsHlsIdrFallback::Auto,
        AUTO_BOUND_SECS,
        1,
    )
    .await;
}

#[tokio::test]
async fn zenith_tf1_live_capture_falls_back_after_a_short_bound() {
    let Some(path) = test_fixtures::require("zenith-http/tf1-live.ts", "ts_hls_idr_fallback")
    else {
        return;
    };
    let (fallback, bound) = short_bound();
    assert_falls_back("tf1live", &path, Via::Http, fallback, bound, MIN_SEGMENTS).await;
}

#[tokio::test]
async fn never_keeps_the_strict_behaviour_but_reports_it() {
    let name = "strict-gulli";
    let path = looped_ts("ts/gulli-opengop.ts", name, 0, GULLI_LOOP_REPS);
    let server = start(route(name, file_input(&path), TsHlsIdrFallback::Never));
    server.wait_for_idr_status(name, "awaiting").await;
    let metrics = server.metrics().await;
    assert_eq!(metric_value(&metrics, COUNTER, name, "awaiting"), Some(1.0));
    assert_eq!(metric_value(&metrics, COUNTER, name, "fell_back"), None);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let playlist = server.playlist(name).await;
    assert!(
        !playlist.contains("#EXTINF:"),
        "strict mode must serve no segment for an IDR-less stream: {playlist}"
    );
    let _ = std::fs::remove_file(&path);
}

fn looped_ts(rel: &str, tag: &str, skip: usize, reps: usize) -> PathBuf {
    let bytes = std::fs::read(test_fixtures::path(rel)).expect("fixture");
    let ir: Media = TsDemux::new().unpackage(&bytes).expect("demux fixture");
    let track = ir
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC");
    let body = &track.samples[skip..];
    let first = body[0].dts.expect("dts");
    let last = body.last().expect("sample");
    let span = last.dts.expect("dts") + i64::from(last.duration.expect("duration")) - first;
    let mut stream = Vec::new();
    for k in 0..reps as i64 {
        for s in body {
            stream.push(Sample::new(
                s.data.to_vec(),
                s.dts.map(|d| d + span * k),
                s.pts.map(|p| p + span * k),
                s.duration,
                s.flags.is_sync,
            ));
        }
    }
    let media = Media::new(vec![Track::new(track.spec, stream)], MOVIE_TIMESCALE);
    let ts = TsMux::new().package(&media).expect("mux");
    let path = std::env::temp_dir().join(format!(
        "multimux-idr-fallback-{tag}-{}.ts",
        std::process::id()
    ));
    std::fs::write(&path, ts).expect("write");
    path
}

#[tokio::test]
async fn idr_stream_never_reports_or_falls_back() {
    let name = "control-france2";
    let path = looped_ts(
        "ts/france2.ts",
        name,
        FRANCE2_FIRST_NON_IDR_SYNC,
        FRANCE2_LOOP_REPS,
    );
    let server = start(route(
        name,
        file_input(&path),
        TsHlsIdrFallback::Targets(SHORT_BOUND_TARGETS * 5),
    ));
    let uris = server.segments(name, 3).await;
    for uri in &uris {
        let seg = server
            .bytes(&format!("http://{}/{name}/{uri}", server.media))
            .await;
        let video = video_samples(&seg);
        assert!(
            access_unit_is_idr(NalCodec::Avc, &video[0].data),
            "{uri} must start on an IDR"
        );
    }
    assert_eq!(
        server.status(name).await["ts_hls_idr"],
        serde_json::Value::Null
    );
    let metrics = server.metrics().await;
    assert_eq!(metric_value(&metrics, COUNTER, name, "fell_back"), None);
    assert_eq!(metric_value(&metrics, COUNTER, name, "awaiting"), None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn config_option_parses_and_rejects_unknown_value() {
    let base = |extra: &str| {
        format!(
            r#"{{"name":"cam","input":{{"type":"file","path":"/x.ts"}},"outputs":["ts_hls"]{extra}}}"#
        )
    };
    let parse = |json: String| serde_json::from_str::<Route>(&json);
    assert_eq!(
        parse(base("")).expect("default").ts_hls_idr_fallback,
        TsHlsIdrFallback::Auto
    );
    assert_eq!(
        parse(base(r#","ts_hls_idr_fallback":"never""#))
            .expect("never")
            .ts_hls_idr_fallback,
        TsHlsIdrFallback::Never
    );
    assert_eq!(
        parse(base(r#","ts_hls_idr_fallback":{"targets":5}"#))
            .expect("targets")
            .ts_hls_idr_fallback,
        TsHlsIdrFallback::Targets(5)
    );
    for bad in [r#""bogus""#, r#""Never""#, r#"{"targets":"x"}"#] {
        parse(base(&format!(r#","ts_hls_idr_fallback":{bad}"#)))
            .expect_err("unknown value must be rejected");
    }
    assert_eq!(
        TsHlsIdrFallback::Auto.bound(4.0),
        Duration::from_secs_f64(12.0)
    );
    assert_eq!(
        TsHlsIdrFallback::Auto.bound(1.0),
        Duration::from_secs_f64(6.0)
    );
    assert_eq!(
        TsHlsIdrFallback::Targets(2).bound(1.0),
        Duration::from_secs_f64(2.0)
    );
}

const VALIDATOR_WINDOW_SECS: &str = "14";
const DURATION_MISMATCH_CODE: i64 = -50060;
const MIN_VALIDATED_SEGMENTS: usize = 4;

fn collect_codes(value: &serde_json::Value, out: &mut Vec<(i64, String)>) {
    match value {
        serde_json::Value::Object(map) => {
            if let (Some(code), Some(detail)) = (
                map.get("errorStatusCode")
                    .and_then(serde_json::Value::as_i64),
                map.get("errorDetail").and_then(serde_json::Value::as_str),
            ) {
                out.push((code, detail.to_owned()));
            }
            map.values().for_each(|v| collect_codes(v, out));
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| collect_codes(v, out)),
        _ => {}
    }
}

/// Apple's own validator against the live route: every segment's measured
/// duration must agree with its `EXTINF` (round-9 F2).
async fn assert_validator_finds_no_duration_mismatch(tag: &str, input: InputSpec) {
    let probe = std::process::Command::new("mediastreamvalidator")
        .arg("--version")
        .output();
    if probe.is_err() {
        eprintln!("{tag}: SKIPPED - mediastreamvalidator is not installed (macOS-only oracle)");
        return;
    }
    let name = format!("validated-{tag}");
    let server = start(route(
        &name,
        input,
        TsHlsIdrFallback::Targets(SHORT_BOUND_TARGETS),
    ));
    server.segments(&name, MIN_VALIDATED_SEGMENTS).await;
    let json_path =
        std::env::temp_dir().join(format!("multimux-msv-{name}-{}.json", std::process::id()));
    let url = format!("http://{}/{name}/media.m3u8", server.media);
    let out = json_path.clone();
    let run = tokio::task::spawn_blocking(move || {
        std::process::Command::new("mediastreamvalidator")
            .args(["--quiet", "-t", VALIDATOR_WINDOW_SECS, "-O"])
            .arg(&out)
            .arg(&url)
            .output()
    })
    .await
    .expect("join")
    .expect("run mediastreamvalidator");
    let text = std::fs::read_to_string(&json_path).unwrap_or_else(|e| {
        panic!(
            "{tag}: mediastreamvalidator wrote no JSON ({e}): {}",
            String::from_utf8_lossy(&run.stderr)
        )
    });
    let _ = std::fs::remove_file(&json_path);
    let report: serde_json::Value = serde_json::from_str(&text).expect("validator json");
    assert!(
        report["processedSegmentsCount"].as_i64().unwrap_or(0) >= 2,
        "{tag}: the validator processed no segments: {report}"
    );
    let mut findings = Vec::new();
    collect_codes(&report, &mut findings);
    let mismatches: Vec<_> = findings
        .iter()
        .filter(|(code, _)| *code == DURATION_MISMATCH_CODE)
        .collect();
    assert!(
        mismatches.is_empty(),
        "{tag}: playlist vs segment duration mismatch: {mismatches:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn validator_accepts_segment_durations_of_an_open_gop_capture() {
    let Some(path) = test_fixtures::require("zenith-http/tf1-live.ts", "ts_hls_idr_fallback")
    else {
        return;
    };
    assert_validator_finds_no_duration_mismatch("tf1", file_input(&path)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn validator_accepts_segment_durations_of_the_public_open_gop_capture() {
    let path = test_fixtures::path("ts/gulli-opengop.ts");
    assert_validator_finds_no_duration_mismatch("gulli", file_input(&path)).await;
}
