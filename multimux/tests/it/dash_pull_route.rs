//! `dash_pull` through the full `serve_with_registry_on` route: the route
//! config's `allow_cross_origin` reaches the pull, and a pulled presentation
//! produces closed segments on the LL-HLS output.
//!
//! The MPD is served by one origin and names a `BaseURL` on a second, so a
//! pull that follows it contacts two origins; by default it must not.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on;

use crate::support::listener;
use listener::bind_tcp;

const ROUTE_TARGET_DURATION_SECS: f64 = 1.0;
const ROUTE_PART_TARGET_MS: u32 = 200;
const ROUTE_WINDOW_SEGMENTS: usize = 8;
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const OBSERVE_CEILING: Duration = Duration::from_secs(30);
const REFUSED_SETTLE: Duration = Duration::from_secs(3);

struct Media {
    hits: AtomicUsize,
}

async fn media_handler(
    AxumPath(name): AxumPath<String>,
    State(media): State<Arc<Media>>,
) -> axum::response::Response {
    media.hits.fetch_add(1, Ordering::SeqCst);
    match std::fs::read(test_fixtures::path(&format!("dash/{name}"))) {
        Ok(bytes) => bytes.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The media origin, and an MPD origin whose manifest points at it.
async fn two_origins() -> (String, Arc<Media>) {
    let media = Arc::new(Media {
        hits: AtomicUsize::new(0),
    });
    let (media_addr, media_listener) = bind_tcp();
    let app = Router::new()
        .route("/{name}", get(media_handler))
        .with_state(media.clone());
    tokio::spawn(async move {
        axum::serve(media_listener, app)
            .await
            .expect("media origin")
    });

    let mpd = test_fixtures::read_to_string("dash/manifest.mpd").replacen(
        "<Period ",
        &format!(r#"<BaseURL>http://{media_addr}/</BaseURL><Period "#),
        1,
    );
    let (mpd_addr, mpd_listener) = bind_tcp();
    let app = Router::new().route(
        "/manifest.mpd",
        get(move || {
            let mpd = mpd.clone();
            async move { mpd }
        }),
    );
    tokio::spawn(async move { axum::serve(mpd_listener, app).await.expect("mpd origin") });
    (format!("http://{mpd_addr}/manifest.mpd"), media)
}

fn route_config(bind: SocketAddr, mpd: String, allow_cross_origin: bool) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: ROUTE_TARGET_DURATION_SECS,
        part_target_ms: ROUTE_PART_TARGET_MS,
        window_segments: ROUTE_WINDOW_SEGMENTS,
        routes: vec![Route {
            name: "pulled".to_string(),
            input: InputSpec::DashPull {
                url: mpd,
                auth: None,
                allow_cross_origin,
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
    }
}

async fn extinf_within(client: &reqwest::Client, url: &str, ceiling: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + ceiling;
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    while tokio::time::Instant::now() < deadline {
        tick.tick().await;
        if let Ok(resp) = client.get(url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_route_that_allows_cross_origin_urls_pulls_the_media_from_the_other_origin() {
    let (mpd, media) = two_origins().await;
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        route_config(bind, mpd, true),
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let closed = extinf_within(
        &client,
        &format!("http://{bind}/pulled/media.m3u8"),
        OBSERVE_CEILING,
    )
    .await;
    server.abort();
    assert!(closed, "the pulled presentation never closed a segment");
    assert!(media.hits.load(Ordering::SeqCst) > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_route_refuses_the_other_origin_unless_it_opts_in() {
    let (mpd, media) = two_origins().await;
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        route_config(bind, mpd, false),
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let closed = extinf_within(
        &client,
        &format!("http://{bind}/pulled/media.m3u8"),
        REFUSED_SETTLE,
    )
    .await;
    server.abort();
    assert!(!closed, "nothing may be served from a refused origin");
    assert_eq!(
        media.hits.load(Ordering::SeqCst),
        0,
        "the server never fetched the address the MPD named"
    );
}
