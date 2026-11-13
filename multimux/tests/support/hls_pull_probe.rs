//! A production HLS pull (`run_hls_pull`) against a live origin, observed from both ends: the
//! samples it publishes into its `Trunk` as they appear, and what it asked the origin for
//! (blocking reloads, preload-hint prefetches), counted from the requests the origin receives.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::Response;
use broadcast_common::Timestamp;
use media_plane::ProgramId;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use multimux::RouteHandle;
use multimux::source::hls_pull::{HlsPullRoute, run_hls_pull};
use tokio_util::sync::CancellationToken;
use transmux::{Sample, TrackSpec};

const PRELOAD_HINT_TAG: &str = "#EXT-X-PRELOAD-HINT:";
const TIMED_RING: usize = 4096;
const RING: usize = 16;
fn nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("capacities are non-zero")
}

const READ: Duration = Duration::from_secs(30);

/// What a pull asked the origin for: blocking reloads (`_HLS_msn`/`_HLS_part` on a playlist GET)
/// and preload-hint prefetches (a part GET for the url the previous playlist's
/// `#EXT-X-PRELOAD-HINT` named).
#[derive(Debug, Default, Clone, Copy)]
pub struct Activity {
    pub blocking_reloads: u64,
    pub preload_hint_resource_fetches: u64,
}

#[derive(Default)]
pub struct Probe {
    blocking_reloads: AtomicU64,
    preload_hint_resource_fetches: AtomicU64,
    hint: Mutex<Option<String>>,
}

impl Probe {
    pub fn activity(&self) -> Activity {
        Activity {
            blocking_reloads: self.blocking_reloads.load(Ordering::SeqCst),
            preload_hint_resource_fetches: self
                .preload_hint_resource_fetches
                .load(Ordering::SeqCst),
        }
    }
}

fn preload_hint(playlist: &[u8]) -> Option<String> {
    let text = core::str::from_utf8(playlist).ok()?;
    let attributes = text
        .lines()
        .find_map(|line| line.strip_prefix(PRELOAD_HINT_TAG))?;
    attributes.split(',').find_map(|attribute| {
        let value = attribute.trim().strip_prefix("URI=\"")?;
        Some(value.trim_end_matches('"').to_string())
    })
}

async fn observe(State(probe): State<Arc<Probe>>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let is_playlist = path.ends_with(".m3u8");
    if is_playlist && query.contains("_HLS_msn") {
        probe.blocking_reloads.fetch_add(1, Ordering::SeqCst);
    }
    if !is_playlist {
        let requested = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let mut hint = probe.hint.lock().expect("probe hint lock");
        if hint.as_deref() == Some(requested.as_str()) {
            probe
                .preload_hint_resource_fetches
                .fetch_add(1, Ordering::SeqCst);
            *hint = None;
        }
    }
    let response = next.run(request).await;
    if !is_playlist {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("read the playlist body");
    *probe.hint.lock().expect("probe hint lock") = preload_hint(&bytes);
    Response::from_parts(parts, Body::from(bytes))
}

/// Wraps `app` so every request it serves is observed by `probe`.
pub fn observed(app: Router, probe: Arc<Probe>) -> Router {
    app.layer(middleware::from_fn_with_state(probe, observe))
}

pub struct Pulled {
    pub specs: Vec<TrackSpec>,
    pub samples: usize,
}

/// Pulls `playlist_url` until `enough` samples have been published, handing each to `on_sample`
/// the moment it is seen in the `Trunk`, then stops the pull. A `Lagged` cursor fails the run.
pub async fn pull_samples(
    playlist_url: &str,
    enough: usize,
    mut on_sample: impl FnMut(&Sample),
) -> Pulled {
    let route = HlsPullRoute::new("pulled", playlist_url.to_owned()).with_timeouts(
        multimux::source::IngestTimeouts {
            connect: READ,
            read: READ,
        },
    );
    let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
    let config = TrunkConfig::default()
        .with_timed_capacity(nz(TIMED_RING))
        .with_sparse_capacity(nz(RING))
        .with_segment_capacity(nz(RING))
        .with_event_capacity(nz(RING))
        .with_part_capacity(nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let route_handle = Arc::clone(&route_handle);
        let cancel = cancel.clone();
        async move { run_hls_pull(&route, config, handshake, &route_handle, cancel).await }
    });
    let trunk = route_handle.await_first_trunk().await;
    let mut cursor = trunk.subscribe_from_backlog();
    let mut samples = 0usize;
    while samples < enough && !run.is_finished() {
        match cursor.poll() {
            Some(SampleCursorItem::Timed { sample, .. }) => {
                samples += 1;
                on_sample(&sample);
            }
            Some(SampleCursorItem::Lagged { skipped }) => {
                panic!("the pull's Trunk cursor lagged by {skipped}")
            }
            Some(_) => {}
            None => tokio::task::yield_now().await,
        }
    }
    while let Some(item) = cursor.poll() {
        if let SampleCursorItem::Timed { sample, .. } = item
            && samples < enough
        {
            samples += 1;
            on_sample(&sample);
        }
    }
    let specs = tokio::time::timeout(READ, async {
        loop {
            let specs = route_handle.track_specs(ProgramId(0));
            if !specs.is_empty() {
                return specs;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the pulled programme records its track set");
    cancel.cancel();
    let _ = run.await;
    Pulled { specs, samples }
}
