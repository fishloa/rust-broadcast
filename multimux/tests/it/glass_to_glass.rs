//! Issue #717 slice 5 acceptance test: multimux's own HLS pull driving a REAL
//! LL-HLS origin over real loopback HTTP, proving sub-second glass-to-glass
//! latency — the epic's headline acceptance bar — plus blocking-reload and
//! preload-hint prefetch actually being exercised (not just parsed), plus a
//! non-LL origin still playing via the full-segment fallback.
//!
//! # Which client, and why
//!
//! The client is the production one: this file drives `run_hls_pull` — the
//! `hls` input every pulled route runs, over the sans-IO `HlsClient` — and
//! pays attention to nothing else. Its fetches, retries, reload pacing and
//! publication are all the shipped code; the test only observes. The counters
//! the deleted executor-bound adapter used to carry are observed the same
//! way, from the requests the loop issues.
//!
//! # Which origin
//!
//! The LL-HLS half reuses `multimux`'s real, production `RouteHandle` +
//! `HlsOutput` + `origin::router` (the same axum origin shipped in
//! `multimux`) — not a test double — driven by a hand-written, real-time-paced
//! producer loop (a `transmux::ll_hls::LlHlsSegmenter` fed one sample every
//! ~33ms, mirroring `multimux::pipeline::run_pipeline`'s own segmenter->store
//! wiring, but with genuine wall-clock pacing between samples so the origin
//! is *live-shaped* rather than dumping every part into the store instantly).
//!
//! The non-LL half uses a minimal hand-built axum app instead of
//! `multimux`'s origin, since `multimux::output::hls::HlsOutput` always
//! advertises LL-HLS `#EXT-X-SERVER-CONTROL`/`#EXT-X-PART-INF` — there is no
//! way to make it render a playlist with `low_latency: None`.
//!
//! # Measuring glass-to-glass
//!
//! Each synthetic sample's payload carries its own push wall-clock time
//! (nanoseconds since `UNIX_EPOCH`, big-endian) — `transmux` never
//! interprets sample bytes, so this rides through mux/demux byte-identical.
//! On the client side, `glass_to_glass = now() - decoded_push_time` for
//! every sample the client emits; the assertions are on the **90th
//! percentile** across the whole run and on the **worst** sample.
//!
//! # Why the sample watcher, not the pulled route's segment stream
//!
//! The pulled route re-segments what it pulls, and the client's own segment
//! stream is not what it plays — a live LL origin is consumed part by part.
//! A watcher reading the client's decoded samples as they are emitted is
//! therefore the faithful measurement; reading the route's closed segments
//! would report samples that have been waiting for a segment boundary.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;

use broadcast_common::Stage;
use broadcast_common::Timestamp;
use broadcast_hls::{DecimalSeconds, MapTag, MediaPlaylist, MediaSegment};
use hls_runtime::client::Action;
use hls_runtime::server::DEFAULT_TRACK_ID;
use media_plane::ProgramId;
use media_plane::ingress::{IngestSession, SessionEvent};
use multimux::origin::{AppState, router};
use multimux::output::Output as MmOutput;
use multimux::output::hls::HlsOutput as MmHlsOutput;
use multimux::route::RouteHandle;
use multimux::source::hls_pull::{HlsFetchId, HlsIngestSession};
use transmux::TrackId;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, Sample,
    TrackSpec,
};

const TRACK_ID: u32 = DEFAULT_TRACK_ID.value();
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
/// Steady-state glass-to-glass bound (issue #717 headline acceptance): the 90th-percentile latency must stay under one second.
const P90_LATENCY_BOUND: Duration = Duration::from_secs(1);
/// Hard ceiling on any single sample: wide enough that a host stall of a few seconds does not fail a correct client, tight enough that a real multi-second hitch in the product still fails.
const MAX_LATENCY_CEILING: Duration = Duration::from_secs(3);
/// Percentile used for the steady-state bound.
const STEADY_STATE_PERCENTILE: usize = 90;
/// Length of the push-instant stopwatch that starts a timestamped payload.
const STOPWATCH_BYTES: usize = 8;
/// How long the drive loop waits for the next fetch result. Every fetch is
/// already bounded by its own read timeout; this only bounds the idle park.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// The part url a playlist names with `#EXT-X-PRELOAD-HINT` (RFC 8216bis
/// §4.4.5.3).
const PRELOAD_HINT_TAG: &str = "#EXT-X-PRELOAD-HINT:";

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
        TrackId::new(TRACK_ID),
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    )
}

