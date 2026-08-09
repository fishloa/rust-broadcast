//! ACAP entrypoint (`device`-gated; only builds inside the Axis ACAP Native
//! SDK sysroot). Wires the real capture -> LL-HLS pipeline together:
//!
//! - Loads [`acap_multimux::admin::Config`] from the ACAP
//!   `axparameter`-backed [`acap_multimux::admin::AxParameterStore`].
//! - Builds a [`multimux::RouteHandle`] sized from the config's LL-HLS
//!   tuning (target segment duration / part target / window).
//! - Starts [`acap_multimux::vdo_source::VdoIngestSession`] and drives it
//!   through [`multimux::supervise_driver`]/[`multimux::source::advance_route`]
//!   on a **dedicated OS thread with its own current-thread tokio runtime** —
//!   see "Threading" below.
//! - Serves the LL-HLS origin (`multimux::origin::router`) nested under
//!   `/hls`, merged with the admin config/status routes
//!   (`acap_multimux::admin::admin_router`), on `127.0.0.1:<port>` (matching
//!   `manifest.json`'s `reverseProxy` targets).
//!
//! # Threading
//!
//! [`VdoIngestSession::feed`](broadcast_common::Stage::feed) ultimately calls
//! `vdo::RunningStream::next_buffer`, a **blocking** FFI call into
//! `libvdo.so` that only returns once the camera has produced the next frame
//! (see `vdo_source.rs`'s module doc). Running that on an axum worker thread
//! would eventually starve every request being served on the same
//! `rt-multi-thread` runtime once all worker threads happen to be parked in
//! that blocking call. Instead the whole capture/segment/store pipeline runs
//! on a plain `std::thread::spawn`'d OS thread with its own
//! `current_thread` tokio runtime — the blocking call only ever stalls that
//! one dedicated thread, never axum's.
//!
//! # Why `supervise_driver`/`advance_route`, not `run_pipeline`
//!
//! multimux 0.5 (issue #805) deleted `multimux::pipeline::{run_pipeline,
//! SampleSource}` outright once every input — including this app's VDO
//! capture, the last holdout — was ported onto the single
//! `media_plane::ingress` `Dialer`/`Listener` + `IngestSession` architecture.
//! [`run_vdo_capture`] is this app's own `attempt` closure (the same shape
//! `multimux::examples::custom_scheme`'s `run_demo` and every in-tree
//! `run_*` entry point use): [`multimux::supervise_driver`] calls it, retries
//! it with backoff on failure, and [`multimux::source::advance_route`] is
//! the one facade call inside it that both publishes the driver-minted
//! `Trunk` into the route and turns queued samples into servable segments.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acap_multimux::admin::{self, AxParameterStore, ConfigStore, StatusHandle};
use acap_multimux::convert::Codec;
use acap_multimux::vdo_source::VdoIngestSession;
use broadcast_common::Timestamp;
use log::{error, info};
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;
use multimux::origin::AppState;
use multimux::output::{Output, OutputKind};
use multimux::source::{DriverProgress, advance_route};
use multimux::{Backoff, MultimuxError, RouteHandle};

/// The single served stream's name in LL-HLS URLs
/// (`…/hls/<STREAM_NAME>/media.m3u8`) — this app captures exactly one VDO
/// channel per `Config`, so one fixed stream name is enough. Also doubles as
/// the route name `supervise_driver` logs/labels metrics under.
const STREAM_NAME: &str = "cam";

/// The URL prefix AXIS OS's Apache reverse proxy forwards verbatim to this
/// app — `/local/<appName>` with `appName` from `manifest.json`
/// (`acapmultimux`). The proxy does not strip it, so every route is served
/// under this prefix. Keep in lockstep with `manifest.json`'s `setup.appName`.
const URL_PREFIX: &str = "/local/acapmultimux";

