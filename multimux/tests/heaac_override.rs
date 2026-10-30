//! Per-route `audio_profiles` HE-AAC override over a real ADTS-in-TS HE-AAC
//! stream (issue #1428): the fMP4 init carries explicit SBR/PS signalling and
//! the master playlist `CODECS` says `mp4a.40.5` / `mp4a.40.29`.
//!
//! Real artifacts: `fixtures/ts/heaac/heaac_v{1,2}_adts.ts` (macOS AudioToolbox
//! encoder; commands in `fixtures/ts/GENERATE.md`). Every test drives the
//! production path: `RouteHandle::set_track_specs` + `ProgramSegmenter::pump`
//! over a `Trunk`, read back through the axum router as a player would.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_common::Parse;
use broadcast_common::mux::Unpackage;
use broadcast_hls::{MasterPlaylist, MediaPlaylist};
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use tower::ServiceExt;
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{MovieBox, MovieFragmentBox};
use transmux::ts_demux::TsDemux;

use multimux::config::{AudioProfile, AudioProfileOverride, AudioTrackSelector, Config};
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
const V1_FIXTURE: &str = "ts/heaac/heaac_v1_adts.ts";
const V2_FIXTURE: &str = "ts/heaac/heaac_v2_adts.ts";
const V1_ASC: [u8; 4] = [0x2B, 0x92, 0x08, 0x00];
const OUTPUT_HZ: u32 = 44_100;
const AAC_FRAME_OUTPUT_TICKS: u32 = 2048;
const BOX_HEADER_LEN: usize = 8;
const HANDLER_SOUND: [u8; 4] = *b"soun";
const V2_ASC: [u8; 4] = [0xEB, 0x8A, 0x08, 0x00];

struct Source {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

fn load(fixture: &str) -> Vec<Source> {
    let bytes = test_fixtures::read(fixture);
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

fn route_app(fixture: &str, profiles: Vec<AudioProfileOverride>) -> Router {
    let sources = load(fixture);
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

async fn master_codecs(app: &Router) -> String {
    let text = String::from_utf8(ok(app, "master.m3u8").await).expect("utf8");
    let master =
        MasterPlaylist::parse(&text).unwrap_or_else(|e| panic!("master must parse: {e}\n{text}"));
    master
        .variants
        .first()
        .and_then(|v| v.codecs.clone())
        .unwrap_or_else(|| panic!("variant carries CODECS:\n{text}"))
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

fn declared(profile: AudioProfile) -> Vec<AudioProfileOverride> {
    vec![AudioProfileOverride {
        track: AudioTrackSelector::Index(0),
        audio_profile: profile,
    }]
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test]
async fn heaac_override_route_master_codecs_say_mp4a_40_5() {
    let app = route_app(V1_FIXTURE, declared(AudioProfile::HeAac));
    let codecs = master_codecs(&app).await;
    assert!(codecs.contains("mp4a.40.5"), "CODECS was {codecs}");
    assert!(!codecs.contains("mp4a.40.2"), "CODECS was {codecs}");
    let init = init_bytes(&app).await;
    assert!(
        contains(&init, &V1_ASC),
        "init carries the explicit SBR ASC"
    );
}

#[tokio::test]
async fn heaac_v2_override_route_master_codecs_say_mp4a_40_29() {
    let app = route_app(V2_FIXTURE, declared(AudioProfile::HeAacV2));
    let codecs = master_codecs(&app).await;
    assert!(codecs.contains("mp4a.40.29"), "CODECS was {codecs}");
    assert!(contains(&init_bytes(&app).await, &V2_ASC));
}

#[tokio::test]
async fn route_without_audio_profiles_keeps_implicit_aac_lc() {
    let app = route_app(V1_FIXTURE, Vec::new());
    let codecs = master_codecs(&app).await;
    assert!(codecs.contains("mp4a.40.2"), "CODECS was {codecs}");
    let init = init_bytes(&app).await;
    assert!(!contains(&init, &V1_ASC));
}

#[tokio::test]
async fn override_for_a_missing_audio_track_changes_nothing() {
    let app = route_app(
        V1_FIXTURE,
        vec![AudioProfileOverride {
            track: AudioTrackSelector::Index(7),
            audio_profile: AudioProfile::HeAac,
        }],
    );
    assert!(master_codecs(&app).await.contains("mp4a.40.2"));
}

#[test]
fn audio_profiles_config_parses_and_rejects_duplicates() {
    let config = |profiles: &str| {
        let json = format!(
            r#"{{"routes":[{{"name":"cam","input":{{"type":"rtsp","url":"rtsp://host/s1"}},
            "outputs":["llhls"],"audio_profiles":{profiles}}}]}}"#
        );
        serde_json::from_str::<Config>(&json).expect("config parses")
    };
    let cfg = config(
        r#"[{"track":{"index":0},"audio_profile":"he-aac"},
            {"track":{"track_id":258},"audio_profile":"he-aac-v2"}]"#,
    );
    cfg.validate().expect("valid");
    assert_eq!(
        cfg.routes[0].audio_profiles,
        vec![
            AudioProfileOverride {
                track: AudioTrackSelector::Index(0),
                audio_profile: AudioProfile::HeAac
            },
            AudioProfileOverride {
                track: AudioTrackSelector::TrackId(258),
                audio_profile: AudioProfile::HeAacV2
            },
        ]
    );
    assert!(
        config(
            r#"[{"track":{"index":0},"audio_profile":"he-aac"},
                {"track":{"index":0},"audio_profile":"he-aac-v2"}]"#
        )
        .validate()
        .is_err()
    );
    let none = serde_json::from_str::<Config>(
        r#"{"routes":[{"name":"cam","input":{"type":"rtsp","url":"rtsp://host/s1"}}]}"#,
    )
    .expect("config parses");
    assert!(none.routes[0].audio_profiles.is_empty());
    assert_eq!(AudioProfile::HeAac.to_string(), "he-aac");
    assert_eq!(AudioProfile::HeAacV2.to_string(), "he-aac-v2");
}