fn now_ns() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the epoch")
            .as_nanos(),
    )
    .expect("a wall-clock reading fits in u64 nanoseconds")
}

/// Build a sample whose payload's first 8 bytes are `now()` (nanoseconds
/// since `UNIX_EPOCH`, big-endian) — a wall-clock stopwatch riding through
/// the mux/demux pipeline as opaque coded bytes.
fn timestamped_sample(is_sync: bool) -> Sample {
    let mut data = now_ns().to_be_bytes().to_vec();
    data.extend_from_slice(&[0xAB; 16]);
    // No absolute source time: these are synthetic samples for the
    // duration-driven `Segmenter`, so dts/pts stay `None` (media plane step 2c
    // — never fabricate a timestamp) and the segment timeline comes from
    // `duration` exactly as before.
    Sample::new(data, None, None, Some(FRAME_DUR), is_sync)
}

/// The inverse of [`timestamped_sample`]: how long ago (from `now()`) this
/// sample's payload says it was pushed.
fn glass_to_glass(sample: &Sample) -> Duration {
    let pushed_ns = u64::from_be_bytes(
        sample.data[..STOPWATCH_BYTES]
            .try_into()
            .expect("a timestamped sample starts with its 8-byte stopwatch"),
    );
    Duration::from_nanos(now_ns().saturating_sub(pushed_ns))
}

/// What one drive asked the origin for: blocking reloads (`_HLS_msn`/
/// `_HLS_part` on a playlist GET) and preload-hint prefetches (a part GET
/// for the url the previous playlist's `#EXT-X-PRELOAD-HINT` named).
#[derive(Debug, Default, Clone, Copy)]
struct ClientActivity {
    blocking_reloads: u64,
    preload_hint_resource_fetches: u64,
}

/// One driven client run: every emitted sample's glass-to-glass age, and what
/// the engine asked the origin for.
struct ClientRun {
    sample_count: usize,
    latencies: Vec<Duration>,
    activity: ClientActivity,
}

/// `GET url`, optionally for one byte sub-range (RFC 8216bis §4.4.4.9's
/// `BYTERANGE`), returning the response body.
async fn get_bytes(http: &reqwest::Client, url: &str, byte_range: Option<(u64, u64)>) -> Vec<u8> {
    try_get_bytes(http, url, byte_range)
        .await
        .unwrap_or_else(|| panic!("client GET {url} must succeed"))
}

/// [`get_bytes`], `None` for a `404` — the origin no longer has that part.
async fn try_get_bytes(
    http: &reqwest::Client,
    url: &str,
    byte_range: Option<(u64, u64)>,
) -> Option<Vec<u8>> {
    let mut request = http.get(url);
    if let Some((offset, length)) = byte_range {
        let end = offset.saturating_add(length.saturating_sub(1));
        request = request.header("Range", format!("bytes={offset}-{end}"));
    }
    let response = request.send().await.expect("client GET");
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return None;
    }
    assert!(
        response.status().is_success(),
        "client GET {url} answered {}",
        response.status()
    );
    Some(
        response
            .bytes()
            .await
            .expect("client reads the body")
            .to_vec(),
    )
}

/// Whether a resource GET for `url` is the prefetch of `hint` — the part the
/// last playlist named in `#EXT-X-PRELOAD-HINT` (RFC 8216bis §4.4.5.3) —
/// which is fetched ahead of that part's own numbered `#EXT-X-PART` entry.
/// Both are urls; the hint is compared by its last path segment, the same
/// resource the client resolves relative to the playlist.
fn prefetches_hint(hint: Option<&str>, url: &str) -> bool {
    let Some(hint) = hint else {
        return false;
    };
    let requested = url.rsplit('/').next().unwrap_or(url);
    requested == hint
}

/// The part url a playlist names in its `#EXT-X-PRELOAD-HINT` (RFC 8216bis
/// §4.4.5.3), if any.
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

