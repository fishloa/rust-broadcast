//! Issue #717 slice 5 acceptance test: `TokioClient` driving a REAL LL-HLS
//! origin over real loopback HTTP, proving sub-second glass-to-glass
//! latency — the epic's headline acceptance bar — plus blocking-reload and
//! preload-hint prefetch actually being exercised (not just parsed), plus a
//! non-LL origin still playing via the full-segment fallback.
//!
//! # Which origin
//!
//! The LL-HLS half of this file reuses `multimux`'s real, production
//! `MediaStore` + `LlHlsOutput` + `origin::router` (the same axum origin
//! shipped in `multimux`, added as a dev-dependency here) — not a test
//! double — driven by a hand-written, real-time-paced producer loop (a
//! `transmux::ll_hls::LlHlsSegmenter` fed one sample every ~33ms, mirroring
//! `multimux::pipeline::run_pipeline`'s own segmenter->store wiring, but with
//! genuine wall-clock pacing between samples so the origin is *live-shaped*
//! rather than dumping every part into the store instantly). No dev-dep
//! cycle: `multimux` does not depend on `ll-hls-runtime`.
//!
//! The non-LL half uses a minimal hand-built axum app instead of
//! `multimux`'s origin, since `multimux::output::llhls::LlHlsOutput` always
//! advertises LL-HLS `#EXT-X-SERVER-CONTROL`/`#EXT-X-PART-INF` — there is no
//! way to make it render a playlist with `low_latency: None`.
//!
//! # Measuring glass-to-glass
//!
//! Each synthetic sample's payload carries its own push wall-clock time
//! (nanoseconds since `UNIX_EPOCH`, big-endian) — `transmux` never
//! interprets sample bytes, so this rides through mux/demux byte-identical.
//! On the client side, `glass_to_glass = now() - decoded_push_time` for
//! every sample the client emits; the assertion is on the **max** observed
//! across the whole run.

#![cfg(feature = "tokio")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;

use ll_hls_runtime::client::Output;
use ll_hls_runtime::client::tokio_client::TokioClient;
use multimux::origin::{AppState, router};
use multimux::output::Output as MmOutput;
use multimux::output::llhls::LlHlsOutput;
use multimux::store::MediaStore;
use transmux::hls::{MapTag, MediaPlaylist, MediaSegment};
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, Sample,
    TrackSpec,
};

/// Matches `multimux::output::llhls::DEFAULT_TRACK_ID` (the single track the
/// origin renders under `part-1-*`/`seg-1-*`/`init-1.mp4` filenames).
const TRACK_ID: u32 = 1;
/// Matches `multimux::pipeline`'s own fixed CMAF movie timescale.
const MOVIE_TIMESCALE: u32 = 90_000;
const VIDEO_TIMESCALE: u32 = 90_000;
const FPS: u32 = 30;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / FPS;
/// 3 seconds of synthetic live video — enough to cross several part/segment
/// boundaries at the timings below while keeping the test's wall-clock cost
/// to a few seconds.
const FRAME_COUNT: u32 = 90;
/// A sync (IDR) sample twice a second.
const SYNC_EVERY: u32 = 15;
/// Small part target so parts/blocking-reload/preload-hint all cycle
/// quickly relative to the 1s test-suite budget this needs to stay under.
const PART_TARGET_MS: u32 = 120;
const TARGET_DURATION_SECS: f64 = 1.0;
const WINDOW_SEGMENTS: usize = 8;

fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

fn video_track() -> TrackSpec {
    TrackSpec::new(
        TRACK_ID,
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    )
}

/// Build a sample whose payload's first 8 bytes are `now()` (nanoseconds
/// since `UNIX_EPOCH`, big-endian) — a wall-clock stopwatch riding through
/// the mux/demux pipeline as opaque coded bytes.
fn timestamped_sample(is_sync: bool) -> Sample {
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_nanos() as u64;
    let mut data = now_ns.to_be_bytes().to_vec();
    data.extend_from_slice(&[0xAB; 16]);
    Sample::new(data, FRAME_DUR, is_sync, 0)
}

