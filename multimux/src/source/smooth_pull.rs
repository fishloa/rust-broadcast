//! Microsoft Smooth Streaming (MS-SSTR) pull ingest source (issue #759): the
//! media-plane [`IngestSession`] adapter and tokio drive loop around the shared
//! [`smooth_runtime::SmoothClient`] engine (issue #2370).
//!
//! The engine owns every protocol decision: manifest refresh, protection refusal,
//! stream selection, chunk plans, track-id discovery, the synthesized init
//! segment, timestamp rebasing, retry/backoff, live join and discontinuities.
//! This module only maps the engine's [`smooth_runtime::Action`]s to GETs,
//! feeds the bodies and failures back, and turns its outputs into
//! [`SessionEvent`]s.
//!
//! # PlayReady / PIFF sample encryption is NOT supported
//!
//! A manifest `<Protection>` element or a fragment carrying CENC/PIFF
//! sample-encryption boxes ends the session with a typed
//! [`crate::error::MultimuxError::Encrypted`].

use std::collections::VecDeque;
use std::time::Duration;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestSession, ProgramId, SessionEvent, run_dial,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use reqwest::{Client as HttpClient, StatusCode};
use smooth_runtime::{
    Action, Dropped, FetchFailure, Output, RequestId, SmoothClient, SmoothConfig, SmoothError,
};
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::driver::terminal_result;
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, scope_credentials, strip_userinfo,
};
use crate::source::pull_target::{PullKind, PullRoute};
use broadcast_auth::AuthState;

const NANOS_PER_MICRO: u64 = 1_000;

/// How long [`run_smooth_pull`] waits before polling its session again when it has,
/// momentarily, neither an outbound request queued nor a fetch in flight. A bare
/// `continue` there would spin without an `.await`, which on a current-thread
/// runtime starves the in-flight fetches this loop is waiting for.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);

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
        _cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        run_smooth_pull(&route, trunk_config, handshake, &route_handle).await
    }
}

/// What a fetch produced, fed back into [`SmoothIngestSession`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum SmoothInput<'a> {
    /// The manifest body for request `RequestId`.
    Manifest(RequestId, &'a [u8]),
    /// A fragment body for request `RequestId`.
    Fragment(RequestId, &'a [u8]),
    /// The fetch for request `RequestId` produced no usable body.
    Failed(RequestId, FetchFailure),
}

/// The sans-IO Smooth-pull [`IngestSession`]. [`run_smooth_pull`] performs
/// every real GET and feeds the results in.
pub struct SmoothIngestSession {
    client: SmoothClient,
    now_us: u64,
    established: bool,
    pending_events: VecDeque<SessionEvent>,
    failure: Option<MultimuxError>,
}

fn fatal_error(error: &SmoothError) -> MultimuxError {
    match error {
        SmoothError::ManifestProtected | SmoothError::FragmentEncrypted => {
            MultimuxError::Encrypted {
                reason: format!("smooth-pull: {error}"),
            }
        }
        _ => MultimuxError::Connect {
            reason: format!("smooth-pull: {error}"),
        },
    }
}

impl SmoothIngestSession {
    /// Construct a fresh session — performs no I/O.
    pub fn new(manifest_url: Url) -> Self {
        Self {
            client: SmoothClient::new(manifest_url, SmoothConfig::default()),
            now_us: 0,
            established: false,
            pending_events: VecDeque::new(),
            failure: None,
        }
    }

    /// True once a static presentation has played to its end.
    pub fn ended(&self) -> bool {
        self.client.is_ended()
    }

    fn collect(&mut self) {
        if !self.established && self.client.is_started() {
            self.established = true;
            self.pending_events.push_back(SessionEvent::Established);
        }
        while let Some(output) = self.client.next_output() {
            self.apply(output);
        }
    }

    fn apply(&mut self, output: Output) {
        match output {
            Output::Tracks(tracks) => self.pending_events.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks,
            }),
            Output::TracksChanged(tracks) => {
                self.pending_events.push_back(SessionEvent::TracksChanged {
                    program: ProgramId(0),
                    tracks,
                });
            }
            Output::Sample { track, sample } => {
                self.pending_events.push_back(SessionEvent::Sample {
                    program: ProgramId(0),
                    track_id: track,
                    retention: RetentionClass::Timed,
                    sample,
                });
            }
            Output::Discontinuity => {
                tracing::warn!("smooth-pull: timeline discontinuity");
            }
            Output::Dropped(dropped) => Self::note_dropped(&dropped),
            Output::Fatal(error) => {
                self.failure.get_or_insert_with(|| fatal_error(&error));
            }
            _ => {}
        }
    }

    fn note_dropped(dropped: &Dropped) {
        match dropped {
            Dropped::Fragment { stream, status } => {
                tracing::warn!(
                    stream = stream.0,
                    ?status,
                    "smooth-pull: fragment kept failing; skipped"
                );
                metrics::counter!(crate::prometheus::PULL_FRAGMENT_ABANDONED_TOTAL).increment(1);
            }
            Dropped::StreamLost { stream } => {
                tracing::warn!(
                    stream = stream.0,
                    "smooth-pull: refreshed manifest has no matching StreamIndex; abandoning \
                     this stream rather than stalling silently"
                );
                metrics::counter!(crate::prometheus::PULL_STREAM_REFRESH_MISS_TOTAL).increment(1);
            }
            other => tracing::warn!(?other, "smooth-pull: dropped"),
        }
    }

    fn take_failure(&mut self) -> Result<()> {
        self.failure.take().map_or(Ok(()), Err)
    }
}

