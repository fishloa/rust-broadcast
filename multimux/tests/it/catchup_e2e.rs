use std::time::Duration;

const WINDOW_SECS: f64 = 1.0;

use crate::support::listener::bind_tcp;
use crate::support::shared_it::{fixture_string, poll_until_extinf};
use axum::http::StatusCode;
use broadcast_hls::MediaPlaylist;
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on;
use transmux::validate::{Severity, validate_media_segment};

fn dvr_config(tmp: &std::path::Path) -> DvrConfig {
    DvrConfig::default()
        .with_enabled(true)
        .with_archive_root(tmp.to_string_lossy().to_string())
        .with_retention_periods(1)
        .with_retention_bytes(0)
        .with_period_duration_secs(1)
}

fn file_config(bind: std::net::SocketAddr, path: String, tmp: &std::path::Path) -> Config {
    Config::default()
        .with_bind(bind.to_string())
        .with_target_duration_secs(0.5)
        .with_part_target(Duration::from_millis(100))
        .with_window_segments(8)
        .with_routes(vec![Route {
            name: "cam".to_string(),
            input: InputSpec::File {
                path,
                loop_file: true,
            },
            outputs: vec![OutputKind::Catchup],
            dvr: dvr_config(tmp),
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }])
}

#[tokio::test]
async fn catchup_output_end_to_end() {
    let tmp = std::env::temp_dir().join(format!("catchup-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let (bind_addr, bind_listener) = bind_tcp();
    let config = file_config(bind_addr, fixture_string("ts/h264-aac.ts"), &tmp);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));

    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/catchup.m3u8");

    // Let the archive accumulate past at least one segment boundary.
    let body = poll_until_extinf(&client, &playlist_url).await;
    assert!(body.contains("#EXTINF:"), "playlist: {body}");

    // Request a catch-up window and the full playlist back to back until both
    // share one live edge, then check the window is exactly the trailing run of
    // the full playlist whose span before the edge segment fits the window.
    let windowed_url = format!("{playlist_url}?window_secs=1");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let (full, windowed) = loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "full and windowed playlists never shared a live edge"
        );
        let full_text = client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let windowed_text = client
            .get(&windowed_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let full = MediaPlaylist::parse(&full_text).expect("full catch-up playlist parses");
        let windowed =
            MediaPlaylist::parse(&windowed_text).expect("windowed catch-up playlist parses");
        let same_edge =
            full.segments.last().map(|s| &s.uri) == windowed.segments.last().map(|s| &s.uri);
        if same_edge && full.segments.len() >= 3 {
            break (full, windowed);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let mut keep_from = full.segments.len() - 1;
    let mut span = 0.0_f64;
    for (i, segment) in full.segments[..keep_from].iter().enumerate().rev() {
        span += segment.duration.get();
        if span > WINDOW_SECS {
            break;
        }
        keep_from = i;
    }
    let expected: Vec<&str> = full.segments[keep_from..]
        .iter()
        .map(|s| s.uri.as_str())
        .collect();
    let got: Vec<&str> = windowed.segments.iter().map(|s| s.uri.as_str()).collect();
    assert_eq!(
        got, expected,
        "window_secs=1 must keep exactly the trailing 1 s of the archive"
    );
    assert!(
        got.len() < full.segments.len(),
        "the window must trim at least one segment: full={}",
        full.segments.len()
    );

    // Fetch every referenced segment: served and right container.
    for segment in &windowed.segments {
        let url = reqwest::Url::parse(&playlist_url)
            .and_then(|base| base.join(&segment.uri))
            .expect("segment URI resolves against the playlist URL");
        let resp = client.get(url.clone()).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "GET {url}");
        let bytes = resp.bytes().await.unwrap();
        assert!(!bytes.is_empty(), "empty segment {url}");
        let issues = validate_media_segment(&bytes);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{url}: {errors:?}");
    }

    // Request a window that has been evicted -> documented error status (404). With one
    // retained period, period 0 is evicted as soon as the archive rolls; period 1 itself is
    // evicted by period 2 a second later, so it is not a stable thing to wait for: poll period 0
    // itself until it answers 404.
    let vod0 = format!("http://{bind_addr}/cam/vod/p0.m3u8");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut status = client.get(&vod0).send().await.unwrap().status();
    while status != StatusCode::NOT_FOUND && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        status = client.get(&vod0).send().await.unwrap().status();
    }
    assert_eq!(status, StatusCode::NOT_FOUND, "evicted period must be 404");

    server.abort();
    let _ = std::fs::remove_dir_all(&tmp);
}
