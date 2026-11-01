//! Closed-caption signalling in the master playlist of a route whose video
//! carries CEA-608/708 (issue #1428), driven through the production
//! `ProgramSegmenter::pump` path and read back over the axum router.

use std::collections::{BTreeSet, HashMap};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_common::mux::Unpackage;
use broadcast_hls::{ClosedCaptionsRef, InstreamId, MasterPlaylist, MediaType};
use hls_runtime::server::ClosedCaptionDecl;
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use tower::ServiceExt;
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
const SIDE_RINGS: usize = 64;
const PUMP_EVERY: usize = 64;
const DECLARED_BPS: u64 = 16_000_000;
const STREAM: &str = "cam";
const UNCAPTIONED_LONG: &str = "ts/h264_aac_40s.ts";
const UNCAPTIONED_SHORT: &str = "ts/h264_aac.ts";
const TWO_FIELD_CAPTURE: &str = "captions/mux.js/multi-channel-608-captions.ts";
const SHAKA_CAPTURE: &str = "captions/shaka/captions-test.ts";
const CAPTIONED: &str = "ts/transformers-eia608-h264.ts";
const VALIDATOR_BIN: &str = "/usr/local/bin/mediastreamvalidator";
const VALIDATOR_DEADLINE: Duration = Duration::from_secs(60);
const VALIDATOR_TARGET_SECS: &str = "5";
const MUST_LEVEL: i64 = 1;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

struct CaptionedRoute {
    route: Arc<RouteHandle>,
    app: Router,
}