impl Stage for SmoothIngestSession {
    type In<'a> = SmoothInput<'a>;
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, input: SmoothInput<'_>, now: Timestamp) -> Result<()> {
        self.now_us = now.as_nanos() / NANOS_PER_MICRO;
        match input {
            SmoothInput::Manifest(id, bytes) => self.client.on_manifest(self.now_us, id, bytes),
            SmoothInput::Fragment(id, bytes) => self.client.on_fragment(self.now_us, id, bytes),
            SmoothInput::Failed(id, failure) => self.client.on_failure(self.now_us, id, failure),
        }
        self.collect();
        self.take_failure()
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending_events.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.take_failure()
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        self.client
            .next_deadline()
            .map(|us| Timestamp::from_nanos(us.saturating_mul(NANOS_PER_MICRO)))
    }

    fn on_deadline(&mut self, now: Timestamp) {
        self.now_us = now.as_nanos() / NANOS_PER_MICRO;
    }

    fn demand(&self) -> Demand {
        Demand::new(crate::source::MAX_TS_READ)
    }
}

impl IngestSession for SmoothIngestSession {
    type Request = Action;

    fn poll_transmit(&mut self) -> Option<Action> {
        let action = self.client.poll(self.now_us);
        self.collect();
        action
    }
}

/// Constructs a [`SmoothIngestSession`] — performs **no I/O**.
pub struct SmoothPullDialer {
    manifest_url: Url,
}

impl Dialer for SmoothPullDialer {
    type Session = SmoothIngestSession;
    type Error = MultimuxError;

    fn dial(&mut self) -> Result<SmoothIngestSession> {
        Ok(SmoothIngestSession::new(self.manifest_url.clone()))
    }
}

struct FetchError {
    status: Option<u16>,
    error: MultimuxError,
    retry_after: Option<Duration>,
}

fn status_error(what: &str, status: StatusCode, retry_after: Option<Duration>) -> FetchError {
    let error = if status == StatusCode::UNAUTHORIZED {
        MultimuxError::Auth {
            reason: format!("smooth-pull {what}: {status}"),
        }
    } else {
        MultimuxError::Connect {
            reason: format!("smooth-pull {what}: HTTP {status}"),
        }
    };
    FetchError {
        status: Some(status.as_u16()),
        error,
        retry_after,
    }
}

async fn fetch_one_bytes(
    client: &HttpClient,
    url: &str,
    auth: &mut AuthState,
    what: &str,
    now_us: u64,
) -> std::result::Result<Vec<u8>, FetchError> {
    let response = authenticated_get(client, url, auth, now_us)
        .await
        .map_err(|error| FetchError {
            status: None,
            error,
            retry_after: None,
        })?;
    let status = response.status();
    if !status.is_success() {
        let retry_after = crate::source::server_retry_after(status, response.headers());
        return Err(status_error(what, status, retry_after));
    }
    crate::source::read_body_capped(response, crate::source::MAX_HTTP_BODY_BYTES, what)
        .await
        .map_err(|error| FetchError {
            status: None,
            error,
            retry_after: None,
        })
}

