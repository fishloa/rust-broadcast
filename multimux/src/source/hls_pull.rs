//! HLS-pull ingest source (issue #663 P3c / #717 / #760): a remote (LL-)HLS origin pulled by a
//! relay-mode [`broadcast_client::Session`] through the shared session host
//! (`crate::source::client_host`).
//!
//! The session owns the protocol: playlist reload pacing and Low-Latency blocking reload
//! (RFC 8216bis), part prefetch from the preload hint, AES-128 keys, fMP4 and classic-TS segment
//! demux, discontinuities, per-resource retry with backoff, `Retry-After`, redirects, cookies and
//! Basic/Digest/Bearer auth. The host performs its HTTP requests (at most
//! [`MAX_INFLIGHT_FETCHES`](crate::source::MAX_INFLIGHT_FETCHES) media requests at once, a
//! session setting) and writes what the session emits into the route's `Trunk`.
//!
//! A finite playlist (`#EXT-X-ENDLIST`) ends the pull cleanly and the route is served as ended;
//! an authentication or permission refusal (`401`/`403`) fails the attempt at once rather than
//! being retried.

use broadcast_client::{Config, SourceKind};
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;
use std::convert::Infallible;

use crate::error::{MultimuxError, Result};
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep};
use crate::source::pull_target::{PullKind, PullRoute};

/// How many times one media resource is requested before the session gives up on it.
const MAX_RESOURCE_ATTEMPTS: u32 = 8;

/// Delay before the first retry of a failed resource, milliseconds; each retry doubles it up to
/// [`RESOURCE_RETRY_CAP_MS`]. An eviction race reappears on the next playlist reload seconds
/// later, so a fixed short delay would burn the whole budget before it heals.
const RESOURCE_RETRY_BASE_MS: u64 = 500;

/// Cap on the exponential resource retry delay, milliseconds.
const RESOURCE_RETRY_CAP_MS: u64 = 30_000;

/// Cap on a `#EXT-X-KEY` key-file body. A key file is exactly 16 octets (RFC 8216 §5.1); the
/// slack only lets a short error page through to be rejected by length instead of by the cap.
const MAX_KEY_BODY_BYTES: usize = 4096;

const WHAT: &str = "hls-pull";

/// A remote (LL-)HLS Media Playlist to pull: its URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
pub type HlsPullRoute = PullRoute<HlsPullKind>;

/// The [`PullKind`] of a [`HlsPullRoute`].
#[derive(Debug, Clone, Copy)]
pub struct HlsPullKind;

impl PullKind for HlsPullKind {
    const ROUTE: &'static str = "HlsPullRoute";
    type Extra = ();
}

impl crate::source::pull_target::PullDrive for HlsPullKind {
    const SEGMENTED: bool = true;

    async fn drive(
        route: std::sync::Arc<crate::source::pull_target::PullRoute<Self>>,
        trunk_config: media_plane::trunk::TrunkConfig,
        handshake: media_plane::ingress::HandshakePolicy,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        run_hls_pull(&route, trunk_config, handshake, &route_handle, cancel).await
    }
}

fn retry_schedule_ms() -> Vec<u64> {
    let mut schedule = Vec::new();
    let mut delay = RESOURCE_RETRY_BASE_MS;
    for _ in 0..MAX_RESOURCE_ATTEMPTS {
        schedule.push(delay.min(RESOURCE_RETRY_CAP_MS));
        delay = delay.saturating_mul(2);
    }
    schedule
}

fn session_config(route: &HlsPullRoute) -> Config {
    let mut config = Config::relay();
    config.kind = SourceKind::Hls;
    let mut config = config.with_timeouts(route.target.timeouts);
    config.max_queued_bytes = crate::source::SEGMENTED_EVENT_QUEUE_BYTES;
    config.max_manifest_bytes = crate::source::MAX_HTTP_BODY_BYTES;
    config.max_segment_bytes = crate::source::MAX_HTTP_BODY_BYTES;
    config.max_key_bytes = MAX_KEY_BODY_BYTES;
    config.max_inflight_segments = crate::source::MAX_INFLIGHT_FETCHES;
    config.max_segment_failures = MAX_RESOURCE_ATTEMPTS;
    config.backoff_ms = retry_schedule_ms();
    config.backoff_cap_ms = RESOURCE_RETRY_CAP_MS;
    config
}