fn build(fixture: &str, overrides: Vec<ClosedCaptionDecl>, who: &str) -> Option<CaptionedRoute> {
    let bytes = test_fixtures::require_bytes(fixture, who)?;
    let media = TsDemux::new().unpackage(&bytes).expect("demux fixture");
    let trunk = Trunk::new(TrunkConfig::new(
        nz(RING),
        nz(SIDE_RINGS),
        nz(WINDOW_SEGMENTS),
        nz(SIDE_RINGS),
        nz(RING),
    ));
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(
        media
            .tracks
            .iter()
            .filter(|t| t.spec.config.is_muxable_in_bmff())
            .map(|t| t.spec.clone())
            .collect(),
    );
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(DECLARED_BPS)
            .with_closed_caption_overrides(overrides),
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
    for (ti, track) in media.tracks.iter().enumerate() {
        if !track.spec.config.is_muxable_in_bmff() {
            continue;
        }
        for (si, s) in track.samples.iter().enumerate() {
            let dts = s.dts.or(s.pts).unwrap_or(0);
            order.push((dts as f64 / f64::from(track.spec.timescale), ti, si));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (n, (_, ti, si)) in order.into_iter().enumerate() {
        let track = &media.tracks[ti];
        let mut sample = track.samples[si].clone();
        sample.dts = sample.dts.or(sample.pts);
        writer.publish(track.spec.track_id, RetentionClass::Timed, sample);
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
            Arc::clone(&route),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    Some(CaptionedRoute { route, app })
}

impl CaptionedRoute {
    async fn get(&self, path: &str) -> (StatusCode, Vec<u8>) {
        let resp = self
            .app
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

    async fn master_text(&self) -> String {
        let (status, body) = self.get("master.m3u8").await;
        assert_eq!(status, StatusCode::OK, "GET master.m3u8");
        String::from_utf8(body).expect("utf8")
    }
}

fn caption_renditions(master: &MasterPlaylist) -> Vec<&broadcast_hls::MediaRendition> {
    master
        .media
        .iter()
        .filter(|m| m.media_type == MediaType::ClosedCaptions)
        .collect()
}

#[tokio::test]
async fn master_lists_closed_captions_rendition_when_video_carries_608() {
    let Some(rig) = build(
        CAPTIONED,
        Vec::new(),
        "master_lists_closed_captions_rendition_when_video_carries_608",
    ) else {
        return;
    };
    let text = rig.master_text().await;
    let master = MasterPlaylist::parse(&text).expect("master parses");

    let captions = caption_renditions(&master);
    let cc1 = captions
        .iter()
        .find(|m| m.instream_id == Some(InstreamId::Cc(1)))
        .unwrap_or_else(|| panic!("no CC1 rendition in:\n{text}"));
    let groups: BTreeSet<&str> = captions.iter().map(|m| m.group_id.as_str()).collect();
    assert_eq!(groups.len(), 1, "one GROUP-ID for all captions:\n{text}");
    let names: BTreeSet<&str> = captions.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names.len(), captions.len(), "NAME unique in group:\n{text}");
    assert!(cc1.uri.is_none(), "closed captions carry no URI");

    assert!(!master.variants.is_empty());
    for variant in &master.variants {
        assert_eq!(
            variant.closed_captions,
            Some(ClosedCaptionsRef::Group(cc1.group_id.clone())),
            "every STREAM-INF references the caption group:\n{text}"
        );
    }
    assert_eq!(
        master.to_m3u8().expect("re-serialise"),
        text,
        "parse then serialise reproduces the served bytes"
    );
}

#[tokio::test]
async fn configured_caption_name_and_language_win() {
    let overrides = vec![
        ClosedCaptionDecl {
            instream_id: "CC1".into(),
            name: "English".into(),
            language: "en".into(),
        },
        ClosedCaptionDecl {
            instream_id: "CC4".into(),
            name: "Never detected".into(),
            language: "de".into(),
        },
    ];
    let Some(rig) = build(
        CAPTIONED,
        overrides,
        "configured_caption_name_and_language_win",
    ) else {
        return;
    };
    let master = MasterPlaylist::parse(&rig.master_text().await).expect("master parses");
    let captions = caption_renditions(&master);

    let cc1 = captions
        .iter()
        .find(|m| m.instream_id == Some(InstreamId::Cc(1)))
        .expect("CC1 advertised");
    assert_eq!(cc1.name, "English");
    assert_eq!(cc1.language.as_deref(), Some("en"));

    let service1 = captions
        .iter()
        .find(|m| m.instream_id == Some(InstreamId::Service(1)))
        .expect("SERVICE1 advertised");
    assert_eq!(service1.name, "SERVICE1");
    assert_eq!(service1.language, None);

    assert!(
        captions
            .iter()
            .all(|m| m.instream_id != Some(InstreamId::Cc(4))),
        "an override never advertises a stream the video does not carry"
    );
}

#[tokio::test]
async fn route_scanned_without_captions_declares_none() {
    let Some(rig) = build(
        CAPTIONED,
        Vec::new(),
        "route_scanned_without_captions_declares_none",
    ) else {
        return;
    };
    assert!(rig.route.note_closed_captions(ProgramId(0), Vec::new()));
    let text = rig.master_text().await;
    let master = MasterPlaylist::parse(&text).expect("master parses");
    assert!(caption_renditions(&master).is_empty());
    assert!(!master.variants.is_empty());
    assert!(
        master
            .variants
            .iter()
            .all(|v| v.closed_captions == Some(ClosedCaptionsRef::None)),
        "{text}"
    );
}

#[tokio::test]
async fn long_caption_free_stream_declares_none_after_the_scan_window() {
    let Some(rig) = build(
        UNCAPTIONED_LONG,
        Vec::new(),
        "long_caption_free_stream_declares_none_after_the_scan_window",
    ) else {
        return;
    };
    let text = rig.master_text().await;
    let master = MasterPlaylist::parse(&text).expect("master parses");
    assert!(caption_renditions(&master).is_empty(), "{text}");
    assert!(!master.variants.is_empty());
    assert!(
        master
            .variants
            .iter()
            .all(|v| v.closed_captions == Some(ClosedCaptionsRef::None)),
        "{text}"
    );
    assert_eq!(
        text.matches("CLOSED-CAPTIONS=NONE").count(),
        master.variants.len()
    );
}

#[tokio::test]
async fn short_caption_free_stream_claims_nothing_before_the_scan_window_ends() {
    let Some(rig) = build(
        UNCAPTIONED_SHORT,
        Vec::new(),
        "short_caption_free_stream_claims_nothing_before_the_scan_window_ends",
    ) else {
        return;
    };
    let text = rig.master_text().await;
    assert!(!text.contains("CLOSED-CAPTIONS"), "{text}");
}

async fn declared_instream_ids(fixture: &str, test: &str) -> Option<(String, Vec<InstreamId>)> {
    let rig = build(fixture, Vec::new(), test)?;
    let text = rig.master_text().await;
    let master = MasterPlaylist::parse(&text).expect("master parses");
    let ids = caption_renditions(&master)
        .iter()
        .map(|m| m.instream_id.clone().expect("INSTREAM-ID"))
        .collect();
    Some((text, ids))
}

#[tokio::test]
async fn two_field_capture_declares_cc1_and_cc3() {
    let Some((text, ids)) =
        declared_instream_ids(TWO_FIELD_CAPTURE, "two_field_capture_declares_cc1_and_cc3").await
    else {
        return;
    };
    assert_eq!(ids, [InstreamId::Cc(1), InstreamId::Cc(3)], "{text}");
    assert_eq!(
        text.matches("CLOSED-CAPTIONS=\"cc\"").count(),
        MasterPlaylist::parse(&text).expect("parse").variants.len()
    );
    assert!(!text.contains("CLOSED-CAPTIONS=NONE"));
}

#[tokio::test]
async fn shaka_capture_declares_cc1_and_service1() {
    let Some((text, ids)) =
        declared_instream_ids(SHAKA_CAPTURE, "shaka_capture_declares_cc1_and_service1").await
    else {
        return;
    };
    assert_eq!(ids, [InstreamId::Cc(1), InstreamId::Service(1)], "{text}");
}

#[test]
fn invalid_caption_override_is_rejected_by_config_validation() {
    let bad = ClosedCaptionDecl {
        instream_id: "CC5".into(),
        ..ClosedCaptionDecl::default()
    };
    assert!(hls_runtime::server::validate_closed_caption_overrides(&[bad]).is_err());
    let one = ClosedCaptionDecl {
        instream_id: "SERVICE3".into(),
        ..ClosedCaptionDecl::default()
    };
    assert!(
        hls_runtime::server::validate_closed_caption_overrides(&[one.clone(), one]).is_err(),
        "duplicate INSTREAM-ID"
    );
}

fn collect_findings(v: &serde_json::Value, path: &str, out: &mut Vec<(i64, String)>) {
    match v {
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
                    let comment = m.get("errorComment").and_then(|v| v.as_str()).unwrap_or("");
                    out.push((
                        if parse_failed { MUST_LEVEL } else { level },
                        format!("{path}: {comment}"),
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

async fn validator_findings(rig: &CaptionedRoute, tag: &str) -> Vec<(i64, String)> {
    use std::fs;
    use std::process::Command;

    let scratch = std::env::temp_dir().join(format!(
        "closed-captions-validator-{}-{tag}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("scratch");

    let mut uris = BTreeSet::new();
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let (status, body) = rig.get(name).await;
        assert_eq!(status, StatusCode::OK, "GET {name}");
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
        let (status, body) = rig.get(uri).await;
        if status == StatusCode::OK {
            fs::write(scratch.join(uri), body).expect("write resource");
        }
    }

    let report = scratch.join("report.json");
    let mut cmd = Command::new(VALIDATOR_BIN);
    cmd.current_dir(&scratch)
        .arg("--quiet")
        .arg("-t")
        .arg(VALIDATOR_TARGET_SECS)
        .arg("-O")
        .arg(&report)
        .arg("master.m3u8");
    let _ = test_bounded::output_bounded(&mut cmd, VALIDATOR_DEADLINE).expect("run validator");
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).expect("report")).expect("json");
    let _ = fs::remove_dir_all(&scratch);
    let mut findings = Vec::new();
    collect_findings(&json, "$", &mut findings);
    findings
}

#[tokio::test]
async fn apple_validator_reports_nothing_new_about_captions() {
    if !std::path::Path::new(VALIDATOR_BIN).exists() {
        eprintln!(
            "SKIP apple_validator_reports_nothing_new_about_captions: {VALIDATOR_BIN} absent"
        );
        return;
    }
    let Some(on) = build(
        CAPTIONED,
        Vec::new(),
        "apple_validator_reports_nothing_new_about_captions",
    ) else {
        return;
    };
    let Some(off) = build(
        CAPTIONED,
        Vec::new(),
        "apple_validator_reports_nothing_new_about_captions",
    ) else {
        return;
    };
    assert!(
        on.master_text().await.contains("TYPE=CLOSED-CAPTIONS"),
        "the captioned route must advertise captions, or the comparison is vacuous"
    );
    assert!(off.route.note_closed_captions(ProgramId(0), Vec::new()));

    let with_captions = validator_findings(&on, "on").await;
    let without_captions = validator_findings(&off, "off").await;
    let baseline: BTreeSet<&(i64, String)> = without_captions.iter().collect();
    let new: Vec<&(i64, String)> = with_captions
        .iter()
        .filter(|f| !baseline.contains(f))
        .collect();
    for (level, message) in &with_captions {
        eprintln!("VALIDATOR[on] level={level} {message}");
    }
    for (level, message) in &without_captions {
        eprintln!("VALIDATOR[off] level={level} {message}");
    }
    for (level, message) in &new {
        eprintln!("VALIDATOR[new] level={level} {message}");
    }
    assert!(
        new.is_empty(),
        "captions added validator findings the caption-free route lacks: {new:?}"
    );
    for (_, message) in with_captions.iter().chain(&without_captions) {
        let lower = message.to_lowercase();
        assert!(
            !lower.contains("caption") && !lower.contains("instream"),
            "validator finding about captions: {message}"
        );
    }
}
