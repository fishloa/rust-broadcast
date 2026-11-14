//! Sample-sequence goldens for the Smooth pull ingest session playing the
//! committed `fixtures/smooth/{av,av-low}` fixtures from a loopback origin.
//! `SMOOTH_PULL_GOLDEN_BLESS=<dir>` writes the traces instead of comparing.
//!
//! The session's instants run on a clock [`CLOCK_SPEEDUP`] times faster than real time
//! ([`ScaledHostClock`]), so its retry backoff for the deliberately missing fragment (2 s, 4 s,
//! 8 s, then 10 s per attempt) costs a fraction of that in wall time. The loopback origin answers
//! in real time, well inside the scaled request timers.

use std::sync::Arc;
use std::time::Duration;

use crate::clock::ScaledHostClock;
use crate::source::smooth_pull::{SmoothPullRoute, run_with_clock};
use crate::{MultimuxError, RouteConfig, RouteHandle};
use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use broadcast_client::ErrorKind;
use broadcast_common::Timestamp;
use media_plane::ProgramId;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use tokio_util::sync::CancellationToken;

fn nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("non-zero ring")
}

const TIMED_RING: usize = 4096;
const RING: usize = 16;
const CLOCK_SPEEDUP: u32 = 20;
const PULL_CEILING: Duration = Duration::from_secs(150);
const READ_TIMEOUT: Duration = Duration::from_secs(120);
const GOLDEN_BLESS_ENV: &str = "SMOOTH_PULL_GOLDEN_BLESS";
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

fn fnv1a_trace_line(track: u32, sample: &transmux::Sample) -> String {
    format!(
        "sample track={} dts={:?} pts={:?} dur={:?} sync={} len={} fnv={:016x}\n",
        track,
        sample.dts,
        sample.pts,
        sample.duration,
        sample.flags.is_sync,
        sample.data.len(),
        fnv1a(&sample.data),
    )
}

async fn fixture_handler(
    AxumPath(path): AxumPath<String>,
    State(fixture): State<Arc<String>>,
) -> axum::response::Response {
    let rel = path.replace("%28", "(").replace("%29", ")");
    match std::fs::read(test_fixtures::path(&format!("smooth/{fixture}/{rel}"))) {
        Ok(bytes) => bytes.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_fixture(fixture: &str) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/{*path}", get(fixture_handler))
        .with_state(Arc::new(fixture.to_owned()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum server");
    });
    (format!("http://{addr}/Manifest"), server)
}

/// Pulls the fixture to its end through `run_smooth_pull` and renders the track set and every
/// sample the route published, in the order it published them.
async fn sample_trace(fixture: &str) -> String {
    let (url, server) = serve_fixture(fixture).await;
    let route = SmoothPullRoute::new("golden", url).with_timeouts(
        broadcast_common::timeouts::Timeouts::default()
            .with_connect(READ_TIMEOUT)
            .with_read_idle(READ_TIMEOUT),
    );
    let route_handle = Arc::new(RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(4.0)
            .with_part_target(std::time::Duration::from_millis(500))
            .with_window_segments(4),
    ));
    let config = TrunkConfig::default()
        .with_timed_capacity(nz(TIMED_RING))
        .with_sparse_capacity(nz(RING))
        .with_segment_capacity(nz(RING))
        .with_event_capacity(nz(RING))
        .with_part_capacity(nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let result = tokio::time::timeout(
        PULL_CEILING,
        run_with_clock(
            &route,
            config,
            handshake,
            &route_handle,
            CancellationToken::new(),
            Arc::new(ScaledHostClock::new(CLOCK_SPEEDUP)),
        ),
    )
    .await
    .expect("a static manifest must end");
    server.abort();
    let mut out = String::new();
    let trunk = route_handle.await_first_trunk().await;
    let specs = loop {
        let specs = route_handle.track_specs(ProgramId(0));
        if !specs.is_empty() {
            break specs;
        }
        tokio::task::yield_now().await;
    };
    let ids: Vec<u32> = specs.iter().map(|t| t.track_id.value()).collect();
    out.push_str(&format!("tracks {ids:?}\n"));
    let mut cursor = trunk.subscribe_from_backlog();
    while let Some(item) = cursor.poll() {
        if let SampleCursorItem::Timed { track_id, sample } = item {
            out.push_str(&fnv1a_trace_line(track_id.value(), &sample));
        }
    }
    match result {
        Ok(()) => out.push_str("ended\n"),
        Err(MultimuxError::Session {
            kind: ErrorKind::SegmentFetchFailed { status },
            ..
        }) => out.push_str(&format!(
            "fatal connect failed: smooth-pull: fragment fetch failed (status {status:?})\n"
        )),
        Err(e) => out.push_str(&format!("fatal {e}\n")),
    }
    out
}

async fn check_golden(fixture: &str) {
    let name = format!("pull-multimux-{fixture}.txt");
    let actual = sample_trace(fixture).await;
    if let Ok(dir) = std::env::var(GOLDEN_BLESS_ENV) {
        std::fs::write(std::path::Path::new(&dir).join(&name), &actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(test_fixtures::path(&format!("golden/smooth/{name}")))
        .expect("read golden");
    assert_eq!(actual, expected, "{name} differs from the recorded trace");
}

#[tokio::test]
async fn smooth_pull_golden_av() {
    check_golden("av").await;
}

#[tokio::test]
async fn smooth_pull_golden_av_low() {
    check_golden("av-low").await;
}