/// Drives `route` until its playlist ends or the session fails outright.
///
/// Returns `Ok(())` when the origin's `#EXT-X-ENDLIST` is fully accounted for (the route is then
/// marked ended: a finite playlist has nothing more to pull) and `Err` on a failure the session
/// does not retry. Each fetch is bounded by [`Timeouts::read_idle`](broadcast_common::timeouts::Timeouts::read_idle),
/// and the handshake (until the first track set resolves) by `handshake`.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 — see
/// `crate::source::rtsp::run_rtsp`'s own doc for what `crate::source::report_driver_progress`
/// does with it each iteration.
pub async fn run_hls_pull(
    route: &HlsPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let timeouts = route.target.timeouts;
    let mut step = SessionStep::open(
        &route.target.url,
        session_config(route),
        route.target.auth.clone(),
        SessionLabels::new(WHAT, timeouts).session_retries(),
        std::sync::Arc::clone(route_handle),
        false,
    )?;
    let driver = IngestDriver::new(
        EventIngest::new(),
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut sink = tokio::io::sink();
    let err = crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        cancel,
        timeouts,
        &mut sink,
        &mut step,
        |never: Infallible| match never {},
    )
    .await;
    let result = match err {
        MultimuxError::Connect { reason } if reason.starts_with("session ended") => Ok(()),
        other => Err(other),
    };
    crate::source::finish_vod(route_handle, result)
}

#[cfg(test)]
mod tests {
    use crate::route::RouteConfig;

    use super::*;

    #[test]
    fn the_session_queue_is_budgeted_in_bytes() {
        let config = session_config(&HlsPullRoute::new("q", "http://127.0.0.1:1/media.m3u8"));
        assert_eq!(
            config.max_queued_bytes,
            crate::source::SEGMENTED_EVENT_QUEUE_BYTES
        );
    }
    use crate::testutil::MockAuthScheme;
    use broadcast_auth::Credentials;
    use broadcast_common::Timestamp;
    use broadcast_common::timeouts::Timeouts;
    use broadcast_hls::{MediaPlaylist, MediaSegment};
    use media_plane::ingress::{HandshakePolicy, ProgramId};
    use media_plane::trunk::{SampleCursorItem, TrunkConfig};
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use transmux::TrackId;
    use transmux::TrackSpec;
    use transmux::ll_hls::LlHlsSegmenter;
    use transmux::pipeline::Sample;
    use transmux::{
        AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig,
    };

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    /// W14d: a resource whose fetch fails with a permanent **auth** error
    /// (401/403) must fail the session immediately, not retry 8 times with
    /// backoff (which would only delay the reconnect).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_hls_pull_fails_fast_on_a_permanent_resource_error() {
        use axum::Router;
        use axum::extract::{Path as AxumPath, State};
        use axum::response::{IntoResponse, Response as AxumResponse};
        use axum::routing::get;
        use std::sync::atomic::{AtomicU64, Ordering};

        let (playlist_text, init, segments) = build_cmaf_fixture();

        #[derive(Clone)]
        struct State2 {
            playlist: String,
            init: Vec<u8>,
            segments: std::sync::Arc<Vec<Vec<u8>>>,
            seg0_requests: std::sync::Arc<AtomicU64>,
        }

