//! End-to-end integration tests for the I-frame (trick-play) origin path
//! (issue #1413 P3b). Reuses the `feed_via_segmenter` pattern from
//! `origin_llhls.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use broadcast_common::traits::Parse;
use transmux::TrackId;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::output::ts_hls::TsHlsOutput;
use multimux::route::RouteHandle;
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

    seg.flush().expect("flush succeeds");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add_segment");
    }
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

fn video_batches() -> Vec<Vec<(u32, Sample)>> {
    video_batches_n(120)
}

#[tokio::test]
async fn iframe_master_variant_gating() {
    // before any segment: no I-frame variant
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
    );
    let seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    store.set_init(program, seg.init_segment().expect("init"));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        !playlist.contains("EXT-X-I-FRAME-STREAM-INF"),
        "before segments: {playlist}"
    );

    // after segments: exactly one
    let store2 = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    feed_via_segmenter(&store2, vec![video_track_spec()], video_batches());
    let mut streams2 = HashMap::new();
    streams2.insert(
        "cam".to_string(),
        (
            store2.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app2 = router(Arc::new(AppState::new(streams2)));
    let resp = app2.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert_eq!(
        playlist.matches("EXT-X-I-FRAME-STREAM-INF").count(),
        1,
        "playlist: {playlist}"
    );
    assert!(
        playlist.contains("URI=\"iframe.m3u8\""),
        "playlist: {playlist}"
    );

    // audio-only route: no I-frame variant
    use transmux::rtp_sdp::aac_config_from_asc_hex;
    let audio_spec = TrackSpec::new(
        TrackId::new(1),
        48000,
        aac_config_from_asc_hex("1210").expect("aac"),
    );
    let mut audio_batches = Vec::new();
    for i in 0..120u32 {
        let data = vec![0xBBu8; 64];
        let sample = Sample::new(
            data,
            Some(i64::from(i) * 1024),
            Some(i64::from(i) * 1024),
            Some(1024),
            true,
        );
        audio_batches.push(vec![(1u32, sample)]);
    }
    let audio_store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    feed_via_segmenter(&audio_store, vec![audio_spec], audio_batches);
    let mut streams3 = HashMap::new();
    streams3.insert(
        "cam".to_string(),
        (
            audio_store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app3 = router(Arc::new(AppState::new(streams3)));
    let resp = app3.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        !playlist.contains("EXT-X-I-FRAME-STREAM-INF"),
        "audio-only: {playlist}"
    );
}

#[tokio::test]
async fn iframe_playlist_byterange_matches_independent_calc() {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        playlist.contains("#EXT-X-I-FRAMES-ONLY"),
        "playlist: {playlist}"
    );

    // parse entries
    let mut entries: Vec<(u64, String)> = Vec::new();
    let lines: Vec<&str> = playlist.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("#EXT-X-BYTERANGE:") {
            let len: u64 = rest.split('@').next().unwrap().trim().parse().unwrap();
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
            {
                j += 1;
            }
            if j < lines.len() {
                entries.push((len, lines[j].trim().to_string()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    assert!(!entries.is_empty(), "no entries: {playlist}");

    for (len, uri) in &entries {
        // independent calc from served segment bytes
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{uri}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let seg = body_bytes(resp).await;
        let expected = first_sample_end(&seg);
        assert_eq!(
            *len, expected,
            "uri {uri}: advertised {len} vs computed {expected}"
        );
    }
}

fn first_sample_end(seg: &[u8]) -> u64 {
    // find moof
    let mut offset = 0usize;
    let mut moof_off: Option<usize> = None;
    let mut moof_total: usize = 0;
    let mut moof_hdr: usize = 0;
    while offset + transmux::box_types::BOX_HEADER_MIN_SIZE <= seg.len() {
        let rest = &seg[offset..];
        let header = transmux::box_types::BoxHeader::parse(rest).expect("box header");
        let hdr_sz = header.header_size();
        if hdr_sz > rest.len() {
            break;
        }
        let total = if header.size == u64::from(transmux::box_types::SIZE_TO_EOF) {
            rest.len()
        } else {
            match usize::try_from(header.size) {
                Ok(t) if t >= hdr_sz => t,
                _ => break,
            }
        };
        if header.box_type.is(b"moof") {
            moof_off = Some(offset);
            moof_total = total.min(rest.len());
            moof_hdr = hdr_sz;
            break;
        }
        let step = total.min(rest.len());
        if step == 0 {
            break;
        }
        offset += step;
    }
    let moof_off = moof_off.expect("moof");
    let body = &seg[moof_off + moof_hdr..moof_off + moof_total];
    let moof = transmux::movie_fragment::MovieFragmentBox::parse_body(body).expect("moof parse");
    let mdats = transmux::frag_offsets::mdat_ranges(seg);
    let samples =
        transmux::frag_offsets::sample_ranges_in(seg, &mdats, moof_off, &moof, TrackId::new(1))
            .expect("sample ranges");
    samples[0].end as u64
}

#[tokio::test]
async fn iframe_range_requests() {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // get one entry
    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    let playlist = body_string(resp).await;
    let mut entries: Vec<(u64, String)> = Vec::new();
    let lines: Vec<&str> = playlist.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("#EXT-X-BYTERANGE:") {
            let len: u64 = rest.split('@').next().unwrap().trim().parse().unwrap();
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
            {
                j += 1;
            }
            if j < lines.len() {
                entries.push((len, lines[j].trim().to_string()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let (len, uri) = entries[0].clone();

    // full segment
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{uri}")))
        .await
        .unwrap();
    let total_seg = body_bytes(resp).await;

    // range request
    let req = Request::builder()
        .uri(format!("/cam/{uri}"))
        .header("range", format!("bytes=0-{}", len - 1))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let cr = resp
        .headers()
        .get("content-range")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cr, format!("bytes 0-{}/{}", len - 1, total_seg.len()));
    let body = body_bytes(resp).await;
    assert_eq!(body, total_seg[..len as usize]);
}

#[tokio::test]
async fn iframe_part_query_rejected() {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches());
    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app
        .oneshot(get("/cam/iframe.m3u8?_HLS_part=0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ts_hls_no_iframe() {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
    );
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(TsHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = app.clone().oneshot(get("/cam/master.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;
    assert!(
        !playlist.contains("EXT-X-I-FRAME-STREAM-INF"),
        "playlist: {playlist}"
    );
}

// validator test (f)
#[tokio::test]
async fn iframe_validator_clean() {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;

    // availability check
    let available = std::process::Command::new("mediastreamvalidator")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !available {
        eprintln!("SKIP iframe_validator_clean: `mediastreamvalidator` not on PATH ...");
        return;
    }

    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
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
    // leave the in-progress (open) segment unflushed so a preload hint exists

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/iframe-validator-tmp");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();

    // fetch playlists
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{name}")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        fs::write(scratch.join(name), body_bytes(resp).await).unwrap();
    }

    // collect URIs from media + iframe playlists
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

    // run validator
    use test_bounded::output_bounded;
    let mut cmd = Command::new("mediastreamvalidator");
    cmd.current_dir(&scratch);
    cmd.arg("--quiet")
        .arg("-t")
        .arg("5")
        .arg("-O")
        .arg(std::env::temp_dir().join("iframe-validator.json"))
        .arg("master.m3u8");
    let out = output_bounded(&mut cmd, Duration::from_secs(35)).expect("run");
    let text = fs::read_to_string(std::env::temp_dir().join("iframe-validator.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let _ = out;
    // collect findings
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
    // Level-1 (MUST) findings Apple's validator still reports against the
    // live LL-HLS origin, each reproduced and documented in issue #1429
    // (the first two also fire on a single-variant master with NO I-frame
    // variant). This is an exact allow-list, not a suppression: any other
    // level-1 finding fails the test, and so does a listed one that
    // disappears (so the list shrinks when #1429 is fixed).
    const KNOWN_1429: &[&str] = &[
        "No rendition reports in low-latency playlist",
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

// (g) SCTE-35 case
#[tokio::test]
async fn iframe_scte35_emsg_shift() {
    use media_plane::trunk::EventAnchor;
    use timed_metadata::{MediaTime, TimeAnchor, Timeline};

    fn parse_emsg_boxes(bytes: &[u8]) -> Vec<transmux::EmsgBox<'_>> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 8 <= bytes.len() {
            let size =
                u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
            if size < 8 || i + size > bytes.len() {
                break;
            }
            if &bytes[i + 4..i + 8] == b"emsg"
                && let Ok(e) = transmux::EmsgBox::parse(&bytes[i..i + size])
            {
                out.push(e);
            }
            i += size;
        }
        out
    }

    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8)
            .with_declared_bandwidth_bps(5_000_000),
    );
    let program = media_plane::ProgramId(0);
    let trunk = store.publish_new_program(program);

    // Note segment boundaries up front on a short-lived segment writer.
    let seg_writer = trunk.segment_writer().expect("seg writer");
    seg_writer.set_time_anchor(TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 1_000_000_000_000,
    });
    seg_writer.note_segment_start(media_plane::ids::SegmentSeq::new(1), MediaTime(0));
    seg_writer.note_segment_start(media_plane::ids::SegmentSeq::new(2), MediaTime(90_000));
    drop(seg_writer);

    // build segmenter init
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init"));

    for batch in video_batches() {
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
    seg.flush().expect("flush");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add");
    }

    // publish scte35 event at start of segment 1
    let writer = trunk.writer().expect("writer");
    let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
    let splice_bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let mut timeline = Timeline::new();
    let ev = timeline.push_scte35(&splice_bytes).unwrap();
    writer.publish_event(
        ev.clone(),
        EventAnchor::Media(ev.at.unwrap_or(MediaTime(0))),
    );

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // I-frame playlist URIs use the iframe-seg- prefix.
    let resp = app.clone().oneshot(get("/cam/iframe.m3u8")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let playlist = body_string(resp).await;

    let mut len: u64 = 0;
    let mut uri = String::new();
    let lines: Vec<&str> = playlist.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("#EXT-X-BYTERANGE:") {
            len = rest.split('@').next().unwrap().trim().parse().unwrap();
            let mut j = i + 1;
            while j < lines.len()
                && (lines[j].trim().is_empty() || lines[j].trim_start().starts_with('#'))
            {
                j += 1;
            }
            if j < lines.len() {
                uri = lines[j].trim().to_string();
            }
            break;
        } else {
            i += 1;
        }
    }
    assert!(!uri.is_empty(), "no uri found");
    assert!(uri.starts_with("iframe-seg-"), "uri={uri}");

    // stored bytes behind the I-frame URI (no emsg)
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{uri}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let stored = body_bytes(resp).await;
    let computed = first_sample_end(&stored);
    assert_eq!(len, computed, "advertised len must match stored geometry");
    assert!(
        parse_emsg_boxes(&stored).is_empty(),
        "stored bytes must have no emsg"
    );

    // Range request returns first len bytes of stored segment, no emsg in them.
    let req = Request::builder()
        .uri(format!("/cam/{uri}"))
        .header("range", format!("bytes=0-{}", len - 1))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let range_body = body_bytes(resp).await;
    assert_eq!(range_body, stored[..len as usize]);
    assert!(parse_emsg_boxes(&range_body).is_empty());

    // nonexistent iframe-seg seq => 404
    let nonexistent = {
        let (prefix, _) = uri.rsplit_once('-').unwrap();
        format!("{prefix}-999.m4s")
    };
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{nonexistent}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // media seg- fetch of same seq returns emsg-injected bytes
    let media_uri = uri.replacen("iframe-seg-", "seg-", 1);
    let resp = app
        .clone()
        .oneshot(get(&format!("/cam/{media_uri}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let media_served = body_bytes(resp).await;
    assert!(
        media_served.len() > stored.len(),
        "emsg injection must grow length"
    );
    assert!(
        !parse_emsg_boxes(&media_served).is_empty(),
        "media segment must contain emsg"
    );
    assert!(parse_emsg_boxes(&stored).is_empty());

    let _ = trunk;
}