/// The inverse of [`timestamped_sample`]: how long ago (from `now()`) this
/// sample's payload says it was pushed.
fn glass_to_glass(sample: &Sample) -> Duration {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&sample.data[..8]);
    let pushed_ns = u64::from_be_bytes(buf);
    let pushed = UNIX_EPOCH + Duration::from_nanos(pushed_ns);
    SystemTime::now()
        .duration_since(pushed)
        .unwrap_or(Duration::ZERO)
}

/// Feed [`FRAME_COUNT`] samples into `store` via a real
/// `transmux::ll_hls::LlHlsSegmenter`, real-time-paced at [`FPS`] frames/sec
/// — a live-shaped producer, not a batch dump.
async fn run_live_producer(store: Arc<MediaStore>) {
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        MOVIE_TIMESCALE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(seg.init_segment().expect("init segment builds"));

    let frame_interval = Duration::from_millis(1000 / u64::from(FPS));
    for i in 0..FRAME_COUNT {
        let is_sync = i % SYNC_EVERY == 0;
        seg.push(TRACK_ID, timestamped_sample(is_sync))
            .expect("push succeeds");
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

/// Start the real `multimux` LL-HLS origin on an ephemeral loopback port,
/// serving one stream named `live`.
async fn start_ll_origin() -> (Arc<MediaStore>, String, tokio::task::JoinHandle<()>) {
    let store = Arc::new(MediaStore::new(
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
        WINDOW_SEGMENTS,
    ));
    let mut streams = HashMap::new();
    streams.insert(
        "live".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn MmOutput>],
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
    (store, format!("http://{addr}/live/media.m3u8"), server)
}

// ===========================================================================
// The epic's headline acceptance: sub-second glass-to-glass over loopback,
// blocking reload + preload-hint prefetch actually exercised.
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn glass_to_glass_sub_second_over_loopback_with_blocking_reload_and_prefetch() {
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        let (store, playlist_url, server) = start_ll_origin().await;
        let producer = tokio::spawn(run_live_producer(store));

        let mut client = TokioClient::new(&playlist_url).expect("client builds");
        let mut max_latency = Duration::ZERO;
        let mut sample_count = 0usize;

        while sample_count < FRAME_COUNT as usize {
            match client.next_output().await.expect("client must not error") {
                Some(Output::Samples { samples, .. }) => {
                    for s in &samples {
                        let latency = glass_to_glass(s);
                        if latency > max_latency {
                            max_latency = latency;
                        }
                        sample_count += 1;
                    }
                }
                Some(Output::EndOfStream) | None => break,
                Some(_other) => {}
            }
        }

        producer.await.expect("producer task must not panic");
        server.abort();

        (sample_count, max_latency, client.stats())
    })
    .await
    .expect("glass-to-glass test must complete within 25s (deadlock/hang guard)");

    let (sample_count, max_latency, stats) = result;

    assert_eq!(
        sample_count, FRAME_COUNT as usize,
        "must reconstruct every sample the producer pushed, no gaps/duplicates"
    );
    assert!(
        max_latency < Duration::from_secs(1),
        "glass-to-glass latency must be sub-second (issue #717 headline acceptance): {max_latency:?}"
    );

    assert!(
        stats.blocking_reloads > 0,
        "must have issued at least one Blocking Playlist Reload (_HLS_msn/_HLS_part): {stats:?}"
    );
    assert!(
        stats.preload_hint_resource_fetches > 0,
        "must have fetched at least one part via its #EXT-X-PRELOAD-HINT, ahead of its own \
         numbered #EXT-X-PART appearance: {stats:?}"
    );

    eprintln!(
        "glass-to-glass: {sample_count} samples, max latency {max_latency:?}, stats {stats:?}"
    );
}

