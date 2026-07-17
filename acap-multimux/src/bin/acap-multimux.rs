//! ACAP entrypoint (`device`-gated; only builds inside the Axis ACAP Native
//! SDK sysroot). Wires the real capture -> LL-HLS pipeline together:
//!
//! - Loads [`acap_multimux::admin::Config`] from the ACAP
//!   `axparameter`-backed [`acap_multimux::admin::AxParameterStore`].
//! - Builds a [`multimux::store::StreamStore`] sized from the config's LL-HLS
//!   tuning (target segment duration / part target / window).
//! - Starts [`acap_multimux::vdo_source::VdoSource`] and drives it through
//!   [`multimux::pipeline::run_pipeline`] on a **dedicated OS thread with its
//!   own current-thread tokio runtime** — see "Threading" below.
//! - Serves the LL-HLS origin (`multimux::origin::router`) nested under
//!   `/hls`, merged with the admin config/status routes
//!   (`acap_multimux::admin::admin_router`), on `127.0.0.1:<port>` (matching
//!   `manifest.json`'s `reverseProxy` targets).
//!
//! # Threading
//!
//! [`VdoSource::next_samples`](acap_multimux::vdo_source::VdoSource)
//! ultimately calls `vdo::RunningStream::next_buffer`, a **blocking** FFI call
//! into `libvdo.so` that only returns once the camera has produced the next
//! frame (see `vdo_source.rs`'s module doc). Running that on an axum worker
//! thread would eventually starve every request being served on the same
//! `rt-multi-thread` runtime once all worker threads happen to be parked in
//! that blocking call. Instead the whole capture/segment/store pipeline runs
//! on a plain `std::thread::spawn`'d OS thread with its own
//! `current_thread` tokio runtime — the blocking call only ever stalls that
//! one dedicated thread, never axum's.
use std::collections::HashMap;
use std::sync::Arc;

use acap_multimux::admin::{self, AxParameterStore, ConfigStore, StatusHandle};
use acap_multimux::convert::Codec;
use acap_multimux::vdo_source::VdoSource;
use log::{error, info};
use multimux::origin::AppState;
use multimux::store::StreamStore;

/// The single served stream's name in LL-HLS URLs
/// (`/hls/<STREAM_NAME>/media.m3u8`) — this app captures exactly one VDO
/// channel per `Config`, so one fixed stream name is enough.
const STREAM_NAME: &str = "cam";

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
    let cfg = store.load();
    info!("acap-multimux: loaded config: {cfg:?}");

    let stream_store = Arc::new(StreamStore::new(
        cfg.target_duration_secs,
        cfg.part_target_ms,
        cfg.window_segments,
    ));

    let status = StatusHandle::new();

    spawn_capture_pipeline(&cfg, stream_store.clone(), status.clone());

    let mut streams = HashMap::new();
    streams.insert(STREAM_NAME.to_string(), stream_store);
    let app_state = Arc::new(AppState { streams });

    // `/hls` prefix per manifest.json's `reverseProxy` `apiPath: "hls"` entry
    // (hardware-verify note: confirm whether the camera's reverse proxy
    // strips the `apiPath` segment or forwards it verbatim — Task 7 checks
    // this against a real device and adjusts either side if it doesn't
    // match).
    let app = axum::Router::new()
        .nest("/hls", multimux::origin::router(app_state))
        .merge(admin::admin_router(store, status));

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
/// "Threading" section). Logs and updates `status` on either a `VdoSource`
/// init failure or a `run_pipeline` error; either way the thread ends without
/// taking the HTTP origin down (only that camera channel's segments stop
/// advancing).
fn spawn_capture_pipeline(cfg: &admin::Config, store: Arc<StreamStore>, status: StatusHandle) {
    let codec = if cfg.codec == "h265" {
        Codec::H265
    } else {
        Codec::H264
    };
    let channel = cfg.channel;
    let width = cfg.width;
    let height = cfg.height;
    let framerate = cfg.framerate;
    let target_duration_secs = cfg.target_duration_secs;
    let part_target_ms = cfg.part_target_ms;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for the VDO capture pipeline");
        rt.block_on(async move {
            match VdoSource::new(codec, channel, width, height, framerate) {
                Ok(src) => {
                    status.set_running(true);
                    if let Err(e) = multimux::pipeline::run_pipeline(
                        store,
                        target_duration_secs,
                        part_target_ms,
                        src,
                    )
                    .await
                    {
                        error!("acap-multimux: pipeline ended: {e}");
                        status.set_last_error(Some(e.to_string()));
                    }
                    status.set_running(false);
                }
                Err(e) => {
                    error!("acap-multimux: VdoSource init failed: {e}");
                    status.set_last_error(Some(e.to_string()));
                }
            }
        });
    });
}
