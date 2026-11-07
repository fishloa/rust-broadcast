//! Per-route `audio_profiles` HE-AAC override over a real ADTS-in-TS HE-AAC
//! stream (issue #1428): the fMP4 init carries explicit SBR/PS signalling and
//! the master playlist `CODECS` says `mp4a.40.5` / `mp4a.40.29`.
//!
//! Real artifacts: `fixtures/ts/heaac/heaac-v{1,2}-adts.ts` (macOS AudioToolbox
//! encoder; commands in `fixtures/ts/PROVENANCE.md`). Every test drives the
//! production path: `RouteHandle::set_track_specs` + `ProgramSegmenter::pump`
//! over a `Trunk`, read back through the axum router as a player would.

use crate::support::hdr_route::{contains, get, init_bytes, load, ok, route_app_with_profiles};

use axum::Router;
use axum::http::StatusCode;
use broadcast_common::Parse;
use broadcast_hls::{MasterPlaylist, MediaPlaylist};
use transmux::{MovieBox, MovieFragmentBox};

use multimux::config::{AudioProfile, AudioProfileOverride, AudioTrackSelector, Config};

const V1_FIXTURE: &str = "ts/heaac/heaac-v1-adts.ts";
const V2_FIXTURE: &str = "ts/heaac/heaac-v2-adts.ts";
const V1_ASC: [u8; 4] = [0x2B, 0x92, 0x08, 0x00];
const OUTPUT_HZ: u32 = 44_100;
const AAC_FRAME_OUTPUT_TICKS: u32 = 2048;
const BOX_HEADER_LEN: usize = 8;
const HANDLER_SOUND: [u8; 4] = *b"soun";
const V2_ASC: [u8; 4] = [0xEB, 0x8A, 0x08, 0x00];

fn route_app(fixture: &str, profiles: Vec<AudioProfileOverride>) -> Router {
    route_app_with_profiles(load(fixture), profiles)
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

fn declared(profile: AudioProfile) -> Vec<AudioProfileOverride> {
    vec![AudioProfileOverride {
        track: AudioTrackSelector::Index(0),
        audio_profile: profile,
    }]
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
    assert!(matches!(
        config(
            r#"[{"track":{"index":0},"audio_profile":"he-aac"},
                {"track":{"index":0},"audio_profile":"he-aac-v2"}]"#
        )
        .validate(),
        Err(multimux::error::MultimuxError::ConfigInvalid { .. })
    ));
    let none = serde_json::from_str::<Config>(
        r#"{"routes":[{"name":"cam","input":{"type":"rtsp","url":"rtsp://host/s1"}}]}"#,
    )
    .expect("config parses");
    assert!(none.routes[0].audio_profiles.is_empty());
    assert_eq!(AudioProfile::HeAac.to_string(), "he-aac");
    assert_eq!(AudioProfile::HeAacV2.to_string(), "he-aac-v2");
}

async fn validator_findings(app: &Router, tag: &str) -> Vec<test_fixtures::process::msv::Finding> {
    use std::fs;
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
    let json =
        test_fixtures::process::msv::run_report(&scratch, "master.m3u8", Duration::from_secs(60));
    let findings = test_fixtures::process::msv::findings(&json);
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
    if !test_fixtures::process::msv::available() {
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
        assert!(
            size >= BOX_HEADER_LEN && at + size <= bytes.len(),
            "box size"
        );
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
    assert!(
        audio_samples > 0,
        "first media segment carries audio samples"
    );
}