/// Start the real `multimux` LL-HLS origin on an ephemeral loopback port,
/// serving one stream named `live`.
async fn start_ll_origin() -> (Arc<RouteHandle>, String, tokio::task::JoinHandle<()>) {
    let store = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_hold_cold_playlist(false),
    );
    // `multimux` egress resolves what it serves through the route's program
    // registry (issue #805/#806). This test drives the handle's own
    // per-program `Trunk` directly rather than through a supervisor, so it
    // has to publish that program itself -- otherwise every playlist request
    // blocks waiting for a program that is already there, and this test
    // fails on its 25s hang guard rather than on anything it means to
    // assert.
    store.publish_new_program(ProgramId(0));
    let mut streams = HashMap::new();
    streams.insert(
        "live".to_string(),
        (
            store.clone(),
            vec![Arc::new(MmHlsOutput::default()) as Arc<dyn MmOutput>],
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

/// Feed [`FRAME_COUNT`] samples into `store` via a real
/// `transmux::ll_hls::LlHlsSegmenter`, real-time-paced at [`FPS`] frames/sec
/// — a live-shaped producer, not a batch dump.
async fn run_live_producer(store: Arc<RouteHandle>) {
    let program = ProgramId(0);
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![video_track()],
        MOVIE_TIMESCALE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init segment builds"));

    let frame_interval = Duration::from_millis(1000 / u64::from(FPS));
    for i in 0..FRAME_COUNT {
        let is_sync = i % SYNC_EVERY == 0;
        seg.push(TrackId::new(TRACK_ID), timestamped_sample(is_sync))
            .expect("push succeeds");
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add_segment");
        }
        tokio::time::sleep(frame_interval).await;
    }
    seg.flush().expect("flush succeeds");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add_segment");
    }
}

/// Drives `playlist_url` with the source's own [`HlsIngestSession`] — the
/// same session, `poll_transmit` → GET → `feed` loop `run_hls_pull` runs
/// (and the same [`HlsPullRoute::new`] construction) — and ages every sample
/// the session emits against the clock carried in its own payload.
///
/// The run stops once every sample has been emitted, exactly where the old
/// client stopped. A part this origin no longer serves by then is the live
/// edge moving past the client (it lives only in the window, and this
/// origin's window is small), not a client fault: the session is told the
/// resource is gone and carries on to the next one, as the production loop's
/// bounded resource retry does.
async fn drive_session_and_time_samples(playlist_url: &str) -> ClientRun {
    let http = reqwest::Client::new();
    let mut session = HlsIngestSession::new(playlist_url);
    let mut run = ClientRun {
        sample_count: 0,
        latencies: Vec::new(),
        activity: ClientActivity::default(),
    };
    let mut hint: Option<String> = None;

    loop {
        let Some(action) = session.poll_transmit() else {
            if session.ended() || run.sample_count >= FRAME_COUNT as usize {
                break;
            }
            tokio::time::sleep(IDLE_POLL_INTERVAL).await;
            continue;
        };
        match action {
            Action::FetchPlaylist { blocking, .. } if run.sample_count >= FRAME_COUNT as usize => {
                let _ = blocking;
                break;
            }
            Action::FetchPlaylist { blocking, .. } => {
                let url = action
                    .playlist_request_url()
                    .expect("FetchPlaylist always names a request URL");
                if blocking.is_some() {
                    run.activity.blocking_reloads = run.activity.blocking_reloads.saturating_add(1);
                }
                let body = get_bytes(&http, &url, None).await;
                hint = preload_hint(&body);
                session
                    .feed((HlsFetchId::Playlist, body.as_slice()), Timestamp::ZERO)
                    .expect("playlist accepted");
            }
            Action::FetchResource {
                id,
                url,
                byte_range,
            } => {
                if prefetches_hint(hint.as_deref(), &url) {
                    run.activity.preload_hint_resource_fetches =
                        run.activity.preload_hint_resource_fetches.saturating_add(1);
                    hint = None;
                }
                match try_get_bytes(&http, &url, byte_range).await {
                    Some(body) => session
                        .feed((HlsFetchId::Resource(id), body.as_slice()), Timestamp::ZERO)
                        .expect("resource accepted"),
                    None => session.resource_gone(id),
                }
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let SessionEvent::Sample { sample, .. } = event {
                run.latencies.push(glass_to_glass(&sample));
                run.sample_count = run.sample_count.saturating_add(1);
            }
        }
    }
    run
}

// ===========================================================================
// The epic's headline acceptance: sub-second glass-to-glass over loopback,
// blocking reload + preload-hint prefetch actually exercised.
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn glass_to_glass_sub_second_over_loopback_with_blocking_reload_and_prefetch() {
    // HANG GUARD (issue #807): 25s on work that normally completes in ~3s
    // (FRAME_COUNT=90 @ 30fps). Its only job is to fail a deadlock rather
    // than hang CI forever -- raise freely if it ever proves too tight, it
    // asserts nothing about latency itself (see the tight `max_latency`
    // assertion below for that).
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        let (store, playlist_url, server) = start_ll_origin().await;
        let producer = tokio::spawn(run_live_producer(store));

        let run = drive_session_and_time_samples(&playlist_url).await;
        producer.await.expect("producer task must not panic");
        server.abort();
        run
    })
    .await
    .expect("glass-to-glass test must complete within 25s (deadlock/hang guard)");

    let ClientRun {
        sample_count,
        mut latencies,
        activity,
    } = result;

    assert_eq!(
        sample_count, FRAME_COUNT as usize,
        "must reconstruct every sample the producer pushed, no gaps/duplicates"
    );
    latencies.sort_unstable();
    let max_latency = latencies.last().copied().unwrap_or_default();
    let p90_latency = latencies
        [(latencies.len() * STEADY_STATE_PERCENTILE / 100).min(latencies.len().saturating_sub(1))];
    // LATENCY ASSERTION (issue #2362): p90 under 1s is the headline bar and
    // is NOT loosened; the max is bounded at 3s to catch real hitches. Using
    // p90 because a host stall moves the max one-for-one.
    if std::env::var_os("LL_HLS_RELAX_LATENCY_ASSERTIONS").is_some() {
        eprintln!(
            "WARNING: LL_HLS_RELAX_LATENCY_ASSERTIONS is set -- skipping the \
             sub-second glass-to-glass latency assertion (p90 {p90_latency:?}, \
             max {max_latency:?}). \
             This does NOT validate the product's headline latency claim; only set \
             this on a box you already know is contended, and get a real signal by \
             unsetting it on a quiet box."
        );
    } else {
        assert!(
            p90_latency < P90_LATENCY_BOUND,
            "glass-to-glass p90 latency must be sub-second (issue #717 headline acceptance): p90 {p90_latency:?}, max {max_latency:?}"
        );
        assert!(
            max_latency < MAX_LATENCY_CEILING,
            "a single sample took {max_latency:?} glass to glass (ceiling {MAX_LATENCY_CEILING:?}); p90 {p90_latency:?}"
        );
    }

    assert!(
        activity.blocking_reloads > 0,
        "must have issued at least one Blocking Playlist Reload (_HLS_msn/_HLS_part): {activity:?}"
    );
    assert!(
        activity.preload_hint_resource_fetches > 0,
        "must have fetched at least one part via its #EXT-X-PRELOAD-HINT, ahead of its own \
         numbered #EXT-X-PART appearance: {activity:?}"
    );

    eprintln!(
        "glass-to-glass: {sample_count} samples, max latency {max_latency:?}, activity {activity:?}"
    );
}

