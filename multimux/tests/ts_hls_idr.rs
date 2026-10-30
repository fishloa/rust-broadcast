//! Apple HLS authoring conformance for the `ts_hls` output (issue #1425):
//! segments start on an IDR (spec 7.4) and HEVC/AV1 are refused (spec 1.5).
//!
//! The ingest is real: the committed `fixtures/ts/france2.ts` capture (real
//! H.264 whose GOPs open with an IDR at samples 0 and 48 and with non-IDR
//! sync samples at 20 and 65) is demuxed, looped with rebased timestamps,
//! re-muxed to a TS file and played through `InputSpec::File` into
//! `serve_with_registry_on`; every segment is fetched back over a real HTTP
//! `GET` and demuxed to check which picture it starts on.
#![cfg(feature = "test-seams")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use broadcast_common::{Package, Unpackage};
use multimux::config::{AdminSpec, Config, InputSpec, OutputAuthSpec, Route, TsHlsCutPolicy};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::{serve_with_registry_on, serve_with_registry_on_admin};
use transmux::media::{Media, Track};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{Fmp4Demux, NalCodec, TsDemux, TsMux, access_unit_is_idr};

#[path = "support/listener.rs"]
mod listener;
use listener::bind_tcp;

const ADMIN_TOKEN: &str = "ts-hls-idr-admin";
const FIRST_NON_IDR_SYNC: usize = 20;
const LOOP_REPS: usize = 4;
const MIN_CLOSED_SEGMENTS: usize = 3;
const HANG_GUARD: Duration = Duration::from_secs(40);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const MOVIE_TIMESCALE: u32 = 90_000;

fn france2_video() -> (TrackSpec, Vec<Sample>) {
    let bytes = std::fs::read(test_fixtures::path("ts/france2.ts")).expect("france2.ts fixture");
    let ir = TsDemux::new().unpackage(&bytes).expect("demux france2");
    let track = ir
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("france2 carries AVC video");
    (track.spec, track.samples)
}

fn looped_from(samples: &[Sample], start: usize, reps: usize) -> Vec<Sample> {
    let body = &samples[start..];
    let first = body[0].dts.expect("dts");
    let last = body.last().expect("sample");
    let span = last.dts.expect("dts") + i64::from(last.duration.expect("duration")) - first;
    let mut out = Vec::new();
    for k in 0..reps as i64 {
        for s in body {
            out.push(Sample::new(
                s.data.to_vec(),
                s.dts.map(|d| d + span * k),
                s.pts.map(|p| p + span * k),
                s.duration,
                s.flags.is_sync,
            ));
        }
    }
    out
}

fn write_ts(tag: &str) -> PathBuf {
    let (spec, samples) = france2_video();
    let stream = looped_from(&samples, FIRST_NON_IDR_SYNC, LOOP_REPS);
    let media = Media::new(vec![Track::new(spec, stream)], MOVIE_TIMESCALE);
    let ts = TsMux::new().package(&media).expect("mux looped stream");
    let path = std::env::temp_dir().join(format!(
        "multimux-ts-hls-idr-{tag}-{}.ts",
        std::process::id()
    ));
    std::fs::write(&path, ts).expect("write ts");
    path
}

fn route(path: &std::path::Path, outputs: Vec<OutputKind>, cut: TsHlsCutPolicy) -> Route {
    Route {
        name: "cam".to_string(),
        input: InputSpec::File {
            path: path.to_string_lossy().into_owned(),
            loop_file: false,
        },
        outputs,
        dvr: DvrConfig::default(),
        ts_hls_cut: cut,
        ts_hls_idr_fallback: Default::default(),
        declared_bandwidth_bps: None,
        audio_renditions: Vec::new(),
        audio_rendition_sources: Vec::new(),
        closed_caption_overrides: Vec::new(),
    }
}

fn config(bind: SocketAddr, route: Route) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 1.0,
        part_target_ms: 200,
        window_segments: 32,
        routes: vec![route],
        ..Config::default()
    }
}

async fn get_text(client: &reqwest::Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
}

async fn get_bytes(client: &reqwest::Client, url: &str) -> Vec<u8> {
    let resp = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET {url}");
    resp.bytes().await.expect("body").to_vec()
}