fn build_client(route: &SmoothPullRoute) -> Result<(HttpClient, Url, AuthState)> {
    let parsed = Url::parse(&route.target.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad Smooth-pull URL {}: {e}",
            crate::redact::redact_url(&route.target.url)
        ),
    })?;
    let credentials =
        resolve_credentials(route.target.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let credentials = scope_credentials(credentials, &clean_url);
    let http = HttpClient::builder()
        .redirect(crate::source::redirect_policy(true))
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    Ok((http, clean_url, credentials))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchKind {
    Manifest,
    Fragment,
}

/// The scheduler key for one Smooth fetch: which request it answers and, when it
/// failed over HTTP, the status.
#[derive(Debug, Clone, Copy)]
struct SmoothFetch {
    id: RequestId,
    kind: FetchKind,
    status: Option<u16>,
}

type SmoothPendingFetch = crate::source::pull::PendingFetch<SmoothFetch>;

fn pending_fetch(
    http: HttpClient,
    auth: AuthState,
    now_us: u64,
    id: RequestId,
    kind: FetchKind,
    url: String,
    read_timeout: Duration,
) -> SmoothPendingFetch {
    let what = match kind {
        FetchKind::Manifest => "manifest",
        FetchKind::Fragment => "fragment",
    };
    let mut auth = auth;
    crate::source::pull::PendingFetch {
        delay: Duration::ZERO,
        fut: Box::pin(async move {
            use crate::source::pull::FetchResult;
            let fetched = tokio::time::timeout(
                read_timeout,
                fetch_one_bytes(&http, &url, &mut auth, what, now_us),
            )
            .await;
            let (status, result) = match fetched {
                Ok(Ok(bytes)) => (None, FetchResult::Ready(bytes)),
                Ok(Err(FetchError {
                    status,
                    error,
                    retry_after,
                })) => (
                    status,
                    FetchResult::Failed {
                        error,
                        learned: Vec::new(),
                        retry_after,
                    },
                ),
                Err(_) => (
                    None,
                    FetchResult::Failed {
                        error: MultimuxError::Connect {
                            reason: format!("smooth-pull {what} read exceeded {read_timeout:?}"),
                        },
                        learned: Vec::new(),
                        retry_after: None,
                    },
                ),
            };
            (SmoothFetch { id, kind, status }, result)
        }),
    }
}

/// Drives `route` to completion — see `dash_pull::run_dash_pull`'s doc for
/// the shared shape (dial → poll_transmit → fetch → feed, bounded fan-out,
/// no session-internal clock/sleep).
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_smooth_pull(
    route: &SmoothPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> Result<()> {
    let (http, clean_url, credentials) = build_client(route)?;
    let mut dialer = SmoothPullDialer {
        manifest_url: clean_url,
    };
    let mut driver = run_dial(
        &mut dialer,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    )?;

    let read_timeout = route.target.timeouts.read;
    let mut scheduler: crate::source::pull::PullScheduler<SmoothFetch> =
        crate::source::pull::PullScheduler::new(crate::source::MAX_INFLIGHT_FETCHES);
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();

    loop {
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        if driver.next_deadline().is_some_and(|d| d <= now) {
            driver.on_deadline(now);
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
        }
        if !driver.health().is_running() {
            return crate::source::finish_vod(route_handle, terminal_result(driver, "smooth-pull"));
        }
        while let Some(action) = driver.poll_transmit() {
            let (id, kind, url) = match action {
                Action::FetchManifest { id, url } => (id, FetchKind::Manifest, url),
                Action::FetchFragment { id, url, .. } => (id, FetchKind::Fragment, url),
                _ => continue,
            };
            scheduler.push(pending_fetch(
                http.clone(),
                credentials.clone(),
                crate::clock::SystemHostClock::now_us(),
                id,
                kind,
                url.to_string(),
                read_timeout,
            ));
        }

        scheduler.pump();

        if scheduler.is_idle() {
            if driver.session().ended() {
                driver.finish();
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
                return crate::source::finish_vod(
                    route_handle,
                    terminal_result(driver, "smooth-pull"),
                );
            }
            match driver.next_deadline() {
                Some(deadline) => {
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    if now < deadline {
                        tokio::time::sleep(deadline.saturating_sub(now)).await;
                    }
                }
                None => {
                    scheduler.next(None, IDLE_POLL_INTERVAL).await;
                }
            }
            continue;
        }

        let joined = scheduler.next(None, IDLE_POLL_INTERVAL).await;
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        match joined {
            Some(crate::source::pull::FetchOutcome::Ready(fetch, bytes)) => {
                let input = match fetch.kind {
                    FetchKind::Manifest => SmoothInput::Manifest(fetch.id, bytes.as_slice()),
                    FetchKind::Fragment => SmoothInput::Fragment(fetch.id, bytes.as_slice()),
                };
                driver.feed(input, now);
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
            }
            Some(crate::source::pull::FetchOutcome::Failed(fetch, e, _, retry_after)) => {
                if fetch.status == Some(StatusCode::UNAUTHORIZED.as_u16()) {
                    return Err(e);
                }
                let failure = FetchFailure {
                    status: fetch.status,
                    retry_after_us: retry_after.map_or(0, |wait| {
                        u64::try_from(wait.as_micros()).unwrap_or(u64::MAX)
                    }),
                };
                driver.feed(SmoothInput::Failed(fetch.id, failure), now);
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
            }
            Some(crate::source::pull::FetchOutcome::TaskPanic(detail)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("smooth-pull: fetch task failed: {detail}"),
                });
            }
            None => {}
        }

        if !driver.health().is_running() {
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            return crate::source::finish_vod(route_handle, terminal_result(driver, "smooth-pull"));
        }

        if driver.session().ended() {
            driver.finish();
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            return crate::source::finish_vod(route_handle, terminal_result(driver, "smooth-pull"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::IngestTimeouts;
    use crate::testutil::drain;
    use axum::Router;
    use axum::extract::{Path as AxumPath, State};
    use axum::http::StatusCode as AxumStatusCode;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use broadcast_common::Package;
    use broadcast_common::Unpackage;
    use broadcast_smooth::{SmoothManifest, StreamType};
    use media_plane::ingress::HandshakePolicy;
    use media_plane::trunk::{SampleCursor, TrunkConfig};
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use transmux::pipeline::build_init_segment;
    use transmux::pipeline::{CodecConfig, TrackSpec};
    use transmux::{Fmp4Demux, TrackId, track_spec_from_quality_level};

    const SYNTHETIC_MOVIE_TIMESCALE: u32 = transmux::VIDEO_CLOCK_RATE;
    use transmux::{Media, SmoothOutput, SmoothPackager, TsDemux};

    #[test]
    fn a_discontinuity_output_is_logged_and_adds_no_session_event() {
        let mut session =
            SmoothIngestSession::new(Url::parse("http://origin.test/Manifest").expect("url"));
        session.apply(Output::Discontinuity);
        assert!(session.pending_events.is_empty());
        assert!(session.failure.is_none());
    }

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
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
        let local_spec =
            track_spec_from_quality_level(track_id, timescale, stream_type, quality).expect("spec");
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

    /// Drives the raw [`SmoothIngestSession`] over real HTTP, returning the
    /// `TrackSpec`s it announced and a per-`track_id` `SessionEvent::Sample`
    /// count.
    ///
    /// Counts `SessionEvent`s rather than `SampleCursor` items for the same
    /// reason `hls_pull`'s equivalent helper does: `Trunk::subscribe()` starts
    /// from *now*, and this session emits `NewProgram` **and** every stream's
    /// already-fetched first fragment's samples inside the same
    /// `finish_awaiting_first_fragments` — i.e. the same `feed` — so no cursor
    /// can exist in time to see that first batch. `Trunk` arrival is asserted
    /// separately, below.
    async fn drive_session_and_count(
        route: &SmoothPullRoute,
    ) -> Result<(Vec<TrackSpec>, HashMap<TrackId, usize>)> {
        let (http, clean_url, mut credentials) = build_client(route)?;
        let mut session = SmoothIngestSession::new(clean_url);
        let mut backlog: VecDeque<Action> = VecDeque::new();
        let mut specs: Vec<TrackSpec> = Vec::new();
        let mut per_track: HashMap<TrackId, usize> = HashMap::new();
        // HANG GUARD (issue #826): ceiling on the whole session-drive loop.
        // Each HTTP fetch (manifest + first fragments + follow-up fragments)
        // completes in ~ms over loopback. Raised to 60s for load tolerance
        // — only job is to fail "never finishes" rather than hang.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

        loop {
            while let Some(a) = session.poll_transmit() {
                backlog.push_back(a);
            }
            while let Some(event) = session.poll() {
                match event {
                    SessionEvent::NewProgram { tracks, .. } => specs = tracks,
                    SessionEvent::Sample { track_id, .. } => {
                        *per_track.entry(track_id).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            let Some(action) = backlog.pop_front() else {
                if session.ended() || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            };
            let now = Timestamp::from_nanos(0);
            let (id, is_manifest, b) = fetch_action(&http, &mut credentials, action).await;
            let input = if is_manifest {
                SmoothInput::Manifest(id, b.as_slice())
            } else {
                SmoothInput::Fragment(id, b.as_slice())
            };
            session.feed(input, now)?;
        }
        Ok((specs, per_track))
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
        let (specs, per_track) =
            // HANG GUARD (issue #826): backstop around the session drive.
            // The session fetches manifest + fragments from a real axum
            // server; each fetch ~ms, total drive time bounded by the
            // content's fragment count. Raised to 60s for load tolerance
            // since it only exists to fail "never finishes" rather than hang.
            tokio::time::timeout(Duration::from_secs(60), drive_session_and_count(&route))
                .await
                .expect("drive timed out")
                .expect("drive");

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

    /// The `Trunk`-side counterpart to [`drive_session_and_count`]: drives the
    /// same route through a real [`media_plane::ingress::IngestDriver`] and
    /// asserts real samples land on a real [`SampleCursor`]. `> 0`, not an
    /// exact count, for the reason that helper's doc gives.
    #[tokio::test]
    async fn samples_reach_the_trunk_through_the_ingest_driver() {
        let (url, server, _media) = start_fixture_server(None).await;
        let route = SmoothPullRoute::new("smooth-cam", url);

        let (http, clean_url, mut credentials) = build_client(&route).expect("build client");
        let mut dialer = SmoothPullDialer {
            manifest_url: clean_url,
        };
        let mut driver = run_dial(
            &mut dialer,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        )
        .expect("dial");

        let mut backlog: VecDeque<Action> = VecDeque::new();
        let mut cursor: Option<SampleCursor> = None;
        let mut total = 0usize;
        // HANG GUARD (issue #826): ceiling on the Trunk-drive loop, same
        // reasoning as `drive_session_and_count`'s deadline.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            while let Some(a) = driver.poll_transmit() {
                backlog.push_back(a);
            }
            let Some(action) = backlog.pop_front() else {
                if driver.session().ended() || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            };
            let now = Timestamp::from_nanos(0);
            let (id, is_manifest, b) = fetch_action(&http, &mut credentials, action).await;
            let input = if is_manifest {
                SmoothInput::Manifest(id, b.as_slice())
            } else {
                SmoothInput::Fragment(id, b.as_slice())
            };
            driver.feed(input, now);
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                total += drain(c);
            }
        }

        assert!(
            total > 0,
            "real samples must reach the Trunk through IngestDriver, got {total}"
        );
        server.abort();
    }

    /// The full `run_smooth_pull` drive loop against the same fixture: a
    /// static (non-`IsLive`) manifest must end cleanly.
    #[tokio::test]
    async fn run_smooth_pull_completes_cleanly_on_a_static_manifest() {
        let (url, server, _media) = start_fixture_server(None).await;
        let route = SmoothPullRoute::new("smooth-cam", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // HANG GUARD (issue #826): backstop around `run_smooth_pull`.
        // The static manifest is exhausted by the session's fragment-fetch
        // loop; `run_smooth_pull` returns on its own once complete.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
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
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
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
    /// `IngestTimeouts::read`, not hang forever.
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
        let route = SmoothPullRoute::new("smooth-stalled", url).with_timeouts(IngestTimeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_secs(1),
        });
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // DISCRIMINATOR (issue #826): must prove the operation returns
        // through the CONFIGURED read timeout, not via any longer system
        // default. Gap widened: configured read timeout raised from 150ms
        // to 2s, assertion window raised from 10s to 15s — 15s is still
        // well below any plausible fallback. MUTATION CHECKED: inflating
        // the configured read timeout to 30s makes the 15s outer timeout
        // fire first, producing `Elapsed`.
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect(
            "run_smooth_pull must return via IngestTimeouts::read, not via \
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
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // HANG GUARD (issue #826): backstop around `run_smooth_pull` for
        // the DRM manifest test. The manifest parse check returns
        // immediately so `run_smooth_pull` fails within ~ms.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("must not hang");
        match result {
            Err(MultimuxError::Encrypted { .. }) => {}
            Ok(_) => panic!("expected MultimuxError::Encrypted, got Ok(())"),
            Err(other) => panic!("expected MultimuxError::Encrypted, got {other:?}"),
        }
        server.abort();
    }

    async fn fetch_action(
        http: &HttpClient,
        auth: &mut AuthState,
        action: Action,
    ) -> (RequestId, bool, Vec<u8>) {
        let (id, is_manifest, url) = match action {
            Action::FetchManifest { id, url } => (id, true, url),
            Action::FetchFragment { id, url, .. } => (id, false, url),
            _ => panic!("unexpected action"),
        };
        let body = fetch_one_bytes(
            http,
            url.as_str(),
            auth,
            "fetch",
            crate::clock::SystemHostClock::now_us(),
        )
        .await
        .unwrap_or_else(|e| panic!("fetch {url}: {}", e.error));
        (id, is_manifest, body)
    }
}