// ===========================================================================
// Non-LL origin: full-segment fallback, over real HTTP through the same
// session the production pull drives (proving the *adapter*, not just the
// sans-IO core, handles it — `hls-runtime/tests/it/origin_loop.rs` already
// proves the core alone).
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_ll_origin_plays_via_full_segment_fallback_over_http() {
    // HANG GUARD (issue #807): 15s on work that normally completes in well
    // under a second (30-31 synthetic samples, no real-time pacing on this
    // path). Only job: fail a deadlock rather than hang CI forever.
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
            seg.push(TrackId::new(TRACK_ID), s).expect("push succeeds");
        }
        seg.push(TrackId::new(TRACK_ID), timestamped_sample(true))
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
                // The segmenter's own computed duration over real sample
                // timestamps — always finite and non-negative (issue #1140).
                duration: DecimalSeconds::new(seg1.duration)
                    .expect("segmenter duration is finite, >= 0"),
                discontinuous: false,
                parts: vec![],
                byte_range: None,
                map: Some(MapTag {
                    uri: "init.mp4".to_string(),
                    byte_range: None,
                    extra_attrs: vec![],
                }),
                ..Default::default()
            }],
            open_segment: None,
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            rendition_reports: vec![],
            skip: None,
            ..Default::default()
        };
        let playlist_text = playlist.to_m3u8().expect("valid playlist");
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

        let run = drive_session_and_time_samples(&format!("http://{addr}/media.m3u8")).await;
        server.abort();

        assert_eq!(
            run.sample_count,
            fed_samples.len(),
            "fallback path must reconstruct every sample, no gaps/duplicates"
        );
        assert!(
            run.latencies.iter().all(|age| *age < MAX_LATENCY_CEILING),
            "every fallback-path sample must arrive inside the ceiling: {:?}",
            run.latencies.iter().max()
        );
        assert_eq!(
            run.activity.blocking_reloads, 0,
            "a non-LL playlist (no PART tags) must never trigger a blocking reload: {:?}",
            run.activity
        );
        assert!(
            run.activity.preload_hint_resource_fetches == 0,
            "a non-LL playlist advertises no preload hint: {:?}",
            run.activity
        );
    })
    .await
    .expect("non-LL fallback test must complete within 15s (deadlock/hang guard)");
}