/// `Trunk` ring capacities for the VDO capture driver — this app's own
/// choice (mirroring `multimux::source::driver_trunk_config`'s production
/// sizing, which is `pub(crate)` and so not reusable directly by an external
/// crate like this one). `segment_capacity` is `Config::window_segments`
/// (the advertised LL-HLS window depth); the rest are generous fixed sizes
/// for a single-track video capture.
const DRIVER_TIMED_CAPACITY: usize = 64;
const DRIVER_SPARSE_CAPACITY: usize = 16;
const DRIVER_EVENT_CAPACITY: usize = 64;
const DRIVER_PART_CAPACITY: usize = 64;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    acap_logging::init_logger();
    info!("acap-multimux: starting");

    let store = match AxParameterStore::new() {
        Ok(store) => Arc::new(store),
        Err(e) => {
            error!("acap-multimux: axparameter store open failed: {e}");
            std::process::exit(1);
        }
    };

    let status = StatusHandle::new();

    // Issue #955: a broken config backend used to be indistinguishable from
    // an unconfigured one — `load()` swallowed the error and handed back
    // `Config::default()` either way. `LoadOutcome::error` is `Some` only
    // for a genuinely broken backend (not "nothing stored yet"), so record
    // it on `status` immediately — before the config's own effects (codec,
    // port, …) are even applied — so `/admin/status`'s `last_error` shows
    // it from the very first request, not just after a `POST /admin/config`
    // round-trip. Uses `set_config_error`, not `set_last_error`: the capture
    // pipeline (started below, on its own thread) clears/sets
    // `last_error`'s pipeline slot on every retry attempt, which would
    // otherwise erase this within moments of boot.
    let outcome = store.load();
    if let Some(reason) = outcome.error() {
        error!("acap-multimux: config load failed, running on defaults: {reason}");
        status.set_config_error(Some(format!("config load: {reason}")));
    }
    let cfg = outcome.into_config();
    info!("acap-multimux: loaded config: {cfg:?}");

    let route_handle = Arc::new(RouteHandle::new(
        cfg.target_duration_secs,
        cfg.part_target_ms,
        cfg.window_segments,
    ));

    spawn_capture_pipeline(&cfg, route_handle.clone(), status.clone());

    // `Config` carries no configurable playlist filename, so this app serves
    // LL-HLS's default media playlist name (`llhls::DEFAULT_PLAYLIST_NAME`,
    // `media.m3u8`) — matching the relative-URI playlists documented below
    // (`/local/acapmultimux/hls/<stream>/media.m3u8`).
    let outputs: Vec<Arc<dyn Output>> = vec![OutputKind::LlHls.build()];
    let mut streams = HashMap::new();
    streams.insert(STREAM_NAME.to_string(), (route_handle, outputs));
    let app_state = Arc::new(AppState::new(streams));

    // AXIS OS's Apache reverse proxy forwards the FULL request path to the
    // target verbatim — it does NOT strip the `/local/<appName>/<apiPath>`
    // prefix (confirmed on hardware, #669, and matches Axis's own C/CivetWeb
    // and axum reverse-proxy examples, which register routes at the full
    // prefixed path). So the app must serve its routes under the real proxied
    // path: `/local/acapmultimux/hls/<stream>/…` and
    // `/local/acapmultimux/admin/…`. The origin's playlists use relative URIs
    // (`media.m3u8`, `seg-*.m4s`), which resolve correctly under the prefix.
    let inner = axum::Router::new()
        .nest("/hls", multimux::origin::router(app_state))
        .merge(admin::admin_router(store, status));
    let app = axum::Router::new().nest(URL_PREFIX, inner);

    let bind_addr = format!("127.0.0.1:{}", cfg.port);
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("acap-multimux: failed to bind {bind_addr}: {e}");
            std::process::exit(1);
        }
    };
    info!("acap-multimux: listening on {bind_addr}");

    if let Err(e) = axum::serve(listener, app).await {
        error!("acap-multimux: axum server error: {e}");
        std::process::exit(1);
    }
}

/// Start the VDO capture -> LL-HLS segmentation pipeline on its own OS thread
/// with its own `current_thread` tokio runtime (see the module doc's
/// "Threading" section): [`multimux::supervise_driver`] driving
/// [`run_vdo_capture`], forever, retrying with backoff on failure. Never
/// observes shutdown (`_keep_alive` is held for the thread's whole lifetime
/// so the paired `watch::Receiver` never errors) — this app has no graceful
/// shutdown concept today, matching the pre-port behaviour (the process was
/// simply killed to stop it).
fn spawn_capture_pipeline(
    cfg: &admin::Config,
    route_handle: Arc<RouteHandle>,
    status: StatusHandle,
) {
    let codec = if cfg.codec == "h265" {
        Codec::H265
    } else {
        Codec::H264
    };
    let channel = cfg.channel;
    let width = cfg.width;
    let height = cfg.height;
    let framerate = cfg.framerate;
    let window_segments = cfg.window_segments;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for the VDO capture pipeline");
        rt.block_on(async move {
            let (_keep_alive, shutdown_rx) = tokio::sync::watch::channel(false);
            supervise_driver_forever(
                codec,
                channel,
                width,
                height,
                framerate,
                window_segments,
                status,
                route_handle,
                shutdown_rx,
            )
            .await;
        });
    });
}

/// Wraps [`multimux::supervise_driver`] over [`run_vdo_capture`] — pulled out
/// of [`spawn_capture_pipeline`] only so the parameters `run_vdo_capture`'s
/// closure captures are named once, not because this does anything
/// `supervise_driver` doesn't already do on its own.
#[allow(clippy::too_many_arguments)]
async fn supervise_driver_forever(
    codec: Codec,
    channel: u32,
    width: u32,
    height: u32,
    framerate: u32,
    window_segments: usize,
    status: StatusHandle,
    route_handle: Arc<RouteHandle>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    multimux::supervise_driver(
        move |route_handle| {
            let status = status.clone();
            async move {
                run_vdo_capture(
                    codec,
                    channel,
                    width,
                    height,
                    framerate,
                    window_segments,
                    &status,
                    &route_handle,
                )
                .await
            }
        },
        route_handle,
        Backoff::production_default(),
        STREAM_NAME.to_string(),
        shutdown_rx,
    )
    .await;
}

