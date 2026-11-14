//! Microsoft Smooth Streaming (MS-SSTR) pull ingest source (issue #759): a remote client Manifest
//! and its fragments pulled by a relay-mode [`broadcast_client::Session`] through the shared
//! session host (`crate::source::client_host`).
//!
//! The session owns every Smooth decision: manifest refresh, stream selection, chunk plans,
//! track-id discovery, the synthesized init segment, timestamp rebasing, retry with `Retry-After`,
//! live join and discontinuities. The host performs its HTTP requests.
//!
//! # PlayReady / PIFF sample encryption is NOT supported
//!
//! A manifest `<Protection>` element or a fragment carrying CENC/PIFF sample-encryption boxes
//! ends the pull with the typed [`crate::error::MultimuxError::Session`] naming
//! `ErrorKind::UnsupportedEncryption`.

use std::convert::Infallible;

use broadcast_client::{Config, SourceKind};
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;

use crate::error::{MultimuxError, Result};
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep};
use crate::source::pull_target::{PullKind, PullRoute};

const WHAT: &str = "smooth-pull";

/// A remote MS-SSTR client Manifest to pull: its URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
pub type SmoothPullRoute = PullRoute<SmoothPullKind>;

/// The [`PullKind`] of a [`SmoothPullRoute`].
#[derive(Debug, Clone, Copy)]
pub struct SmoothPullKind;

impl PullKind for SmoothPullKind {
    const ROUTE: &'static str = "SmoothPullRoute";
    type Extra = ();
}

impl crate::source::pull_target::PullDrive for SmoothPullKind {
    const SEGMENTED: bool = true;

    async fn drive(
        route: std::sync::Arc<crate::source::pull_target::PullRoute<Self>>,
        trunk_config: media_plane::trunk::TrunkConfig,
        handshake: media_plane::ingress::HandshakePolicy,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        run_smooth_pull(&route, trunk_config, handshake, &route_handle, cancel).await
    }
}

fn session_config(route: &SmoothPullRoute) -> Config {
    let mut config = Config::relay();
    config.kind = SourceKind::Smooth;
    let mut config = config.with_timeouts(route.target.timeouts);
    config.max_queued_bytes = crate::source::SEGMENTED_EVENT_QUEUE_BYTES;
    config.max_manifest_bytes = crate::source::MAX_HTTP_BODY_BYTES;
    config.max_segment_bytes = crate::source::MAX_HTTP_BODY_BYTES;
    config
}

/// Drives `route` until a static manifest has played to its end or the session fails outright; a
/// live manifest never ends on its own.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 — see
/// `crate::source::rtsp::run_rtsp`'s own doc for what `crate::source::report_driver_progress`
/// does with it each iteration.
pub async fn run_smooth_pull(
    route: &SmoothPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    run_with_clock(
        route,
        trunk_config,
        handshake,
        route_handle,
        cancel,
        std::sync::Arc::new(crate::clock::SystemHostClock::new()),
    )
    .await
}

