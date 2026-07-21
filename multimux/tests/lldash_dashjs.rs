//! Issue #721 acceptance: headless **dash.js** low-latency playback against
//! a real multimux LL-DASH origin -- the gate that decides whether the
//! true chunked-transfer design (`crate::output::ll_dash`'s module docs)
//! actually works for a real player, not just that its MPD parses.
//!
//! # What this proves that unit tests can't
//!
//! `multimux/src/output/ll_dash.rs` and `multimux/src/origin/resource.rs`'s
//! own unit/integration tests prove the MPD is well-formed and that the
//! chunked-transfer handler streams the right bytes in the right order. They
//! do **not** prove a real low-latency DASH client can actually play the
//! result: dash.js has to fetch the manifest, recognize the LL signalling,
//! open a `MediaSource`, incrementally append the streamed (not-yet-closed)
//! segment's bytes to a `SourceBuffer` as they arrive over chunked transfer,
//! and decode real H.264 -- any mismatch in the CMAF fragment shape (see
//! `output::ll_dash`'s module docs on the missing leading `styp`) would show
//! up here as a stalled/erroring player, not a Rust-side assertion failure.
//!
//! # Real fixture, not synthetic bytes
//!
//! Reuses the exact same real, ffmpeg-encoded fixture and live-paced
//! producer shape as `ll-hls-runtime/tests/golden_gate.rs`
//! (`fixtures/ts/h264_aac.ts`, 320x240 Main-profile H.264 @ 25 fps, 3.0 s /
//! 75 frames, demuxed via `TsDemux`) fed through the same real
//! `transmux::ll_hls::LlHlsSegmenter` that feeds the shared `MediaStore` in
//! production -- so a real browser must genuinely decode real video, not an
//! opaque placeholder.
//!
//! # Skip-clean discipline
//!
//! Mirrors `ll-hls-runtime/tests/golden_gate.rs`/`glass_to_glass.rs`: every
//! case skips (printing why) rather than failing when `node`, the vendored
//! `tests/assets/dash.all.min.js`, or the installed
//! `tests/assets/node_modules/playwright` aren't present -- `cargo nextest`
//! stays green on a fresh clone (before `bun install`/`npm install` has been
//! run in `tests/assets/`) or in a CI image without Node. A case that does
//! run asserts against the **real measured** dash.js result (`currentTime`
//! advanced, a finite live-latency sample was observed, no fatal `ERROR`
//! event) -- never just "the child process exited 0".

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use multimux::origin::{AppState, router};
use multimux::output::Output as MmOutput;
use multimux::output::ll_dash::LlDashOutput;
use multimux::store::MediaStore;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{CodecConfig, Sample, TrackSpec, TsDemux};

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 120;
const WINDOW_SEGMENTS: usize = 8;

/// A live playback advance this far in must have crossed at least one
/// segment boundary (`TARGET_DURATION_SECS` == 1.0 s) -- proving continuous
/// playback across the chunked-transfer -> closed-segment transition, not
/// just the first in-progress segment.
const MIN_CURRENT_TIME_SECS: f64 = 1.8;

/// Generous bound: 3 s of real-time-paced content (40 ms/frame * 75 frames)
/// plus dash.js startup/buffering overhead.
const HARNESS_TIMEOUT_MS: u64 = 20_000;

fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
}

fn assets_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/assets"))
}