async fn playlist_with_segments(
    client: &reqwest::Client,
    base: &str,
    suffix: &str,
) -> (String, Vec<String>) {
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    loop {
        tick.tick().await;
        if let Some(body) = get_text(client, &format!("{base}/media.m3u8")).await {
            let uris: Vec<String> = body
                .lines()
                .filter(|l| !l.starts_with('#') && l.ends_with(suffix))
                .map(str::to_owned)
                .collect();
            if uris.len() >= MIN_CLOSED_SEGMENTS {
                return (body, uris);
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "fewer than {MIN_CLOSED_SEGMENTS} closed {suffix} segments within the hang guard"
        );
    }
}

fn ts_segment_starts_on_idr(bytes: &[u8]) -> bool {
    let ir = TsDemux::new()
        .unpackage(bytes)
        .expect("demux served segment");
    let video = ir
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("served segment carries AVC video");
    access_unit_is_idr(NalCodec::Avc, &video.samples[0].data)
}

async fn served_ts_segment_starts(cut: TsHlsCutPolicy, tag: &str) -> Vec<bool> {
    let path = write_ts(tag);
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        config(bind, route(&path, vec![OutputKind::TsHls], cut)),
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let base = format!("http://{bind}/cam");
    let (_, uris) = playlist_with_segments(&client, &base, ".ts").await;
    let mut starts = Vec::new();
    for uri in &uris {
        starts.push(ts_segment_starts_on_idr(
            &get_bytes(&client, &format!("{base}/{uri}")).await,
        ));
    }
    server.abort();
    let _ = std::fs::remove_file(&path);
    starts
}

#[tokio::test]
async fn ts_hls_default_cuts_on_idr_only() {
    let starts = served_ts_segment_starts(TsHlsCutPolicy::default(), "default").await;
    assert!(starts.len() >= MIN_CLOSED_SEGMENTS);
    assert!(
        starts.iter().all(|&idr| idr),
        "every default ts_hls segment must start on an IDR, got {starts:?}"
    );
}

#[tokio::test]
async fn ts_hls_option_any_rap_restores_old_behaviour() {
    let starts = served_ts_segment_starts(TsHlsCutPolicy::AnyRap, "anyrap").await;
    assert!(starts.len() >= MIN_CLOSED_SEGMENTS);
    assert!(
        starts.iter().any(|&idr| !idr),
        "any_rap must still cut on non-IDR random-access points, got {starts:?}"
    );
}

#[tokio::test]
async fn hevc_program_on_ts_hls_fails_with_typed_error() {
    let hevc = test_fixtures::path("ts/hevc/main.ts");
    let (media, media_listener) = bind_tcp();
    let (admin, admin_listener) = bind_tcp();
    let mut cfg = config(
        media,
        route(&hevc, vec![OutputKind::TsHls], TsHlsCutPolicy::default()),
    );
    cfg.admin = Some(AdminSpec {
        bind: admin.to_string(),
        auth: OutputAuthSpec::Bearer {
            token: ADMIN_TOKEN.to_string(),
        },
    });
    let server = tokio::spawn(serve_with_registry_on_admin(
        media_listener,
        admin_listener,
        cfg,
        SchemeRegistry::new(),
    ));

    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    let error = loop {
        tick.tick().await;
        if let Ok(resp) = client
            .get(format!("http://{admin}/admin/routes/cam"))
            .bearer_auth(ADMIN_TOKEN)
            .send()
            .await
            && resp.status().is_success()
            && let Ok(status) = resp.json::<serde_json::Value>().await
            && let Some(text) = status["output_error"].as_str()
        {
            break text.to_owned();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no output_error reported for an HEVC program on ts_hls"
        );
    };
    assert_eq!(
        error,
        "ts_hls output cannot carry HEVC: HEVC is not allowed in MPEG-TS HLS segments"
    );

    let playlist = get_text(&client, &format!("http://{media}/cam/media.m3u8"))
        .await
        .unwrap_or_default();
    assert!(
        !playlist.contains("#EXTINF:"),
        "a failed ts_hls output must not publish segments: {playlist}"
    );
    server.abort();
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
        parse(base("")).expect("default").ts_hls_cut,
        TsHlsCutPolicy::IdrOnly
    );
    assert_eq!(
        parse(base(r#","ts_hls_cut":"idr_only""#))
            .expect("idr_only")
            .ts_hls_cut,
        TsHlsCutPolicy::IdrOnly
    );
    assert_eq!(
        parse(base(r#","ts_hls_cut":"any_rap""#))
            .expect("any_rap")
            .ts_hls_cut,
        TsHlsCutPolicy::AnyRap
    );
    for bad in ["bogus", "IdrOnly", "any-rap", ""] {
        let err = parse(base(&format!(r#","ts_hls_cut":"{bad}""#)))
            .expect_err("unknown value must be rejected");
        assert!(
            err.to_string().contains("unknown variant"),
            "rejection for {bad:?} must name the bad variant: {err}"
        );
    }
    assert_eq!(TsHlsCutPolicy::IdrOnly.name(), "idr_only");
    assert_eq!(TsHlsCutPolicy::AnyRap.to_string(), "any_rap");
}

async fn fmp4_first_segment(cut: TsHlsCutPolicy, tag: &str) -> (Vec<u8>, Vec<u8>) {
    let path = write_ts(tag);
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        config(bind, route(&path, vec![OutputKind::LlHls], cut)),
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let base = format!("http://{bind}/cam");
    let (body, uris) = playlist_with_segments(&client, &base, ".m4s").await;
    let init_uri = body
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
        .and_then(|r| r.split('"').next())
        .expect("playlist names an init segment")
        .to_owned();
    let init = get_bytes(&client, &format!("{base}/{init_uri}")).await;
    let segment = get_bytes(&client, &format!("{base}/{}", uris[0])).await;
    server.abort();
    let _ = std::fs::remove_file(&path);
    (init, segment)
}

#[tokio::test]
async fn fmp4_outputs_unchanged_by_policy() {
    let (init_idr, seg_idr) = fmp4_first_segment(TsHlsCutPolicy::IdrOnly, "fmp4-idr").await;
    let (init_any, seg_any) = fmp4_first_segment(TsHlsCutPolicy::AnyRap, "fmp4-any").await;
    assert_eq!(
        init_idr, init_any,
        "fMP4 init must not depend on ts_hls_cut"
    );
    assert_eq!(
        seg_idr, seg_any,
        "fMP4 segment must not depend on ts_hls_cut"
    );

    let mut whole = init_idr;
    whole.extend_from_slice(&seg_idr);
    let ir = Fmp4Demux::new()
        .unpackage(&whole)
        .expect("demux fMP4 segment");
    let video = ir
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("fMP4 segment carries AVC video");
    assert!(
        !access_unit_is_idr(NalCodec::Avc, &video.samples[0].data),
        "fMP4 still opens on the first random-access point, which here is a non-IDR sync sample"
    );
}