/// [`run_smooth_pull`] with the session's instants taken from `clock`: the seam a test uses to run
/// the pull on a paused tokio clock.
pub(crate) async fn run_with_clock(
    route: &SmoothPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
    clock: std::sync::Arc<dyn crate::clock::HostClock>,
) -> Result<()> {
    let timeouts = route.target.timeouts;
    let mut step = SessionStep::open(
        &route.target.url,
        session_config(route),
        route.target.auth.clone(),
        SessionLabels::new(WHAT, timeouts).session_retries(),
        std::sync::Arc::clone(route_handle),
        false,
    )?
    .with_clock(clock);
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
    use super::*;
    use crate::route::RouteConfig;

    #[test]
    fn the_session_queue_is_budgeted_in_bytes() {
        let config = session_config(&SmoothPullRoute::new("q", "http://127.0.0.1:1/Manifest"));
        assert_eq!(
            config.max_queued_bytes,
            crate::source::SEGMENTED_EVENT_QUEUE_BYTES
        );
    }
    use axum::Router;
    use axum::extract::{Path as AxumPath, State};
    use axum::http::StatusCode as AxumStatusCode;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use broadcast_common::Package;
    use broadcast_common::Timestamp;
    use broadcast_common::Unpackage;
    use broadcast_smooth::{SmoothManifest, StreamType};
    use media_plane::ingress::{HandshakePolicy, ProgramId};
    use media_plane::trunk::{SampleCursorItem, TrunkConfig};
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::time::Duration;
    use transmux::pipeline::build_init_segment;
    use transmux::pipeline::{CodecConfig, TrackSpec};
    use transmux::{Fmp4Demux, TrackId, track_spec_from_quality_level};

    const SYNTHETIC_MOVIE_TIMESCALE: broadcast_common::timescale::Timescale =
        transmux::VIDEO_CLOCK_TIMESCALE;
    use transmux::{Media, SmoothOutput, SmoothPackager, TsDemux};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
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

    fn fixture_ts_path() -> std::path::PathBuf {
        test_fixtures::path("ts/h264-aac.ts")
    }

    fn build_smooth_output() -> (Media, SmoothOutput) {
        let ts = std::fs::read(fixture_ts_path()).expect("h264-aac.ts fixture must exist");
        let media = TsDemux::new()
            .unpackage(ts.as_slice())
            .expect("demux h264-aac.ts");
        let mut pkg = SmoothPackager::default();
        let out = pkg.package(&media).expect("package Smooth");
        (media, out)
    }

    fn video_track_id(media: &Media) -> TrackId {
        media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("video track")
            .spec
            .track_id
    }

    fn audio_track_id(media: &Media) -> TrackId {
        media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
            .expect("audio track")
            .spec
            .track_id
    }

    fn parse_kind_time(path: &str) -> Option<(String, u64)> {
        let frag_part = path.split("Fragments(").nth(1)?;
        let inner = frag_part.strip_suffix(')')?;
        let (kind, time_str) = inner.split_once('=')?;
        Some((kind.to_string(), time_str.parse().ok()?))
    }

    type FragmentHits = Arc<std::sync::Mutex<Vec<((String, u64), std::time::Instant)>>>;

    #[derive(Clone)]
    struct FixtureState {
        manifest: Arc<String>,
        fragments: Arc<HashMap<(String, u64), Vec<u8>>>,
        stall: Option<(String, u64)>,
        throttle_once: Option<(String, u64)>,
        throttled: Arc<std::sync::atomic::AtomicBool>,
        fragment_hits: FragmentHits,
    }

    async fn fixture_handler(
        AxumPath(path): AxumPath<String>,
        State(state): State<FixtureState>,
    ) -> AxumResponse {
        if path == "Manifest" {
            return (*state.manifest).clone().into_response();
        }
        let Some((kind, time)) = parse_kind_time(&path) else {
            return AxumStatusCode::NOT_FOUND.into_response();
        };
        if state.stall.as_ref() == Some(&(kind.clone(), time)) {
            std::future::pending::<()>().await;
            unreachable!("pending future never resolves");
        }
        state
            .fragment_hits
            .lock()
            .expect("lock")
            .push(((kind.clone(), time), std::time::Instant::now()));
        if state.throttle_once.as_ref() == Some(&(kind.clone(), time))
            && !state
                .throttled
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return (AxumStatusCode::SERVICE_UNAVAILABLE, [("retry-after", "2")]).into_response();
        }
        match state.fragments.get(&(kind, time)) {
            Some(bytes) => bytes.clone().into_response(),
            None => AxumStatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn start_fixture_server(
        stall: Option<(&str, u64)>,
    ) -> (String, tokio::task::JoinHandle<()>, Media) {
        let (url, server, media, _) = start_throttling_fixture_server(stall, None).await;
        (url, server, media)
    }

    async fn start_throttling_fixture_server(
        stall: Option<(&str, u64)>,
        throttle_once: Option<(&str, u64)>,
    ) -> (String, tokio::task::JoinHandle<()>, Media, FragmentHits) {
        let (media, out) = build_smooth_output();
        let video_id = video_track_id(&media);
        let audio_id = audio_track_id(&media);

        let mut fragments = HashMap::new();
        for frag in &out.fragments {
            let kind = if frag.track_id == TrackId::new(video_id.value()) {
                "video"
            } else if frag.track_id == TrackId::new(audio_id.value()) {
                "audio"
            } else {
                continue;
            };
            fragments.insert((kind.to_string(), frag.start_time), frag.data.clone());
        }

        let fragment_hits = FragmentHits::default();
        let state = FixtureState {
            manifest: Arc::new(out.manifest.clone()),
            fragments: Arc::new(fragments),
            stall: stall.map(|(k, t)| (k.to_string(), t)),
            throttle_once: throttle_once.map(|(k, t)| (k.to_string(), t)),
            throttled: Arc::default(),
            fragment_hits: Arc::clone(&fragment_hits),
        };
        let app = Router::new()
            .route("/{*path}", get(fixture_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (
            format!("http://{addr}/Manifest"),
            server,
            media,
            fragment_hits,
        )
    }

    async fn start_manifest_only_server(
        manifest: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(
            AxumPath(path): AxumPath<String>,
            State(manifest): State<&'static str>,
        ) -> AxumResponse {
            if path == "Manifest" {
                manifest.into_response()
            } else {
                AxumStatusCode::NOT_FOUND.into_response()
            }
        }
        let app = Router::new()
            .route("/{*path}", get(handler))
            .with_state(manifest);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/Manifest"), server)
    }

    const COUNT_TIMED_RING: usize = 4096;

    /// Pulls `route` to the end of its static manifest and reads back what the route published:
    /// the result, the track set, and the number of samples per track the `Trunk` holds.
    async fn pull_to_end(
        route: &SmoothPullRoute,
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
            run_smooth_pull(
                route,
                config,
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_smooth_pull must not hang against a static manifest");
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

    fn oracle_sample_count(
        media: &Media,
        out: &SmoothOutput,
        track_id: TrackId,
        timescale: u32,
    ) -> usize {
        let track = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == track_id)
            .unwrap();
        let stream_type = match track.spec.config {
            CodecConfig::Avc { .. } => StreamType::Video,
            CodecConfig::Aac { .. } => StreamType::Audio,
            _ => panic!("unexpected codec"),
        };
        let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");
        let si = manifest
            .streams
            .iter()
            .find(|s| s.stream_type == stream_type)
            .expect("StreamIndex");
        let quality = &si.qualities[0];
        let local_spec = track_spec_from_quality_level(
            track_id,
            broadcast_common::timescale::Timescale::try_new(timescale).expect("timescale"),
            stream_type,
            quality,
        )
        .expect("spec");
        let init = build_init_segment(std::slice::from_ref(&local_spec), SYNTHETIC_MOVIE_TIMESCALE)
            .expect("init");

        let mut total = 0usize;
        for frag in out.fragments.iter().filter(|f| f.track_id == track_id) {
            let mut combined = init.clone();
            combined.extend_from_slice(&frag.data);
            let demuxed = Fmp4Demux::new()
                .unpackage(combined.as_slice())
                .expect("oracle demux");
            total += demuxed
                .tracks
                .iter()
                .map(|t| t.samples.len())
                .sum::<usize>();
        }
        total
    }

    /// Biting loopback test: a real axum server serves a runtime-generated
    /// Smooth manifest + fragments; asserts the session resolves BOTH the AVC
    /// and AAC `TrackSpec`s (from `CodecPrivateData` alone — no init segment
    /// ever crossed the wire) with unique remapped track ids, and produces
    /// **exactly** the independently-demuxed oracle's per-track sample counts
    /// — the #758-lesson "audio silently dropped" class of bug this guards
    /// against.
    ///
    /// MUTATION-CHECKED: dropping the `Fmp4Demux::unpackage`/emit in
    /// `emit_fragment_samples`, or never advancing `StreamState::plan` in
    /// `pump_fragment_fetches`, makes the counts stay `0`; collapsing the
    /// local->global remap makes `per_track.len()` go 2 -> 1.
    #[tokio::test]
    async fn loopback_smooth_pull_resolves_both_tracks_and_matches_oracle_sample_count() {
        let (url, server, media) = start_fixture_server(None).await;
        let (_media2, out) = build_smooth_output();

        let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");
        let audio_timescale = manifest
            .streams
            .iter()
            .find(|s| s.stream_type == StreamType::Audio)
            .expect("audio StreamIndex")
            .qualities[0]
            .sampling_rate
            .expect("audio SamplingRate");
        let want_video = oracle_sample_count(
            &media,
            &out,
            TrackId::new(video_track_id(&media).value()),
            transmux::VIDEO_CLOCK_RATE,
        );
        let want_audio = oracle_sample_count(
            &media,
            &out,
            TrackId::new(audio_track_id(&media).value()),
            audio_timescale,
        );
        assert!(
            want_video > 0 && want_audio > 0,
            "sanity: fixture must carry real samples for both streams"
        );

        let route = SmoothPullRoute::new("smooth-cam", url);
        let (result, specs, per_track) = pull_to_end(&route).await;
        assert!(result.is_ok(), "{result:?}");

        assert_eq!(specs.len(), 2, "one video + one audio track: {specs:?}");
        let mut ids: Vec<TrackId> = specs.iter().map(|s| s.track_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 2, "track ids must be unique global ids");

        let video_global = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
            .expect("an AVC track")
            .track_id;
        let audio_global = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Aac { .. }))
            .expect("an AAC track")
            .track_id;
        assert_eq!(
            per_track.len(),
            2,
            "samples must land on exactly 2 distinct track ids: {per_track:?}"
        );
        assert_eq!(
            per_track.get(&video_global).copied().unwrap_or(0),
            want_video,
            "video sample count must match the independent oracle exactly"
        );
        assert_eq!(
            per_track.get(&audio_global).copied().unwrap_or(0),
            want_audio,
            "audio sample count must match the independent oracle exactly"
        );

        server.abort();
    }

    /// The full `run_smooth_pull` drive loop against the same fixture: a
    /// static (non-`IsLive`) manifest must end cleanly.
    #[tokio::test]
    async fn run_smooth_pull_completes_cleanly_on_a_static_manifest() {
        let (url, server, _media) = start_fixture_server(None).await;
        let route = SmoothPullRoute::new("smooth-cam", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        // HANG GUARD (issue #826): backstop around `run_smooth_pull`.
        // The static manifest is exhausted by the session's fragment-fetch
        // loop; `run_smooth_pull` returns on its own once complete.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_smooth_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_smooth_pull must not hang");
        assert!(
            result.is_ok(),
            "a static manifest must end cleanly: {result:?}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn run_smooth_pull_honours_retry_after_on_a_503() {
        let (media, out) = build_smooth_output();
        let video_id = video_track_id(&media);
        let first = out
            .fragments
            .iter()
            .find(|f| f.track_id == TrackId::new(video_id.value()))
            .expect("a video fragment")
            .start_time;
        let (url, server, _media, hits) =
            start_throttling_fixture_server(None, Some(("video", first))).await;
        let route = SmoothPullRoute::new("smooth-retry-after", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_smooth_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_smooth_pull must not hang");
        assert!(result.is_ok(), "{result:?}");
        let times: Vec<std::time::Instant> = hits
            .lock()
            .expect("lock")
            .iter()
            .filter(|(key, _)| *key == ("video".to_string(), first))
            .map(|(_, at)| *at)
            .collect();
        assert_eq!(times.len(), 2, "{times:?}");
        assert!(
            times[1].duration_since(times[0]) >= Duration::from_secs(2),
            "the retry came {:?} after the 503",
            times[1].duration_since(times[0])
        );
        server.abort();
    }

    /// Biting test (issue #663 P5 / #738-#739 ingest-hardening lesson): a
    /// server that resolves the manifest + both streams' first fragments but
    /// then stalls on a later fragment must fail within
    /// `Timeouts::read_idle`, not hang forever.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_on_a_later_fragment() {
        let (media, out) = build_smooth_output();
        let audio_id = audio_track_id(&media);
        let audio_frags: Vec<_> = out
            .fragments
            .iter()
            .filter(|f| f.track_id == TrackId::new(audio_id.value()))
            .collect();
        assert!(
            audio_frags.len() >= 2,
            "fixture must produce at least 2 audio fragments: got {}",
            audio_frags.len()
        );
        let stall_time = audio_frags[1].start_time;

        let (url, server, _media2) = start_fixture_server(Some(("audio", stall_time))).await;
        let route = SmoothPullRoute::new("smooth-stalled", url).with_timeouts(
            broadcast_common::timeouts::Timeouts::default()
                .with_connect(Duration::from_secs(5))
                .with_read_idle(Duration::from_secs(1)),
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        // DISCRIMINATOR (issue #826): must prove the operation returns
        // through the CONFIGURED read timeout, not via any longer system
        // default. Gap widened: configured read timeout raised from 150ms
        // to 2s, assertion window raised from 10s to 15s — 15s is still
        // well below any plausible fallback. MUTATION CHECKED: inflating
        // the configured read timeout to 30s makes the 15s outer timeout
        // fire first, producing `Elapsed`.
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_smooth_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect(
            "run_smooth_pull must return via Timeouts::read_idle, not via \
             the outer assertion window",
        );
        assert!(
            result.is_err(),
            "a server that stalls on a later fragment fetch must fail run_smooth_pull, not \
             hang forever"
        );
        server.abort();
    }

    /// Biting test: a manifest declaring `<Protection>` must fail with a
    /// clear typed error, never silently succeed into a session that would
    /// go on to demux garbage (encrypted) sample bytes.
    #[tokio::test]
    async fn encrypted_manifest_with_protection_element_fails_with_clear_typed_error() {
        const MANIFEST: &str = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" Duration="10000000" TimeScale="10000000">
            <Protection>
                <ProtectionHeader SystemID="9A04F079-9840-4286-AB92-E65BE0885F95">BASE64==</ProtectionHeader>
            </Protection>
            <StreamIndex Type="video" Url="Fragments(video={start time})">
                <QualityLevel Index="0" Bitrate="1" FourCC="H264" CodecPrivateData="000000016742C01EAB0000000168CE3C80"/>
                <c t="0" d="10000000"/>
            </StreamIndex>
        </SmoothStreamingMedia>"#;
        let (url, server) = start_manifest_only_server(MANIFEST).await;
        let route = SmoothPullRoute::new("smooth-drm", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        // HANG GUARD (issue #826): backstop around `run_smooth_pull` for
        // the DRM manifest test. The manifest parse check returns
        // immediately so `run_smooth_pull` fails within ~ms.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_smooth_pull(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("must not hang");
        match result {
            Err(MultimuxError::Session {
                kind: broadcast_client::ErrorKind::UnsupportedEncryption { .. },
                ..
            }) => {}
            Ok(_) => panic!("expected an UnsupportedEncryption session error, got Ok(())"),
            Err(other) => panic!("expected an UnsupportedEncryption session error, got {other:?}"),
        }
        server.abort();
    }
}