macro_rules! skip_unless {
    ($cond:expr, $why:expr) => {
        if !$cond {
            eprintln!("SKIP lldash_dashjs: {}", $why);
            return;
        }
    };
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn harness_ready() -> bool {
    assets_dir().join("dash.all.min.js").is_file()
        && assets_dir().join("node_modules/playwright").is_dir()
}

/// Demux `h264_aac.ts`'s real AVC video track -- same fixture/shape
/// `ll-hls-runtime/tests/golden_gate.rs` uses, forced onto
/// `ll_hls_runtime::server::DEFAULT_TRACK_ID` so the shared store's
/// filenames (`init-1.mp4`/`seg-1-<N>.m4s`) match what the LL-DASH
/// `SegmentTemplate` addresses.
fn real_video_track_and_samples() -> (TrackSpec, Vec<Sample>) {
    let ts = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
    let media = TsDemux::new().demux(&ts).expect("demux h264_aac.ts");
    let video = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("h264_aac.ts must carry an AVC video track");
    let mut spec = video.spec;
    spec.track_id = ll_hls_runtime::server::DEFAULT_TRACK_ID;
    (spec, video.samples)
}

/// Feed every sample in `samples` into `store` via a real
/// `LlHlsSegmenter`, paced at the fixture's own 25 fps (40 ms/frame) --
/// mirrors `golden_gate.rs`'s `run_live_producer` exactly: the shared store
/// this feeds is the same one `crate::output::ll_dash`/
/// `crate::origin::resource`'s chunked-transfer path reads from in
/// production.
async fn run_live_producer(store: Arc<MediaStore>, spec: TrackSpec, samples: Vec<Sample>) {
    let track_id = spec.track_id;
    let movie_timescale = spec.timescale;
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![spec],
        movie_timescale,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(seg.init_segment().expect("init segment builds"));

    let frame_interval = Duration::from_millis(40);
    for sample in samples {
        seg.push(track_id, sample).expect("push succeeds");
        for part in seg.take_ready_parts() {
            store.add_part(part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(segment);
        }
        tokio::time::sleep(frame_interval).await;
    }
    seg.flush().expect("flush succeeds");
    for part in seg.take_ready_parts() {
        store.add_part(part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(segment);
    }
}

/// Start the real `multimux` LL-DASH origin on an ephemeral loopback port,
/// serving one stream (`live`) with only `LlDashOutput` enabled --
/// `manifest-ll.mpd` is the one URL this test's player fetches.
async fn start_ll_dash_origin(
    spec: TrackSpec,
) -> (Arc<MediaStore>, String, tokio::task::JoinHandle<()>) {
    let store = Arc::new(MediaStore::new(
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
        WINDOW_SEGMENTS,
    ));
    // The LL-DASH manifest handler 503s until track specs are known (issue
    // #663 P4.2) -- set it up front, before the producer's own real-time
    // pacing, so the manifest is servable the instant the player asks.
    store.set_track_specs(vec![spec]);

    let mut streams = HashMap::new();
    streams.insert(
        "live".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlDashOutput) as Arc<dyn MmOutput>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("listener has a local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum server");
    });
    (store, format!("http://{addr}/live/manifest-ll.mpd"), server)
}

/// The measured result `tests/assets/check_lldash.mjs` prints as JSON.
#[derive(Debug, serde::Deserialize)]
struct DashJsResult {
    ok: bool,
    reason: Option<String>,
    #[serde(rename = "currentTime")]
    current_time: f64,
    #[serde(rename = "liveLatencySamples")]
    live_latency_samples: Vec<f64>,
    #[serde(rename = "fatalError")]
    fatal_error: Option<String>,
}

/// Shell out to the node/Playwright harness, returning its parsed JSON
/// result. Panics (a genuine test failure, not a skip) if the child process
/// itself couldn't be run or didn't print valid JSON -- by the time this is
/// called, [`harness_ready`]/[`node_available`] have already confirmed the
/// prerequisites are in place, so a failure here is the harness itself
/// misbehaving, not an environment gap.
fn run_dashjs_check(manifest_url: &str) -> DashJsResult {
    let output = Command::new("node")
        .arg(assets_dir().join("check_lldash.mjs"))
        .arg(manifest_url)
        .arg(MIN_CURRENT_TIME_SECS.to_string())
        .arg(HARNESS_TIMEOUT_MS.to_string())
        .current_dir(assets_dir())
        .output()
        .expect("spawn node check_lldash.mjs");

    if !output.stderr.is_empty() {
        eprintln!(
            "[check_lldash.mjs stderr]\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        output.status.success(),
        "check_lldash.mjs must exit 0 (a measured pass/fail is still exit 0 -- only a \
         harness-level failure, e.g. browser launch failure, exits non-zero): {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("check_lldash.mjs must print one JSON object: {e}\nstdout: {stdout}")
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashjs_low_latency_playback_against_real_ll_dash_origin() {
    skip_unless!(node_available(), "node not on PATH");
    skip_unless!(
        harness_ready(),
        "tests/assets/dash.all.min.js or tests/assets/node_modules/playwright missing -- \
         run `bun install` (or `npm install`) in multimux/tests/assets/ first"
    );

    let (spec, fed_samples) = real_video_track_and_samples();
    assert!(
        !fed_samples.is_empty(),
        "h264_aac.ts must demux to at least one video sample"
    );

    let (store, manifest_url, server) = start_ll_dash_origin(spec.clone()).await;
    let producer = tokio::spawn(run_live_producer(store, spec, fed_samples));

    // Run the headless dash.js check on a blocking thread (it shells out and
    // blocks on the child process) while the producer keeps feeding the
    // store concurrently, exactly as a real live route would.
    let manifest_url_for_check = manifest_url.clone();
    let result = tokio::task::spawn_blocking(move || run_dashjs_check(&manifest_url_for_check))
        .await
        .expect("dash.js check task must not panic");

    producer.await.expect("producer task must not panic");
    server.abort();

    eprintln!(
        "lldash_dashjs result: currentTime={:.3}s liveLatencySamples={:?} fatalError={:?} \
         reason={:?} (segment target = {TARGET_DURATION_SECS}s)",
        result.current_time, result.live_latency_samples, result.fatal_error, result.reason,
    );

    assert!(
        result.fatal_error.is_none(),
        "dash.js must report no fatal ERROR event: {:?}",
        result.fatal_error
    );
    assert!(
        result.current_time >= MIN_CURRENT_TIME_SECS,
        "video.currentTime must advance past {MIN_CURRENT_TIME_SECS}s (real playback, not \
         just manifest parse): got {}",
        result.current_time
    );
    assert!(
        !result.live_latency_samples.is_empty(),
        "must observe at least one finite getCurrentLiveLatency() sample"
    );
    let min_latency = result
        .live_latency_samples
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    assert!(
        min_latency < TARGET_DURATION_SECS,
        "measured live latency ({min_latency}s) must be below the whole-segment target \
         ({TARGET_DURATION_SECS}s) -- proves chunked (not whole-segment) availability: \
         samples={:?}",
        result.live_latency_samples
    );
    assert!(
        result.ok,
        "check_lldash.mjs's own success criteria must hold: {:?}",
        result.reason
    );
}
