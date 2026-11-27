//! A route whose damage policy refuses a report ends *that input's* run, and
//! the supervisor restarts it like any other failed input. The report of the
//! run it ended still reaches the route's log and metric, and the origin keeps
//! serving: the failing route is one input, not the process.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use broadcast_core::damage::{DamagePolicy, Severity};
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;

use crate::support::listener;
use listener::bind_tcp;

const ROUTE: &str = "damage-fail";
const KIND: &str = "hls_pull";
const OBSERVE_CEILING: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);
const ROUTE_TARGET_DURATION: Duration = Duration::from_secs(1);
const ROUTE_PART_TARGET_MS: u32 = 200;
const ROUTE_WINDOW_SEGMENTS: usize = 8;
const MIN_RESTARTS: f64 = 2.0;

/// A live Media Playlist whose dangling `EXTINF` describes a segment that is
/// therefore lost: the reader reports it on every fresh attempt.
const DAMAGED_PLAYLIST: &str = "#EXTM3U\n\
     #EXT-X-VERSION:7\n\
     #EXT-X-TARGETDURATION:1\n\
     #EXT-X-MEDIA-SEQUENCE:0\n\
     #EXTINF:1.000,\n";

async fn damaged_playlist_origin() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/media.m3u8", get(|| async { DAMAGED_PLAYLIST }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("local addr");
    let server =
        tokio::spawn(async move { axum::serve(listener, app).await.expect("playlist origin") });
    (format!("http://{addr}/media.m3u8"), server)
}

fn config(bind: SocketAddr, url: String) -> Config {
    Config::default()
        .with_bind(bind.to_string())
        .with_target_duration(ROUTE_TARGET_DURATION)
        .with_part_target(Duration::from_millis(u64::from(ROUTE_PART_TARGET_MS)))
        .with_window_segments(ROUTE_WINDOW_SEGMENTS)
        .with_routes(vec![Route {
            damage_policy: DamagePolicy::new().fail_on(Severity::Degraded),
            name: ROUTE.to_string(),
            input: InputSpec::HlsPull { url, auth: None },
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
        }])
}

/// The counter's value for this route and kind, from a rendered exposition.
fn damage_total(rendered: &str) -> Option<f64> {
    rendered
        .lines()
        .find(|line| {
            line.starts_with("multimux_source_damage_total")
                && line.contains(&format!("route=\"{ROUTE}\""))
                && line.contains(&format!("source=\"{KIND}\""))
        })
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse::<f64>().ok())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_report_restarts_the_input_and_the_origin_stays_up() {
    let (url, playlist_origin) = damaged_playlist_origin().await;
    let metrics = multimux::prometheus::install();
    let (bind, media_listener) = bind_tcp();
    let server = tokio::spawn(
        multimux::Server::new(
            multimux::ServeRequest::new(config(bind, url))
                .with_registry(SchemeRegistry::new())
                .with_prebound_media(media_listener),
        )
        .serve(),
    );

    let deadline = tokio::time::Instant::now() + OBSERVE_CEILING;
    let mut seen = None;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
        seen = damage_total(&metrics.render());
        if seen.is_some_and(|total| total >= MIN_RESTARTS) {
            break;
        }
    }
    assert!(
        seen.is_some_and(|total| total >= MIN_RESTARTS),
        "a refused report must be reported once per attempt and the input restarted \
         ({MIN_RESTARTS} reports seen across attempts), got {seen:?}"
    );

    let response = crate::support::http::client()
        .get(format!("http://{bind}/metrics"))
        .send()
        .await
        .expect("the origin still answers");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "the origin stays up after one route's input is restarted"
    );

    server.abort();
    playlist_origin.abort();
}
