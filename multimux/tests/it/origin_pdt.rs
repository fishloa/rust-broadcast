//! End-to-end coverage of `#EXT-X-PROGRAM-DATE-TIME` on the multimux
//! LL-HLS/TS-HLS origin (issue #1423 part B2b, Apple authoring 8.4 MUST).
//!
//! Feeding is done through `RouteHandle::with_program_date_time(...)` rather
//! than the `Config` path: the detailed assertions below need a controlled
//! segment timeline, and `feed_via_segmenter` is the same helper the
//! `origin_iframe`/`origin_llhls` suites use. The `Config` -> `RouteHandle`
//! wiring itself is exercised by `multimux`'s own `origin` module tests and
//! by the mutation checks in the issue.

use crate::support::origin::{body_bytes, body_string, get};
use crate::support::segmenter_feed::{
    PART_TARGET_MS, TARGET_DURATION_SECS, feed_via_segmenter, video_batches_n, video_track_spec,
};
use multimux::route::RouteConfig;
use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use tower::ServiceExt;

use multimux::config::ProgramDateTimeSource;
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::OutputKind;
use multimux::output::hls::HlsOutput;
use multimux::registry::SchemeRegistry;
use multimux::route::RouteHandle;
use multimux::serve_with_registry_on;
use transmux::Container;
use transmux::TrackId;

use crate::support::listener;
use crate::support::shared_it::poll_until_extinf_yield;
use listener::bind_tcp;
use transmux::ll_hls::LlHlsSegmenter;

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
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_program_date_time(ProgramDateTimeSource::Arrival),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches_n(120));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
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
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_program_date_time(ProgramDateTimeSource::Off),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches_n(60));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
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
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_container(Container::MpegTs)
        .with_program_date_time(ProgramDateTimeSource::Arrival),
    );
    feed_via_segmenter(&store, vec![video_track_spec()], video_batches_n(60));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(HlsOutput::segmented(
                multimux::output::hls::DEFAULT_PLAYLIST_NAME,
            )) as Arc<dyn Output>],
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
    use std::time::Duration;

    if !test_fixtures::process::msv::available() {
        eprintln!("SKIP pdt_validator_clean: `mediastreamvalidator` not on PATH ...");
        return;
    }

    let store = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(TARGET_DURATION_SECS)
                .with_part_target_ms(PART_TARGET_MS)
                .with_window_segments(8),
        )
        .with_program_date_time(ProgramDateTimeSource::Arrival),
    );
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track_spec()],
        transmux::VIDEO_CLOCK_TIMESCALE,
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
            vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
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

    let json =
        test_fixtures::process::msv::run_report(&scratch, "master.m3u8", Duration::from_secs(35));
    let findings = test_fixtures::process::msv::findings(&json);
    const KNOWN: &[(&str, &str)] = &[
        // Issue #1595: a one-rendition low-latency route has no sibling for the
        // media playlist to report (RFC 8216bis Appendix B.1); a report for the
        // I-frame playlist is rejected as "Unknown rendition report".
        ("No rendition reports in low-latency playlist", "#1595"),
        // Issue #1594: the validator reports this on a playlist without
        // EXT-X-PART (here the I-frame playlist) whenever the primary has
        // parts, for any EXT-X-SERVER-CONTROL value; dropping the tag silences
        // it and so does a part tag. RFC 8216bis section 6.2.4 requires every
        // Media Playlist to carry the same EXT-X-SERVER-CONTROL, and section
        // 4.4.4.9 says I-frame playlists SHOULD NOT use Partial Segments.
        ("Missing server control attribute", "#1594"),
    ];
    for f in &findings {
        eprintln!("VALIDATOR: {} level={} {}", f.0, f.1, f.2);
    }
    let errors: Vec<_> = findings.iter().filter(|f| f.1 <= 1).collect();
    let unexpected: Vec<_> = errors
        .iter()
        .filter(|f| !KNOWN.iter().any(|(known, _)| *known == f.2))
        .collect();
    assert!(
        unexpected.is_empty(),
        "NEW validator level-1 findings: {unexpected:?}"
    );
    for (known, issue) in KNOWN {
        assert!(
            errors.iter().any(|f| f.2 == *known),
            "known finding {known:?} ({issue}) no longer reported: remove it from KNOWN"
        );
    }
}

fn file_config(
    bind: std::net::SocketAddr,
    program_date_time: Option<multimux::config::ProgramDateTimeSource>,
) -> Config {
    let mut cfg = Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input: InputSpec::File {
                path: test_fixtures::path("ts/h264-aac.ts")
                    .to_string_lossy()
                    .into_owned(),
                loop_file: false,
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }],
        ..Config::default()
    };
    if let Some(pdt) = program_date_time {
        cfg.program_date_time = pdt;
    }
    cfg
}

#[tokio::test]
async fn config_default_arrival_emits_pdt() {
    let (bind_addr, bind_listener) = bind_tcp();
    // program_date_time omitted => Config default = arrival.
    let config = file_config(bind_addr, None);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let clock_before_ms = jiff::Timestamp::now().as_millisecond();
    let playlist = poll_until_extinf_yield(&client, &playlist_url).await;
    assert!(
        playlist.contains("EXT-X-PROGRAM-DATE-TIME"),
        "default config must emit PDT:\n{playlist}"
    );
    let first: i64 = playlist
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:"))
        .unwrap()
        .trim()
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond();
    assert!(
        (first - (clock_before_ms - 500)).abs() <= 60_000,
        "first PDT {first} not within 60s of wall clock {clock_before_ms}"
    );
    server.abort();
}

#[tokio::test]
async fn config_off_emits_no_pdt() {
    let (bind_addr, bind_listener) = bind_tcp();
    let config = file_config(
        bind_addr,
        Some(multimux::config::ProgramDateTimeSource::Off),
    );
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = poll_until_extinf_yield(&client, &playlist_url).await;
    assert!(
        !playlist.contains("EXT-X-PROGRAM-DATE-TIME"),
        "off config must not emit PDT:\n{playlist}"
    );
    server.abort();
}
