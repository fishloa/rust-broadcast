use std::time::Duration;

use crate::support::listener::bind_tcp;
use crate::support::shared_it::{fixture_string, poll_until_extinf};
use axum::http::StatusCode;
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on;
use transmux::validate::{Severity, validate_media_segment};

fn dvr_config(tmp: &std::path::Path) -> DvrConfig {
    DvrConfig {
        enabled: true,
        archive_root: tmp.to_string_lossy().to_string(),
        retention_periods: 1,
        retention_bytes: 0,
        period_duration_secs: 1,
        ..DvrConfig::default()
    }
}

fn file_config(bind: std::net::SocketAddr, path: String, tmp: &std::path::Path) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
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
        }],
        ..Config::default()
    }
}

fn segment_uris(playlist: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = playlist;
    while let Some(i) = rest.find("seg-") {
        let seg = &rest[i..];
        let end = seg.find(".m4s").map(|e| e + ".m4s".len()).unwrap_or(0);
        if end > 0 {
            out.push(seg[..end].to_string());
            rest = &seg[end..];
        } else {
            rest = &seg[1..];
        }
    }
    out
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

    // Wait for a few segments so the window has something to trim.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut full = body;
    while tokio::time::Instant::now() < deadline {
        let b = client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        if b.matches("#EXTINF").count() >= 3 {
            full = b;
            break;
        }
        full = b;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        full.matches("#EXTINF").count() >= 2,
        "need >=2 segments: {full}"
    );

    // Request a catch-up window and check it covers that window and no more.
    let windowed_url = format!("{playlist_url}?window_secs=1");
    let mut windowed = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let b = client
            .get(&windowed_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        if b.contains("#EXTINF:") {
            windowed = b;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!windowed.is_empty(), "windowed playlist empty");
    assert_eq!(
        full.matches("#EXTINF").count() - windowed.matches("#EXTINF").count(),
        1,
        "a 1s window must trim exactly one segment off the full playlist: windowed={}, full={}",
        windowed.matches("#EXTINF").count(),
        full.matches("#EXTINF").count()
    );

    // Fetch every referenced segment: served and right container.
    for uri in segment_uris(&windowed) {
        let rel = if uri.starts_with("catchup/") {
            uri.clone()
        } else {
            format!("catchup/{uri}")
        };
        let url = format!("http://{bind_addr}/cam/{rel}");
        let resp = client.get(&url).send().await.unwrap();
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

    // Request a window that has been evicted -> documented error status (404).
    // Wait for period 1 to appear (retention rolls), then p0 is evicted.
    let vod1 = format!("http://{bind_addr}/cam/vod/p1.m3u8");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        if let Ok(resp) = client.get(&vod1).send().await
            && resp.status() == StatusCode::OK
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let evicted = client
        .get(format!("http://{bind_addr}/cam/vod/p0.m3u8"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        evicted.status(),
        StatusCode::NOT_FOUND,
        "evicted period must be 404"
    );

    server.abort();
    let _ = std::fs::remove_dir_all(&tmp);
}