        async fn handler(
            AxumPath(name): AxumPath<String>,
            State(state): State<State2>,
        ) -> AxumResponse {
            if name == "media.m3u8" {
                return state.playlist.into_response();
            }
            if name == "init.mp4" {
                return state.init.into_response();
            }
            if name == "seg0.m4s" {
                // Always 403 — a permanent failure.
                state.seg0_requests.fetch_add(1, Ordering::SeqCst);
                return axum::http::StatusCode::FORBIDDEN.into_response();
            }
            if let Some(idx) = name
                .strip_prefix("seg")
                .and_then(|s| s.strip_suffix(".m4s"))
                .and_then(|s| s.parse::<usize>().ok())
                && let Some(bytes) = state.segments.get(idx)
            {
                return bytes.clone().into_response();
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }

        let requests = std::sync::Arc::new(AtomicU64::new(0));
        let app = Router::new()
            .route("/{name}", get(handler))
            .with_state(State2 {
                playlist: playlist_text,
                init,
                segments: std::sync::Arc::new(segments),
                seg0_requests: std::sync::Arc::clone(&requests),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let route = HlsPullRoute::new("pulled-403", format!("http://{addr}/media.m3u8"));
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("must return");
        server.abort();
        assert!(
            matches!(result, Err(crate::error::MultimuxError::Auth(_))),
            "a 403 resource must fail the session"
        );
        // It must NOT have retried 8 times (~2 min of backoff).
        assert!(
            requests.load(Ordering::SeqCst) <= 2,
            "a permanent 403 must not be retried, saw {} requests",
            requests.load(Ordering::SeqCst)
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "fail-fast must return well under the full retry budget"
        );
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(16))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    const TRACK_ID: TrackId = TrackId::new(1);
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
    const TARGET_DURATION_SECS: f64 = 1.0;
    const FRAME_COUNT: u32 = 60;

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

    fn video_track_spec() -> TrackSpec {
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

    /// Builds a real, non-LL (whole-segment) CMAF Media Playlist plus its
    /// init/segment byte blobs by driving a real `LlHlsSegmenter` — the same
    /// "real fixture, not hand-faked bytes" discipline
    /// `media_plane::ts_session::test_support::build_ts_bytes` uses — and renders the
    /// playlist via `broadcast_hls::MediaPlaylist::to_m3u8` (the same real
    /// renderer the workspace's own LL-HLS origin uses), rather than
    /// depending on `multimux`'s own (unrelated, currently-broken — see this
    /// crate's CHANGELOG) `store`/`origin`/`output::hls` modules the pre-5a
    /// version of this test built a whole origin server out of.
    fn build_cmaf_fixture() -> (String, Vec<u8>, Vec<Vec<u8>>) {
        let mut seg = LlHlsSegmenter::with_part_target(
            vec![video_track_spec()],
            transmux::VIDEO_CLOCK_RATE,
            TARGET_DURATION_SECS,
            250,
        )
        .expect("segmenter builds");
        let init = seg.init_segment().expect("init segment builds");

        for i in 0..FRAME_COUNT {
            let is_sync = i % 15 == 0;
            let data = vec![0xABu8.wrapping_add(i as u8); 32];
            let sample = Sample::new(
                data,
                Some(i64::from(i) * i64::from(FRAME_DUR)),
                Some(i64::from(i) * i64::from(FRAME_DUR)),
                Some(FRAME_DUR),
                is_sync,
            );
            seg.push(TRACK_ID, sample).expect("push succeeds");
            for _ in seg.take_ready_parts() {} // non-LL playlist: parts unused
        }
        seg.flush().expect("flush succeeds");

        let map = broadcast_hls::MapTag {
            uri: "init.mp4".to_string(),
            byte_range: None,
            extra_attrs: Vec::new(),
        };
        let mut segments = Vec::new();
        let mut media_segments = Vec::new();
        for (i, segment) in seg.take_ready_segments().into_iter().enumerate() {
            media_segments.push(MediaSegment {
                uri: format!("seg{i}.m4s"),
                // Test fixture: `segment.timing.map(|t| t.duration)` is the segmenter's own
                // computed duration over synthetic sample timestamps —
                // always finite and non-negative (issue #1140).
                duration: broadcast_hls::DecimalSeconds::new(segment.duration)
                    .expect("segmenter duration is finite, >= 0"),
                discontinuous: false,
                parts: Vec::new(),
                byte_range: None,
                map: Some(map.clone()),
                ..Default::default()
            });
            segments.push(segment.bytes);
        }

        let playlist = MediaPlaylist {
            version: 7,
            target_duration: TARGET_DURATION_SECS.ceil() as u32,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: media_segments,
            open_segment: None,
            endlist: true,
            extra_tags: Vec::new(),
            low_latency: None,
            iframes_only: false,
            rendition_reports: Vec::new(),
            skip: None,
            ..Default::default()
        };
        (
            playlist.to_m3u8().expect("valid fixture URIs"),
            init,
            segments,
        )
    }

    /// Starts a real axum server hosting a real CMAF fixture (see
    /// [`build_cmaf_fixture`]) — `init.mp4` + `EXT-X-MAP`, `segN.m4s` per
    /// `MediaSegment`. `auth`, if given, gates every request.
    async fn start_cmaf_fixture_server(
        auth: Option<MockAuthScheme>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        start_counting_cmaf_fixture_server(auth, Arc::new(AtomicUsize::new(0))).await
    }

    /// [`start_cmaf_fixture_server`], counting every `segN.m4s` GET in `hits`.
    async fn start_counting_cmaf_fixture_server(
        auth: Option<MockAuthScheme>,
        hits: Arc<AtomicUsize>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::Router;
        use axum::extract::{Path as AxumPath, State};
        use axum::response::{IntoResponse, Response as AxumResponse};
        use axum::routing::get;

        let (playlist_text, init, segments) = build_cmaf_fixture();

        #[derive(Clone)]
        struct FixtureState {
            playlist: String,
            init: Vec<u8>,
            segments: std::sync::Arc<Vec<Vec<u8>>>,
            hits: Arc<AtomicUsize>,
        }

        async fn handler(
            AxumPath(name): AxumPath<String>,
            State(state): State<FixtureState>,
        ) -> AxumResponse {
            if name == "media.m3u8" {
                return state.playlist.into_response();
            }
            if name == "init.mp4" {
                return state.init.into_response();
            }
            if let Some(idx) = name
                .strip_prefix("seg")
                .and_then(|s| s.strip_suffix(".m4s"))
                .and_then(|s| s.parse::<usize>().ok())
                && let Some(bytes) = state.segments.get(idx)
            {
                state.hits.fetch_add(1, Ordering::SeqCst);
                return bytes.clone().into_response();
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }

        let state = FixtureState {
            playlist: playlist_text,
            init,
            segments: std::sync::Arc::new(segments),
            hits,
        };
        let mut app = Router::new()
            .route("/{name}", get(handler))
            .with_state(state);
        if let Some(scheme) = auth {
            app = crate::testutil::require_auth(app, scheme);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/media.m3u8"), server)
    }

    const COUNT_TIMED_RING: usize = 4096;

    /// Pulls `route` to the end of its finite playlist and reads back what the route published:
    /// the result, the track set, and the number of samples per track the `Trunk` holds.
    async fn pull_to_end(
        route: &HlsPullRoute,
    ) -> (Result<()>, Vec<TrackSpec>, HashMap<TrackId, usize>) {
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let config = TrunkConfig::default()
            .with_timed_capacity(nz(COUNT_TIMED_RING))
            .with_sparse_capacity(nz(16))
            .with_segment_capacity(nz(16))
            .with_event_capacity(nz(16))
            .with_part_capacity(nz(16));
        // HANG GUARD (issue #826): only fails "never finishes" rather than hanging CI.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(
                route,
                config,
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_hls_pull must not hang against a finite playlist");
        let trunk = route_handle.await_first_trunk().await;
        let specs = tokio::time::timeout(Duration::from_secs(10), async {
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
        let mut cursor = trunk.subscribe_from_backlog();
        let mut per_track: HashMap<TrackId, usize> = HashMap::new();
        while let Some(item) = cursor.poll() {
            if let SampleCursorItem::Timed { track_id, .. } = item {
                *per_track.entry(track_id).or_insert(0) += 1;
            }
        }
        (result, specs, per_track)
    }

    /// Biting loopback test: a real axum server serves a real `LlHlsSegmenter`-built CMAF
    /// fixture over real HTTP; the route recovers the right `TrackSpec` and its `Trunk` holds
    /// **exactly** the fixture's own sample count.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn loopback_hls_pull_lands_real_samples_in_trunk() {
        let (url, server) = start_cmaf_fixture_server(None).await;
        let route = HlsPullRoute::new("pulled-cam", url);
        let (result, specs, per_track) = pull_to_end(&route).await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(specs.len(), 1, "one video track recovered: {specs:?}");
        assert_eq!(specs[0].track_id, TRACK_ID);
        assert_eq!(specs[0].timescale.get(), VIDEO_TIMESCALE);
        assert!(
            matches!(specs[0].config, CodecConfig::Avc { .. }),
            "codec config must round-trip as AVC: {:?}",
            specs[0].config
        );
        assert_eq!(
            per_track.get(&TRACK_ID).copied().unwrap_or(0),
            FRAME_COUNT as usize,
            "must pull every real sample from the CMAF fixture, no gaps/duplicates"
        );
        server.abort();
    }

    /// The full `run_hls_pull` drive loop (bounded fan-out, real timeouts)
    /// against the same fixture, asserting it returns cleanly once the
    /// origin's `#EXT-X-ENDLIST` is fully accounted for.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_hls_pull_completes_cleanly_on_a_static_playlist() {
        let (url, server) = start_cmaf_fixture_server(None).await;
        let route = HlsPullRoute::new("pulled-cam", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        // HANG GUARD (issue #826): backstop around the full `run_hls_pull`
        // against a static CMAF fixture. `run_hls_pull` returns on its own
        // once the playlist is exhausted; this only exists to fail "never
        // returns" rather than hang CI, not a timing claim.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_hls_pull must not hang against a static playlist");
        assert!(
            result.is_ok(),
            "a static playlist must end cleanly: {result:?}"
        );
        server.abort();
    }

    /// Round-9 F4: a finite (`#EXT-X-ENDLIST`) playlist pulled under the
    /// route supervisor is fetched once and then served — not re-pulled in a
    /// reconnect loop that hammers the origin and rejects every re-publish.
    ///
    /// Revert-check: without `finish_vod` the supervisor reconnects every
    /// backoff and the segment GET count climbs past the segment count.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finite_playlist_is_fetched_once_and_then_served() {
        let hits = Arc::new(AtomicUsize::new(0));
        let (url, server) = start_counting_cmaf_fixture_server(None, hits.clone()).await;
        let segment_count = build_cmaf_fixture().2.len();
        let route = Arc::new(HlsPullRoute::new("vod", url));
        let route_handle = Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let cancel = tokio_util::sync::CancellationToken::new();
        let attempt_route = route.clone();
        let supervisor = tokio::spawn(crate::origin::supervisor::supervise_driver(
            move |handle| {
                let route = attempt_route.clone();
                async move {
                    run_hls_pull(
                        &route,
                        trunk_config(),
                        handshake(),
                        &handle,
                        tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                }
            },
            route_handle.clone(),
            crate::origin::supervisor::Backoff::new(
                Duration::from_millis(20),
                Duration::from_millis(40),
                2.0,
            ),
            "vod".to_string(),
            cancel.clone(),
        ));

        let settle = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < settle {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            route_handle.health(),
            crate::route::HealthState::Ended,
            "a finished playlist is Ended"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            segment_count,
            "every segment is fetched exactly once"
        );
        let crate::route::ProgramResolution::Found(serving) =
            route_handle.resolve_program(crate::route::SPTS_PROGRAM_ID)
        else {
            panic!("the finished VOD must still be served");
        };
        assert!(
            serving.trunk().last_closed_segment().is_some(),
            "the VOD's segments stay available"
        );
        cancel.cancel();
        supervisor.await.expect("supervisor did not panic");
        server.abort();
    }

    /// Audit run 7, W14d: a resource fetch that fails a **few** times (a
    /// `404` from an eviction race) must be retried, not end the session, and
    /// its retries must not bypass the in-flight bound. The server 404s
    /// `seg0.m4s` on its first two requests and serves it thereafter, and
    /// counts requests; the trunk must end up carrying real samples.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_hls_pull_retries_a_transient_resource_failure() {
        use axum::Router;
        use axum::extract::{Path as AxumPath, State};
        use axum::response::{IntoResponse, Response as AxumResponse};
        use axum::routing::get;
        use std::sync::atomic::AtomicU64;

        let (playlist_text, init, segments) = build_cmaf_fixture();

        #[derive(Clone)]
        struct State2 {
            playlist: String,
            init: Vec<u8>,
            segments: std::sync::Arc<Vec<Vec<u8>>>,
            seg0_requests: std::sync::Arc<AtomicU64>,
        }

        async fn handler(
            AxumPath(name): AxumPath<String>,
            State(state): State<State2>,
        ) -> AxumResponse {
            if name == "media.m3u8" {
                return state.playlist.into_response();
            }
            if name == "init.mp4" {
                return state.init.into_response();
            }
            if name == "seg0.m4s" {
                // Fail the first TWO requests, serve the third — proving the
                // retry (not a lucky single attempt).
                let n = state
                    .seg0_requests
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    return axum::http::StatusCode::NOT_FOUND.into_response();
                }
            }
            if let Some(idx) = name
                .strip_prefix("seg")
                .and_then(|s| s.strip_suffix(".m4s"))
                .and_then(|s| s.parse::<usize>().ok())
                && let Some(bytes) = state.segments.get(idx)
            {
                return bytes.clone().into_response();
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }

        let seg0_requests = std::sync::Arc::new(AtomicU64::new(0));
        let app = Router::new()
            .route("/{name}", get(handler))
            .with_state(State2 {
                playlist: playlist_text,
                init,
                segments: std::sync::Arc::new(segments),
                seg0_requests: std::sync::Arc::clone(&seg0_requests),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });

        let route = HlsPullRoute::new("pulled-retry", format!("http://{addr}/media.m3u8"));
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let handle = std::sync::Arc::clone(&route_handle);
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_hls_pull must not hang");
        assert!(
            result.is_ok(),
            "a transient resource failure must be retried, not end the session: {result:?}"
        );
        assert!(
            seg0_requests.load(std::sync::atomic::Ordering::SeqCst) >= 3,
            "seg0 must have been requested at least 3 times (2 failures + success)"
        );
        let _ = handle;
        let trunk = route_handle.await_first_trunk().await;
        let mut cursor = trunk.subscribe_from_backlog();
        let mut samples = 0usize;
        while let Some(item) = cursor.poll() {
            if matches!(item, SampleCursorItem::Timed { .. }) {
                samples += 1;
            }
        }
        assert!(samples > 0, "the retried segment's samples reach the Trunk");
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_hls_pull_honours_retry_after_on_a_503() {
        use axum::Router;
        use axum::extract::{Path as AxumPath, State};
        use axum::response::{IntoResponse, Response as AxumResponse};
        use axum::routing::get;
        use std::sync::Mutex as StdMutex;
        use std::sync::atomic::AtomicU64;

        let (playlist_text, init, segments) = build_cmaf_fixture();

        #[derive(Clone)]
        struct State2 {
            playlist: String,
            init: Vec<u8>,
            segments: std::sync::Arc<Vec<Vec<u8>>>,
            seg0_requests: std::sync::Arc<AtomicU64>,
            first_at: std::sync::Arc<StdMutex<Option<tokio::time::Instant>>>,
            second_at: std::sync::Arc<StdMutex<Option<tokio::time::Instant>>>,
        }

        async fn handler(
            AxumPath(name): AxumPath<String>,
            State(state): State<State2>,
        ) -> AxumResponse {
            if name == "media.m3u8" {
                return state.playlist.into_response();
            }
            if name == "init.mp4" {
                return state.init.into_response();
            }
            if name == "seg0.m4s" {
                let n = state
                    .seg0_requests
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    *state.first_at.lock().unwrap() = Some(tokio::time::Instant::now());
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        [("retry-after", "2")],
                    )
                        .into_response();
                }
                *state.second_at.lock().unwrap() = Some(tokio::time::Instant::now());
            }
            if let Some(idx) = name
                .strip_prefix("seg")
                .and_then(|s| s.strip_suffix(".m4s"))
                .and_then(|s| s.parse::<usize>().ok())
                && let Some(bytes) = state.segments.get(idx)
            {
                return bytes.clone().into_response();
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }

        let seg0_requests = std::sync::Arc::new(AtomicU64::new(0));
        let first_at = std::sync::Arc::new(StdMutex::new(None));
        let second_at = std::sync::Arc::new(StdMutex::new(None));
        let app = Router::new()
            .route("/{name}", get(handler))
            .with_state(State2 {
                playlist: playlist_text,
                init,
                segments: std::sync::Arc::new(segments),
                seg0_requests: std::sync::Arc::clone(&seg0_requests),
                first_at: std::sync::Arc::clone(&first_at),
                second_at: std::sync::Arc::clone(&second_at),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });

        let route = HlsPullRoute::new("pulled-retry-after", format!("http://{addr}/media.m3u8"));
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let handle = std::sync::Arc::clone(&route_handle);
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_hls_pull must not hang");
        assert!(
            result.is_ok(),
            "a 503 with Retry-After must be retried, not end the session: {result:?}"
        );
        let first = first_at.lock().unwrap().expect("first request recorded");
        let second = second_at.lock().unwrap().expect("second request recorded");
        assert!(
            second.duration_since(first) >= Duration::from_secs(2),
            "retry must wait at least the Retry-After delay"
        );
        let _ = handle;
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_hls_pull_honours_retry_after_on_a_playlist_503() {
        use axum::Router;
        use axum::extract::{Path as AxumPath, State};
        use axum::response::{IntoResponse, Response as AxumResponse};
        use axum::routing::get;
        use std::sync::Mutex as StdMutex;

        let (playlist_text, init, segments) = build_cmaf_fixture();

        #[derive(Clone)]
        struct State2 {
            playlist: String,
            init: Vec<u8>,
            segments: std::sync::Arc<Vec<Vec<u8>>>,
            playlist_at: std::sync::Arc<StdMutex<Vec<std::time::Instant>>>,
        }

        async fn handler(
            AxumPath(name): AxumPath<String>,
            State(state): State<State2>,
        ) -> AxumResponse {
            if name == "media.m3u8" {
                let mut at = state.playlist_at.lock().unwrap();
                at.push(std::time::Instant::now());
                if at.len() == 1 {
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        [("retry-after", "2")],
                    )
                        .into_response();
                }
                return state.playlist.clone().into_response();
            }
            if name == "init.mp4" {
                return state.init.into_response();
            }
            if let Some(idx) = name
                .strip_prefix("seg")
                .and_then(|s| s.strip_suffix(".m4s"))
                .and_then(|s| s.parse::<usize>().ok())
                && let Some(bytes) = state.segments.get(idx)
            {
                return bytes.clone().into_response();
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }

        let playlist_at = std::sync::Arc::new(StdMutex::new(Vec::new()));
        let app = Router::new()
            .route("/{name}", get(handler))
            .with_state(State2 {
                playlist: playlist_text,
                init,
                segments: std::sync::Arc::new(segments),
                playlist_at: std::sync::Arc::clone(&playlist_at),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });

        let route = HlsPullRoute::new("pulled-playlist-wait", format!("http://{addr}/media.m3u8"));
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_hls_pull must not hang");
        assert!(
            result.is_ok(),
            "a playlist 503 with Retry-After must be retried, not end the session: {result:?}"
        );
        let at = playlist_at.lock().unwrap().clone();
        assert!(at.len() >= 2, "{at:?}");
        assert!(
            at[1].duration_since(at[0]) >= Duration::from_secs(2),
            "the playlist retry came {:?} after the 503",
            at[1].duration_since(at[0])
        );
        server.abort();
    }

    /// Issue #760: classic MPEG-TS-segment HLS (HLS v3 — no `EXT-X-MAP`,
    /// self-contained `.ts` segments, the dominant legacy/IPTV form) served
    /// from the real, committed `../fixtures/hls/ts-hls/`
    /// fixture — proving the pump recovers real `TrackSpec`s (from the
    /// client's issue-#760-synthesized `Output::Init`) and every real access
    /// unit lands in the `Trunk`, entirely through the production
    /// `HlsClient` — no TS-specific code in this module at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pulls_classic_ts_segment_hls_and_lands_samples_in_trunk() {
        use axum::Router;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use transmux::TsDemux;

        let fixture_dir = test_fixtures::path("hls/ts-hls");
        let playlist_text =
            std::fs::read_to_string(fixture_dir.join("index.m3u8")).expect("read fixture playlist");
        assert!(
            !playlist_text.contains("EXT-X-MAP"),
            "sanity: fixture must genuinely carry no EXT-X-MAP"
        );
        let seg0 = std::fs::read(fixture_dir.join("index0.ts")).expect("read fixture segment 0");
        let seg1 = std::fs::read(fixture_dir.join("index1.ts")).expect("read fixture segment 1");

        let mut want_total_samples = 0usize;
        for bytes in [&seg0, &seg1] {
            let media = TsDemux::new().demux(bytes).expect("oracle demux");
            want_total_samples += media.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
        }
        assert!(want_total_samples > 0, "sanity: fixture must carry samples");

        let seg0_for_route = seg0.clone();
        let seg1_for_route = seg1.clone();
        let app = Router::new()
            .route(
                "/media.m3u8",
                get(move || {
                    let text = playlist_text.clone();
                    async move { text.into_response() }
                }),
            )
            .route(
                "/index0.ts",
                get(move || {
                    let bytes = seg0_for_route.clone();
                    async move { bytes.into_response() }
                }),
            )
            .route(
                "/index1.ts",
                get(move || {
                    let bytes = seg1_for_route.clone();
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

        let route = HlsPullRoute::new("pulled-ts-hls", format!("http://{addr}/media.m3u8"));

        let (result, specs, per_track) = pull_to_end(&route).await;
        assert!(result.is_ok(), "{result:?}");

        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "must recover the fixture's AVC video track from the synthesized Init: {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "must recover the fixture's AAC audio track from the synthesized Init: {specs:?}"
        );
        let got_total: usize = per_track.values().sum();
        assert_eq!(
            got_total, want_total_samples,
            "must pull every real sample from the real TS-HLS origin, no gaps/duplicates"
        );

        server.abort();
    }

    // --- issue #663 "Finish client-side multi-scheme auth" ---

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "hls-pull-bearer-token";

    async fn drain_via_run_hls_pull(route: HlsPullRoute) -> Result<()> {
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        run_hls_pull(
            &route,
            trunk_config(),
            handshake(),
            &route_handle,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let (url, server) = start_cmaf_fixture_server(Some(MockAuthScheme::Basic {
            username: AUTH_USER.into(),
            password: AUTH_PASS.into(),
        }))
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);
        let route = HlsPullRoute::new("pulled-basic", credentialed);
        // HANG GUARD (issue #826): backstop around `drain_via_run_hls_pull`
        // for the Basic auth test. The route authenticates and drains a
        // static fixture; `run_hls_pull` returns on its own once exhausted.
        let result = tokio::time::timeout(Duration::from_secs(60), drain_via_run_hls_pull(route))
            .await
            .expect("must not hang");
        assert!(
            result.is_ok(),
            "Basic auth from URL userinfo must authenticate: {result:?}"
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn digest_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let (url, server) = start_cmaf_fixture_server(Some(MockAuthScheme::Digest {
            username: AUTH_USER.into(),
            password: AUTH_PASS.into(),
            realm: DIGEST_REALM.into(),
        }))
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);
        let route = HlsPullRoute::new("pulled-digest", credentialed);
        // HANG GUARD (issue #826): same as Basic auth — backstop, not a
        // timing claim.
        let result = tokio::time::timeout(Duration::from_secs(60), drain_via_run_hls_pull(route))
            .await
            .expect("must not hang");
        assert!(
            result.is_ok(),
            "Digest auth from URL userinfo must authenticate: {result:?}"
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bearer_auth_config_supplied_authenticates_and_pulls_samples() {
        let (url, server) = start_cmaf_fixture_server(Some(MockAuthScheme::Bearer {
            token: BEARER_TOKEN.into(),
        }))
        .await;
        let route = HlsPullRoute::new("pulled-bearer", url)
            .with_auth(Some(Credentials::bearer(BEARER_TOKEN)));
        // HANG GUARD (issue #826): same as Basic auth — backstop, not a
        // timing claim.
        let result = tokio::time::timeout(Duration::from_secs(60), drain_via_run_hls_pull(route))
            .await
            .expect("must not hang");
        assert!(
            result.is_ok(),
            "config-supplied Bearer must authenticate: {result:?}"
        );
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_credentials_fail_run_hls_pull() {
        let (url, server) = start_cmaf_fixture_server(Some(MockAuthScheme::Digest {
            username: AUTH_USER.into(),
            password: AUTH_PASS.into(),
            realm: DIGEST_REALM.into(),
        }))
        .await;
        let wrong_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);
        let route = HlsPullRoute::new("pulled-wrong", wrong_creds).with_timeouts(
            broadcast_common::timeouts::Timeouts::default()
                .with_connect(Timeouts::default().connect)
                .with_read_idle(Duration::from_secs(2)),
        );
        // HANG GUARD (issue #826): backstop around `drain_via_run_hls_pull`
        // for wrong credentials. The server returns 401 immediately so
        // `run_hls_pull` fails within ~ms; this only exists to fail "never
        // fails" rather than hang, not a timing claim.
        let result = tokio::time::timeout(Duration::from_secs(60), drain_via_run_hls_pull(route))
            .await
            .expect("must not hang");
        assert!(
            result.is_err(),
            "wrong credentials must fail run_hls_pull, not silently proceed"
        );
        server.abort();
    }

    /// A stalled origin (accepts the playlist request, then never responds)
    /// must fail within `Timeouts::read_idle`, not hang forever.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_on_the_playlist() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let _server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            std::future::pending::<()>().await;
        });

        let route = HlsPullRoute::new("stalled", format!("http://{addr}/media.m3u8"))
            .with_timeouts(
                broadcast_common::timeouts::Timeouts::default()
                    .with_connect(Timeouts::default().connect)
                    .with_read_idle(Duration::from_secs(2)),
            );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        // DISCRIMINATOR (issue #826): must prove the operation returns
        // through the CONFIGURED read timeout, not via any longer system/
        // library default. Gap widened: configured read timeout raised
        // from 150ms to 2s, assertion window raised from 5s to 10s — 10s
        // is still well below any plausible fallback. MUTATION CHECKED:
        // inflating the configured read timeout to 30s makes the 10s
        // outer timeout fire first, producing `Elapsed`.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_hls_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_hls_pull must not exceed the assertion window");
        assert!(
            matches!(result, Err(crate::error::MultimuxError::Connect { .. })),
            "a stalled playlist fetch must fail, not hang forever"
        );
    }
}
