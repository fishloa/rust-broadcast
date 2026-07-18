//! Serve a synthetic LL-HLS stream with no RTSP source and no network
//! dependency, for trying the origin without a real camera.
//!
//! Builds enough synthetic H.264-shaped samples to close a few full
//! segments, drives them through the real [`transmux::ll_hls::LlHlsSegmenter`]
//! (via [`multimux::pipeline::run_pipeline`]) into a
//! [`multimux::store::MediaStore`], then serves that store's single "cam"
//! route under the real axum [`multimux::origin::router`] (via the default
//! [`multimux::output::llhls::LlHlsOutput`]) on an ephemeral localhost port.
//! Once the mock source reaches end-of-stream the store's contents are fixed
//! — the HTTP origin keeps serving that fixed window.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example serve_mock
//! # then, in another terminal:
//! curl http://127.0.0.1:<port>/cam/master.m3u8
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::pipeline::{MockSource, run_pipeline};
use multimux::store::MediaStore;
use transmux::avc_config_from_sprop;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

/// A real-ish `sprop-parameter-sets` pair (SPS+PPS) — the same one used by
/// multimux's own tests — decoded into a genuine `avcC` configuration record.
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

/// 90 kHz video timescale (the CMAF movie timescale `run_pipeline` builds
/// with) — 1/30 s per access unit at 30 fps.
const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
/// Synthetic frame count: 4 s @ 30 fps, comfortably over the target duration
/// below so several segments/parts close.
const FRAME_COUNT: u32 = 120;
/// A sync (IDR) sample once per second at 30 fps.
const SYNC_INTERVAL_FRAMES: u32 = 30;

/// Target full-segment duration, in seconds.
const TARGET_DURATION_SECS: f64 = 1.0;
/// LL-HLS part target, in milliseconds.
const PART_TARGET_MS: u32 = 500;
/// Rolling window depth: full segments retained in RAM.
const WINDOW_SEGMENTS: usize = 8;
/// Served stream name (the URL path segment).
const STREAM_NAME: &str = "cam";

#[tokio::main]
async fn main() {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    let specs = vec![TrackSpec::new(
        1,
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 0,
            height: 0,
        },
    )];

    let mut batches = Vec::new();
    for i in 0..FRAME_COUNT {
        let is_sync = i % SYNC_INTERVAL_FRAMES == 0;
        let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 64];
        let sample = Sample::new(data, FRAME_DUR, is_sync, 0);
        batches.push(vec![(1u32, sample)]);
    }

    let store = Arc::new(MediaStore::new(
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
        WINDOW_SEGMENTS,
    ));
    let source = MockSource::new(specs, batches);
    run_pipeline(
        store.clone(),
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
        source,
        STREAM_NAME,
    )
    .await
    .expect("mock pipeline runs to completion");

    let mut streams = HashMap::new();
    streams.insert(
        STREAM_NAME.to_string(),
        (store, vec![Arc::new(LlHlsOutput) as Arc<dyn Output>]),
    );
    let app = router(Arc::new(AppState::new(streams)));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral localhost port");
    let addr = listener.local_addr().expect("listener has a local address");
    println!("multimux serve_mock: http://{addr}/{STREAM_NAME}/master.m3u8");

    axum::serve(listener, app).await.expect("axum server");
}
