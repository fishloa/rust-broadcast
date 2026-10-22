//! End-to-end coverage of `#EXT-X-PROGRAM-DATE-TIME` on the multimux
//! LL-HLS/TS-HLS origin (issue #1423 part B2b, Apple authoring 8.4 MUST).
//!
//! Feeding is done through `RouteHandle::with_program_date_time(...)` rather
//! than the `Config` path: the detailed assertions below need a controlled
//! segment timeline, and `feed_via_segmenter` is the same helper the
//! `origin_iframe`/`origin_llhls` suites use. The `Config` -> `RouteHandle`
//! wiring itself is exercised by `multimux`'s own `origin` module tests and
//! by the mutation checks in the issue.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use hls_runtime::server::Container;
use multimux::config::ProgramDateTimeSource;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::output::ts_hls::TsHlsOutput;
use multimux::route::RouteHandle;
use transmux::TrackId;
use transmux::avc_config_from_sprop;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 500;

fn video_track_spec() -> TrackSpec {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        TrackId::new(1),
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 0,
            height: 0,
        },
    )
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("well-formed GET request")
}

fn feed_via_segmenter(
    store: &RouteHandle,
    specs: Vec<TrackSpec>,
    batches: Vec<Vec<(u32, Sample)>>,
) {
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);

    let mut seg = LlHlsSegmenter::with_part_target(
        specs,
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init segment builds"));

    for batch in batches {
        for (track_id, sample) in batch {
            seg.push(TrackId::new(track_id), sample)
                .expect("push succeeds");
        }
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add_segment");
        }
    }
    // Leave the in-progress segment unflushed so an open segment (with a
    // preload hint) is present, exercising the open-segment PDT path.
}

fn video_batches_n(n: u32) -> Vec<Vec<(u32, Sample)>> {
    let mut batches = Vec::new();
    for i in 0..n {
        let is_sync = i % 30 == 0;
        let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 64];
        let sample = Sample::new(
            data,
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    batches
}

fn pdt_lines(playlist: &str) -> Vec<i64> {
    playlist
        .lines()
        .filter_map(|l| l.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:"))
        .map(|v| {
            v.trim()
                .parse::<jiff::Timestamp>()
                .unwrap()
                .as_millisecond()
        })
        .collect()
}

#[tokio::test]
async fn arrival_pdt_on_media_and_iframe() {
    let clock_before_ms = jiff::Timestamp::now().as_millisecond();
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_program_date_time(ProgramDateTimeSource::Arrival),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches_n(120));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let media = body_string(app.clone().oneshot(get("/cam/media.m3u8")).await.unwrap()).await;
    let iframe = body_string(app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap()).await;

    let media_pdts = pdt_lines(&media);
    let iframe_pdts = pdt_lines(&iframe);
    assert!(media_pdts.len() >= 3, "media pdts: {media_pdts:?}\n{media}");
    assert!(
        iframe_pdts.len() >= 3,
        "iframe pdts: {iframe_pdts:?}\n{iframe}"
    );

    // Strictly increasing by the segment duration (1 s) for closed segments.
    for w in media_pdts.windows(2) {
        assert_eq!(
            w[1] - w[0],
            1000,
            "media PDTs must advance by 1 s: {media_pdts:?}"
        );
    }
    for w in iframe_pdts.windows(2) {
        assert_eq!(
            w[1] - w[0],
            1000,
            "iframe PDTs must advance by 1 s: {iframe_pdts:?}"
        );
    }
    // Media and I-frame PDTs equal per segment.
    for (a, b) in media_pdts.iter().zip(iframe_pdts.iter()) {
        assert_eq!(a, b, "media/iframe PDT mismatch");
    }
    // Open segment: its PDT precedes its first part.
    let lines: Vec<&str> = media.lines().collect();
    let last_pdt = lines
        .iter()
        .rposition(|l| l.starts_with("#EXT-X-PROGRAM-DATE-TIME:"))
        .expect("open PDT present");
    assert!(
        lines[last_pdt + 1..]
            .iter()
            .any(|l| l.starts_with("#EXT-X-PART:")),
        "open segment PDT must precede its first part:\n{media}"
    );

    // First segment PDT within 5 s of wall clock at test time minus duration.
    let first = media_pdts[0];
    let expected = clock_before_ms - 1000;
    assert!(
        (first - expected).abs() <= 5000,
        "first PDT {first} not within 5 s of {expected}"
    );
}

#[tokio::test]
async fn off_emits_no_pdt() {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_program_date_time(ProgramDateTimeSource::Off),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches_n(60));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{name}")))
            .await
            .unwrap();
        let body = body_string(resp).await;
        assert!(
            !body.contains("EXT-X-PROGRAM-DATE-TIME"),
            "{name} must carry no PDT in Off mode:\n{body}"
        );
    }
}