// ===========================================================================
// Non-LL origin: full-segment fallback, over real HTTP via the same
// `TokioClient` (proving the *adapter*, not just the sans-IO core, handles
// it — `ll-hls-runtime/tests/origin_loop.rs` already proves the core alone).
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_ll_origin_plays_via_full_segment_fallback_over_http() {
    tokio::time::timeout(Duration::from_secs(15), async {
        // target_duration_secs=1.0 (matching FRAME_COUNT=30 @ 30fps = 1.0s):
        // the 31st (sync) sample below crosses the target and auto-closes
        // segment 1 at exactly 30 samples, rolling itself into a new segment
        // 2 that only `flush()` (not this test) closes — mirroring
        // `origin_loop.rs`'s own non-LL test's segment-boundary trick, so
        // `seg1` ends up with exactly `fed_samples.len()` samples.
        let mut seg =
            LlHlsSegmenter::with_part_target(vec![video_track()], MOVIE_TIMESCALE, 1.0, 100_000)
                .expect("segmenter builds");
        let mut fed_samples: Vec<Sample> = Vec::new();
        for i in 0..30u32 {
            let s = timestamped_sample(i == 0);
            fed_samples.push(s.clone());
            seg.push(TRACK_ID, s).expect("push succeeds");
        }
        seg.push(TRACK_ID, timestamped_sample(true))
            .expect("push succeeds");
        seg.flush().expect("flush succeeds");
        let init_bytes = seg.init_segment().expect("init segment builds");
        let seg1 = seg
            .take_ready_segments()
            .into_iter()
            .find(|s| s.segment_seq == 1)
            .expect("segment 1 must have closed");

        let playlist = MediaPlaylist {
            version: 3,
            target_duration: 4,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![MediaSegment {
                uri: "seg1.m4s".to_string(),
                duration: seg1.duration,
                discontinuous: false,
                parts: vec![],
                byte_range: None,
                map: Some(MapTag {
                    uri: "init.mp4".to_string(),
                    byte_range: None,
                }),
            }],
            open_segment: None,
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            rendition_reports: vec![],
            skip: None,
        };
        let playlist_text = playlist.to_m3u8();
        assert!(
            !playlist_text.contains("#EXT-X-PART"),
            "the fixture playlist must genuinely carry no PART tags:\n{playlist_text}"
        );

        let init_for_route = init_bytes.clone();
        let seg1_bytes_for_route = seg1.bytes.clone();
        let app = Router::new()
            .route(
                "/media.m3u8",
                get(move || {
                    let text = playlist_text.clone();
                    async move { text.into_response() }
                }),
            )
            .route(
                "/init.mp4",
                get(move || {
                    let bytes = init_for_route.clone();
                    async move { bytes.into_response() }
                }),
            )
            .route(
                "/seg1.m4s",
                get(move || {
                    let bytes = seg1_bytes_for_route.clone();
                    async move { bytes.into_response() }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("listener has a local address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });

        let mut client =
            TokioClient::new(format!("http://{addr}/media.m3u8")).expect("client builds");
        let mut got_samples: Vec<Sample> = Vec::new();
        let mut saw_init = false;
        loop {
            match client.next_output().await.expect("client must not error") {
                Some(Output::Init(bytes)) => {
                    assert_eq!(bytes, init_bytes, "init bytes must match byte-identical");
                    saw_init = true;
                }
                Some(Output::Samples { samples, .. }) => got_samples.extend(samples),
                Some(Output::EndOfStream) | None => break,
                Some(_other) => {}
            }
        }
        server.abort();

        assert!(saw_init, "must emit init even on the fallback path");
        assert_eq!(got_samples.len(), fed_samples.len());
        for (got, want) in got_samples.iter().zip(fed_samples.iter()) {
            assert_eq!(got.data, want.data, "sample bytes must round-trip exactly");
        }

        let stats = client.stats();
        assert_eq!(
            stats.blocking_reloads, 0,
            "a non-LL playlist (no PART tags) must never trigger a blocking reload: {stats:?}"
        );
        assert!(
            stats.resource_fetches >= 2,
            "must fetch init + the whole segment: {stats:?}"
        );
    })
    .await
    .expect("non-LL fallback test must complete within 15s (deadlock/hang guard)");
}