/// One VDO-capture attempt — the closure [`multimux::supervise_driver`]
/// retries with backoff. Opens the VDO channel
/// ([`VdoIngestSession::new`]), wraps it in an
/// [`media_plane::ingress::IngestDriver`] (no `Dialer`: see `vdo_source`'s
/// own doc for why VDO drives directly), then loops `feed`/`advance_route`
/// forever — each `feed` blocks on the next VDO buffer (see `vdo_source`'s
/// module doc's "Threading" section) — until the session's health leaves
/// [`media_plane::ingress::HealthState::is_running`] (a VDO read/convert
/// failure; a live camera channel has no natural clean end).
async fn run_vdo_capture(
    codec: Codec,
    channel: u32,
    width: u32,
    height: u32,
    framerate: u32,
    window_segments: usize,
    status: &StatusHandle,
    route_handle: &RouteHandle,
) -> multimux::Result<()> {
    let session = VdoIngestSession::new(codec, channel, width, height, framerate).map_err(|e| {
        MultimuxError::Connect {
            reason: format!("VdoIngestSession init failed: {e}"),
        }
    })?;

    let trunk_config = TrunkConfig::new(
        source_nz(DRIVER_TIMED_CAPACITY),
        source_nz(DRIVER_SPARSE_CAPACITY),
        source_nz(window_segments),
        source_nz(DRIVER_EVENT_CAPACITY),
        source_nz(DRIVER_PART_CAPACITY),
    );
    // VDO capture has no network handshake to bound: `VdoIngestSession::new`
    // already resolved everything synchronously, so `Established` is queued
    // before the very first `feed()` call even runs and is drained (promoting
    // out of `Establishing`) before this driver's handshake deadline is ever
    // checked — see `vdo_source`'s own module doc. `u64::MAX` documents "this
    // deadline is unreachable in practice" rather than picking an arbitrary
    // real timeout for a step that can't actually time out.
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let mut progress = DriverProgress::new();
    let start = Instant::now();

    status.set_running(true);
    status.set_last_error(None);

    loop {
        let now = Timestamp::from_instant(start, Instant::now());
        driver.feed((), now);
        advance_route(&driver, route_handle, &mut progress);
        // Issue #955: `StatusHandle` was never touched by the pipeline, so
        // `/admin/status` reported `current_segment`/`current_part`/`frames`
        // as permanent zeros while segments were being served correctly —
        // confirmed on-device (`#EXT-X-MEDIA-SEQUENCE` climbing while
        // `/admin/status` stood still). One VDO buffer is processed per
        // `feed()` call (see this function's own module doc), so counting
        // one here is the natural unit for "frames processed". The
        // segment/part position comes straight from this program's `Trunk`
        // — `last_closed_segment` names the newest *closed* segment, so the
        // one "currently being written" (this field's documented meaning)
        // is one past it, or `0` before anything has closed yet; the part
        // count is how many parts that open segment has accumulated so far.
        status.add_frames(1);
        if let Some(program) = driver.programs().next() {
            if let Some(trunk) = driver.trunk(program) {
                let current_segment = trunk.last_closed_segment().map_or(0, |seq| seq + 1);
                let current_part = trunk.parts_in_segment(current_segment).len() as u32;
                status.set_position(current_segment, current_part);
            }
        }
        if !driver.health().is_running() {
            break;
        }
    }

    status.set_running(false);
    match driver.into_health() {
        media_plane::ingress::HealthState::Failed(e) => {
            let reason = e.to_string();
            status.set_last_error(Some(reason.clone()));
            error!("acap-multimux: VDO capture ended: {reason}");
            Err(MultimuxError::Connect { reason })
        }
        // A live camera channel has no natural clean end, but `Stage::finish`
        // is never called by this loop either, so this arm is unreached in
        // practice; treated as a clean stop rather than manufacturing an
        // error for a state this driver never actually produces on its own.
        _ => Ok(()),
    }
}

/// `NonZeroUsize::new(n).unwrap_or(MIN)` — every capacity this module passes
/// to [`TrunkConfig::new`] is a fixed positive constant except
/// `window_segments`, which `Config::validate` (`src/admin.rs`) already
/// rejects as `0` via the admin API; this is a second, structural backstop
/// (degrading to capacity 1 rather than panicking) for that one
/// caller-configurable value, not the primary guard.
fn source_nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).unwrap_or(std::num::NonZeroUsize::MIN)
}