#[tokio::test]
async fn ts_hls_route_carries_pdt() {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_container(Container::MpegTs)
            .with_program_date_time(ProgramDateTimeSource::Arrival),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches_n(60));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(TsHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app.clone().oneshot(get("/cam/media.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    assert!(
        body.contains("EXT-X-PROGRAM-DATE-TIME"),
        "TS-HLS media playlist must carry PDT:\n{body}"
    );
    let pdts = pdt_lines(&body);
    assert!(
        !pdts.is_empty(),
        "expected a closed segment with PDT: {body}"
    );
}

// Validator test adapted from origin_iframe.rs::iframe_validator_clean.
#[tokio::test]
async fn pdt_validator_clean() {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;

    let available = std::process::Command::new("mediastreamvalidator")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !available {
        eprintln!("SKIP pdt_validator_clean: `mediastreamvalidator` not on PATH ...");
        return;
    }

    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_program_date_time(ProgramDateTimeSource::Arrival),
    );
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init"));
    for batch in video_batches_n(105) {
        for (track_id, sample) in batch {
            seg.push(TrackId::new(track_id), sample).expect("push");
        }
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add");
        }
    }

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/pdt-validator-tmp");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();

    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{name}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        fs::write(scratch.join(name), body_bytes(resp).await).unwrap();
    }

    let mut uris: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in ["media.m3u8", "iframe.m3u8"] {
        let text = fs::read_to_string(scratch.join(name)).unwrap();
        for line in text.lines() {
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
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{uri}")))
            .await
            .unwrap();
        if resp.status() == StatusCode::OK {
            fs::write(scratch.join(uri), body_bytes(resp).await).unwrap();
        }
    }

    use test_bounded::output_bounded;
    let mut cmd = Command::new("mediastreamvalidator");
    cmd.current_dir(&scratch);
    cmd.arg("--quiet")
        .arg("-t")
        .arg("5")
        .arg("-O")
        .arg(std::env::temp_dir().join("pdt-validator.json"))
        .arg("master.m3u8");
    let out = output_bounded(&mut cmd, Duration::from_secs(35)).expect("run");
    let text = fs::read_to_string(std::env::temp_dir().join("pdt-validator.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let _ = out;

    fn collect(value: &serde_json::Value, path: &str, out: &mut Vec<(String, i64, String)>) {
        if let serde_json::Value::Object(map) = value {
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
                    let lvl = if parse_failed { 1 } else { level };
                    out.push((
                        path.to_string(),
                        lvl,
                        m.get("errorComment")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ));
                }
            }
            for (k, v) in map {
                if k != "messages" {
                    collect(v, &format!("{path}/{k}"), out);
                }
            }
        } else if let serde_json::Value::Array(items) = value {
            for (i, v) in items.iter().enumerate() {
                collect(v, &format!("{path}[{i}]"), out);
            }
        }
    }
    let mut findings = Vec::new();
    collect(&json, "$", &mut findings);
    const KNOWN_1429: &[&str] = &[
        "No rendition reports in low-latency playlist",
        "Missing EXT-X-PART-INF tag",
        "Missing server control attribute",
    ];
    for f in &findings {
        eprintln!("VALIDATOR: {} level={} {}", f.0, f.1, f.2);
    }
    let errors: Vec<_> = findings.iter().filter(|f| f.1 <= 1).collect();
    let unexpected: Vec<_> = errors
        .iter()
        .filter(|f| !KNOWN_1429.contains(&f.2.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "NEW validator level-1 findings: {unexpected:?}"
    );
    for known in KNOWN_1429 {
        assert!(
            errors.iter().any(|f| f.2 == *known),
            "known finding {known:?} (#1429) no longer reported: remove it from KNOWN_1429"
        );
    }
}