type Finding = (String, i64, String);

fn collect_findings(value: &serde_json::Value, path: &str, out: &mut Vec<Finding>) {
    match value {
        serde_json::Value::Object(map) => {
            let parse_failed = map
                .get("parseFailed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(serde_json::Value::Array(messages)) = map.get("messages") {
                for m in messages {
                    let level = m
                        .get("errorRequirementLevel")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    out.push((
                        path.to_string(),
                        if parse_failed { 1 } else { level },
                        m.get("errorComment")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ));
                }
            }
            for (k, v) in map {
                if k != "messages" {
                    collect_findings(v, &format!("{path}/{k}"), out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect_findings(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn validator_available() -> bool {
    std::process::Command::new("mediastreamvalidator")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn validator_findings(app: &Router, tag: &str) -> Vec<Finding> {
    use std::fs;
    use std::process::Command;
    use std::time::Duration;

    let scratch = std::env::temp_dir().join(format!(
        "heaac-override-validator-{}-{tag}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("scratch");
    let mut uris = std::collections::BTreeSet::new();
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let body = ok(app, name).await;
        fs::write(scratch.join(name), &body).expect("write playlist");
        if name == "master.m3u8" {
            continue;
        }
        for line in String::from_utf8(body).expect("utf8").lines() {
            if let Some(start) = line.find("URI=\"") {
                let rest = &line[start + 5..];
                if let Some(end) = rest.find('"') {
                    uris.insert(rest[..end].to_string());
                }
            }
            let t = line.trim();
            if !t.is_empty() && !t.starts_with('#') {
                uris.insert(t.to_string());
            }
        }
    }
    for uri in &uris {
        let (status, body) = get(app, uri).await;
        if status == StatusCode::OK {
            fs::write(scratch.join(uri), body).expect("write resource");
        }
    }
    let report = scratch.join("report.json");
    let mut cmd = Command::new("mediastreamvalidator");
    cmd.current_dir(&scratch)
        .arg("--quiet")
        .arg("-t")
        .arg("5")
        .arg("-O")
        .arg(&report)
        .arg("master.m3u8");
    let _ = test_bounded::output_bounded(&mut cmd, Duration::from_secs(60)).expect("run validator");
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).expect("report")).expect("json");
    let mut findings = Vec::new();
    collect_findings(&json, "$", &mut findings);
    for f in &findings {
        eprintln!("VALIDATOR[{tag}]: {} level={} {}", f.0, f.1, f.2);
    }
    let _ = fs::remove_dir_all(&scratch);
    findings
}

fn names_codec_or_sbr(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("sbr") || m.contains("implicit") || m.contains("codecs") || m.contains("mp4a")
}

#[tokio::test]
async fn heaac_override_validator_reports_no_sbr_or_codecs_finding() {
    if !validator_available() {
        eprintln!(
            "SKIP heaac_override_validator_reports_no_sbr_or_codecs_finding: \
             `mediastreamvalidator` not on PATH"
        );
        return;
    }
    let overridden = route_app(V1_FIXTURE, declared(AudioProfile::HeAac));
    let findings = validator_findings(&overridden, "override").await;
    let baseline = route_app(V1_FIXTURE, Vec::new());
    let baseline_findings = validator_findings(&baseline, "implicit").await;
    let audio_findings: Vec<_> = findings
        .iter()
        .filter(|f| names_codec_or_sbr(&f.2))
        .collect();
    assert!(
        audio_findings.is_empty(),
        "validator flags SBR/CODECS on the explicit route: {audio_findings:?}"
    );
    let new_must: Vec<_> = findings
        .iter()
        .filter(|f| f.1 <= 1 && !baseline_findings.iter().any(|b| b.2 == f.2))
        .collect();
    assert!(
        new_must.is_empty(),
        "level-1 findings the override adds over the implicit route: {new_must:?}"
    );
}

fn top_level_boxes(bytes: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + BOX_HEADER_LEN <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 bytes")) as usize;
        assert!(size >= BOX_HEADER_LEN && at + size <= bytes.len(), "box size");
        let four_cc: [u8; 4] = bytes[at + 4..at + 8].try_into().expect("4 bytes");
        out.push((four_cc, &bytes[at..at + size]));
        at += size;
    }
    out
}

#[tokio::test]
async fn heaac_route_init_mdhd_uses_output_rate() {
    let app = route_app(V1_FIXTURE, declared(AudioProfile::HeAac));
    let init = init_bytes(&app).await;
    let moov_bytes = top_level_boxes(&init)
        .into_iter()
        .find(|(cc, _)| cc == b"moov")
        .expect("moov")
        .1;
    let moov = MovieBox::parse(moov_bytes).expect("moov parses");
    let audio = moov
        .tracks
        .iter()
        .find(|t| {
            t.mdia
                .as_ref()
                .and_then(|m| m.hdlr.as_ref())
                .is_some_and(|h| h.handler_type == HANDLER_SOUND)
        })
        .expect("audio trak");
    let mdhd = audio
        .mdia
        .as_ref()
        .and_then(|m| m.mdhd.as_ref())
        .expect("mdhd");
    assert_eq!(mdhd.timescale, OUTPUT_HZ);
    let audio_id = audio.tkhd.track_id;

    let text = String::from_utf8(ok(&app, "media.m3u8").await).expect("utf8");
    let playlist = MediaPlaylist::parse(&text).expect("media playlist parses");
    let segment = ok(&app, &playlist.segments[0].uri).await;
    let trex_default = moov
        .mvex
        .as_ref()
        .and_then(|m| m.trex.iter().find(|t| t.track_id == audio_id))
        .map(|t| t.default_sample_duration);
    let mut audio_samples = 0usize;
    for (_, moof_bytes) in top_level_boxes(&segment)
        .into_iter()
        .filter(|(cc, _)| cc == b"moof")
    {
        let moof = MovieFragmentBox::parse_body(&moof_bytes[BOX_HEADER_LEN..]).expect("moof");
        for traf in moof.traf.iter().filter(|t| t.tfhd.track_id == audio_id) {
            for run in &traf.trun {
                for sample in &run.samples {
                    let duration = sample
                        .sample_duration
                        .or(traf.tfhd.default_sample_duration)
                        .or(trex_default);
                    assert_eq!(duration, Some(AAC_FRAME_OUTPUT_TICKS));
                    audio_samples += 1;
                }
            }
        }
    }
    assert!(audio_samples > 0, "first media segment carries audio samples");
}
