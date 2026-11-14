//! `hls_pull` against CMAF cut from a real fixture and served by a real
//! loopback origin, through the full `serve_with_registry_on` route:
//!
//! - a sliding-window live playlist must yield closed segments on the route's
//!   LL-HLS output (round-9 F5: a segment's worth of samples arrives in one
//!   fetch and used to overrun the trunk's timed ring before the segmenter
//!   could drain it, so no segment ever closed);
//! - a finite `#EXT-X-ENDLIST` playlist must be fetched once and then served,
//!   not re-pulled in a reconnect loop (round-9 F4).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry_on;
use transmux::Repackage;

use crate::support::listener;
use crate::support::shared_it::observe_extinf;
use listener::bind_tcp;

const SOURCE_SEGMENT_SECS: f64 = 2.0;
const PLAYLIST_TARGET_DURATION: u32 = 4;
const WINDOW_SEGMENTS: usize = 4;
const LIVE_ADVANCE: Duration = Duration::from_millis(300);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const VOD_SETTLE: Duration = Duration::from_secs(4);
const ROUTE_TARGET_DURATION_SECS: f64 = 1.0;
const ROUTE_PART_TARGET_MS: u32 = 200;
const ROUTE_WINDOW_SEGMENTS: usize = 8;

struct Cmaf {
    init: Vec<u8>,
    segments: Vec<Vec<u8>>,
}

fn cmaf_from_real_ts() -> Cmaf {
    let ts = std::fs::read(test_fixtures::path("ts/h264-aac-40s.ts")).expect("read fixture");
    let media = transmux::TsDemux::new().demux(&ts).expect("demux fixture");
    let out = Repackage::new(SOURCE_SEGMENT_SECS)
        .run_media(&media)
        .expect("repackage");
    Cmaf {
        init: out.init_segment,
        segments: out.media_segments,
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Live,
    Vod,
}

struct Origin {
    cmaf: Cmaf,
    mode: Mode,
    started: Instant,
    segment_gets: AtomicUsize,
}

fn playlist(origin: &Origin) -> String {
    let total = origin.cmaf.segments.len();
    let (first, last, endlist) = match origin.mode {
        Mode::Vod => (0, total - 1, true),
        Mode::Live => {
            let newest = ((origin.started.elapsed().as_millis() / LIVE_ADVANCE.as_millis())
                as usize)
                .min(total - 1);
            (newest.saturating_sub(WINDOW_SEGMENTS - 1), newest, false)
        }
    };
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{PLAYLIST_TARGET_DURATION}\n\
         #EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-MAP:URI=\"init.mp4\"\n"
    );
    for i in first..=last {
        out.push_str(&format!("#EXTINF:{SOURCE_SEGMENT_SECS:.1},\nseg{i}.m4s\n"));
    }
    if endlist {
        out.push_str("#EXT-X-ENDLIST\n");
    }
    out
}

async fn handler(AxumPath(name): AxumPath<String>, State(origin): State<Arc<Origin>>) -> Response {
    if name == "media.m3u8" {
        return playlist(&origin).into_response();
    }
    if name == "init.mp4" {
        return origin.cmaf.init.clone().into_response();
    }
    if let Some(idx) = name
        .strip_prefix("seg")
        .and_then(|s| s.strip_suffix(".m4s"))
        .and_then(|s| s.parse::<usize>().ok())
        && let Some(bytes) = origin.cmaf.segments.get(idx)
    {
        origin.segment_gets.fetch_add(1, Ordering::SeqCst);
        return bytes.clone().into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn serve_origin(mode: Mode) -> (String, Arc<Origin>) {
    let origin = Arc::new(Origin {
        cmaf: cmaf_from_real_ts(),
        mode,
        started: Instant::now(),
        segment_gets: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/{name}", get(handler))
        .with_state(origin.clone());
    let (addr, tcp) = bind_tcp();
    tokio::spawn(async move {
        axum::serve(tcp, app).await.expect("origin server");
    });
    (format!("http://{addr}/media.m3u8"), origin)
}

fn route_config(bind: SocketAddr, upstream: String) -> Config {
    Config::default()
        .with_bind(bind.to_string())
        .with_target_duration_secs(ROUTE_TARGET_DURATION_SECS)
        .with_part_target(Duration::from_millis(u64::from(ROUTE_PART_TARGET_MS)))
        .with_window_segments(ROUTE_WINDOW_SEGMENTS)
        .with_routes(vec![Route {
            name: "pulled".to_string(),
            input: InputSpec::HlsPull {
                url: upstream,
                auth: None,
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
        }])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sliding_window_live_playlist_closes_segments_on_the_output() {
    let (upstream, _origin) = serve_origin(Mode::Live).await;
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        route_config(bind, upstream),
        SchemeRegistry::new(),
    ));

    let client = reqwest::Client::new();
    let playlist = observe_extinf(&client, &format!("http://{bind}/pulled/media.m3u8")).await;
    server.abort();
    assert!(
        playlist.is_some(),
        "a live pull never produced a closed segment on the LL-HLS output"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finite_playlist_is_fetched_once_and_served() {
    let (upstream, origin) = serve_origin(Mode::Vod).await;
    let segment_count = origin.cmaf.segments.len();
    let (bind, listener) = bind_tcp();
    let server = tokio::spawn(serve_with_registry_on(
        listener,
        route_config(bind, upstream),
        SchemeRegistry::new(),
    ));

    let client = reqwest::Client::new();
    let playlist = observe_extinf(&client, &format!("http://{bind}/pulled/media.m3u8")).await;
    assert!(playlist.is_some(), "the finished VOD must be served");

    let settled = tokio::time::Instant::now() + VOD_SETTLE;
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    while tokio::time::Instant::now() < settled {
        tick.tick().await;
    }
    assert_eq!(
        origin.segment_gets.load(Ordering::SeqCst),
        segment_count,
        "every segment is fetched exactly once, not re-pulled on a reconnect loop"
    );
    let ready = client
        .get(format!("http://{bind}/readyz"))
        .send()
        .await
        .expect("readyz");
    assert_eq!(ready.status(), reqwest::StatusCode::OK);
    server.abort();
}
