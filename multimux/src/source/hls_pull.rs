//! HLS-pull ingest source (issue #663 P3c / #717 / #760; re-ported onto the
//! media-plane ingress traits at plan step 5a round 3): a driven
//! [`hls_runtime::client::HlsClient`] — the sans-IO Low-Latency HLS
//! (RFC 8216bis) playback engine — feeding real fetch bytes in and turning
//! its `Output`s into [`SessionEvent`]s.
//!
//! # Why this reuses `HlsClient`, not `TokioClient`
//!
//! Before this port, this module wrapped
//! `hls_runtime::client::tokio_client::TokioClient` — the executor-bound
//! adapter that owns its own `reqwest` fetch loop internally. That fit the
//! pre-5a `connect()`/`next_samples()` shape, but it cannot fit
//! [`IngestSession`]: `TokioClient` performs its own I/O, so there is nothing
//! for `Stage::feed`/[`IngestSession::poll_transmit`] to drive. `HlsClient`
//! is the sans-IO core `TokioClient` itself wraps —
//! `poll() -> Option<Action>` out, `on_playlist`/`on_resource` in — which is
//! exactly the [`IngestSession::Request`]/`Stage::In` shape round 3 added.
//! This module is now the *other* adapter over the same sans-IO engine,
//! parallel to `TokioClient`, driven by [`media_plane::ingress::IngestDriver`] instead of a bespoke
//! `connect`/`next_samples` pair — so the actual LL-HLS logic (reload
//! scheduling, part/segment dedup, fMP4/classic-TS demux) is still owned
//! entirely by `hls-runtime`, never duplicated here.
//!
//! # Establishment is genuinely ordinary driving here
//!
//! [`HlsPullDialer::dial`] performs no I/O at all — `HlsClient::new` only
//! queues the first `Action::FetchPlaylist`. [`SessionEvent::Established`] +
//! `NewProgram` are queued the moment the recovered `TrackSpec`s are known
//! (the first `Output::Init`, exactly like the pre-5a `wait_for_init`, just
//! reached by feeding responses through the ordinary pump instead of an
//! `async fn` polling loop before the session is ever returned). Until then
//! [`media_plane::ingress::IngestDriver::health`] reports `Establishing`, bounded by the same
//! [`HandshakePolicy`] every other ported source uses — no bespoke
//! `IngestTimeouts::connect` wrapper is needed any more.
//!
//! # Correlating a fetch response: `HlsFetchId`
//!
//! `HlsClient::on_playlist` and `on_resource` are two different methods,
//! but a `Stage::In` is one type. [`HlsFetchId`] is this session's own
//! (opaque to `media-plane`) request/response identity — `Playlist` for the
//! one method, `Resource(id)` wrapping [`ResourceId`] for the other — chosen
//! entirely by this module; the plane never sees it.
//!
//! # Bounded in-flight fetches
//!
//! `HlsClient::poll` can hand back many `Action::FetchResource`s in one
//! drain (e.g. every already-available part of a freshly-opened segment)
//! with nothing in the client itself capping how many the caller launches at
//! once. [`run_hls_pull`] never has more than
//! [`crate::source::MAX_INFLIGHT_FETCHES`] fetches running concurrently —
//! the rest queue in `backlog` until a slot frees — see that constant's docs
//! for why (this project's sixth unbounded-allocation vector, this time in a
//! pull source's own fan-out rather than a session's per-item state).
//!
//! # Known limitation (carried over from the pre-port module)
//!
//! A mid-stream `Output::Init` (the client re-emits it only on a codec-
//! parameter change across an `#EXT-X-DISCONTINUITY`) yields no
//! `SessionEvent` at all — matching [`SessionEvent::NewProgram`]'s "exactly
//! one initial program" case for this source; a pulled origin that changes
//! codec parameters mid-stream is not yet supported.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Duration;

use broadcast_auth::Credentials;
use broadcast_common::{Demand, Stage, Timestamp, Unpackage};
use hls_runtime::client::{Action, HlsClient, Output as HlsOutput, ResourceId};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestSession, ProgramId, SessionEvent, run_dial,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use reqwest::Client as HttpClient;
use transmux::media::Fmp4Demux;
use transmux::pipeline::TrackSpec;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};
use crate::source::{IngestTimeouts, Source};

/// How long a `run_*_pull` drive loop parks when its session has, momentarily,
/// neither an outbound request queued nor a fetch in flight — and has not
/// ended. A bare `continue` there would spin the loop with no `.await` in it,
/// which on a current-thread runtime starves every other task on the executor
/// (including the in-flight fetches this loop is waiting for). Short enough
/// that it costs no observable latency, long enough that it is not a spin.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// How many times one resource fetch (init/part/segment) is retried before
/// the session is failed (audit run 7, W14). A single `404` from an eviction
/// race, or a dropped connection, must not tear down and reconnect the whole
/// route — which, with every viewer riding one session, disrupts them all.
/// Bounded so a permanently missing resource still fails the session rather
/// than retrying forever.
const MAX_RESOURCE_RETRY_ATTEMPTS: u32 = 8;

/// Base delay for the **exponential** resource-fetch retry backoff (audit
/// W14d): the delay for attempt N is `base * 2^(N-1)`, capped at
/// [`RESOURCE_RETRY_MAX_DELAY`]. A fixed 200 ms was far too short for an
/// eviction race (the segment reappears on the next playlist reload, seconds
/// later); starting at 500 ms and doubling reaches ~64 s by the 8th attempt,
/// well past a normal reload cycle, without stalling the live edge on the
/// first retry.
const RESOURCE_RETRY_BASE_DELAY: Duration = Duration::from_millis(500);

/// Growth factor of the resource-fetch retry delay (doubling).
const RESOURCE_RETRY_FACTOR: f64 = 2.0;

/// Cap on the exponential resource-fetch retry delay.
const RESOURCE_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);

/// Exponential backoff for resource-fetch attempt `attempt` (1-based),
/// capped at [`RESOURCE_RETRY_MAX_DELAY`].
fn retry_backoff(attempt: u32) -> Duration {
    // The workspace's one capped-exponential implementation (audit r07-O1 /
    // #1141; SP1.5 moved it onto `backon`); `attempt` here is 1-based,
    // `delay_for_attempt` 0-based.
    crate::reconnect::ReconnectSchedule::from_parts(
        RESOURCE_RETRY_BASE_DELAY,
        RESOURCE_RETRY_MAX_DELAY,
        RESOURCE_RETRY_FACTOR,
    )
    .delay_for_attempt(attempt.saturating_sub(1))
}

/// The `PendingFetch` for one resource fetch, after `delay`. The single
/// place a resource is described, so the first try and every retry share the
/// exact same timeout/error handling. The delay is applied by
/// [`PullScheduler::pump`], not here.
fn resource_fetch(
    http: HttpClient,
    creds: Option<Credentials>,
    fetch_id: HlsFetchId,
    url: String,
    read_timeout: Duration,
    delay: Duration,
) -> crate::source::pull::PendingFetch<HlsFetchId> {
    crate::source::pull::PendingFetch {
        delay,
        fut: Box::pin(async move {
            let result =
                tokio::time::timeout(read_timeout, fetch_bytes(&http, &url, creds.as_ref()))
                    .await
                    .unwrap_or_else(|_| {
                        Err(MultimuxError::Connect {
                            reason: format!(
                                "hls-pull: resource {fetch_id:?} read exceeded {read_timeout:?}"
                            ),
                        })
                    });
            (fetch_id, result)
        }),
    }
}

/// The `PendingFetch` for the playlist fetch (never delayed — a manifest
/// issue is not transient; see the W14 note below).
fn playlist_fetch(
    http: HttpClient,
    creds: Option<Credentials>,
    url: String,
    read_timeout: Duration,
) -> crate::source::pull::PendingFetch<HlsFetchId> {
    crate::source::pull::PendingFetch {
        delay: Duration::ZERO,
        fut: Box::pin(async move {
            let result =
                tokio::time::timeout(read_timeout, fetch_bytes(&http, &url, creds.as_ref()))
                    .await
                    .unwrap_or_else(|_| {
                        Err(MultimuxError::Connect {
                            reason: format!("hls-pull: playlist read exceeded {read_timeout:?}"),
                        })
                    });
            (HlsFetchId::Playlist, result)
        }),
    }
}

/// A remote (LL-)HLS Media Playlist to pull: its URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct HlsPullRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo
    /// — see `crate::source::http_auth::resolve_credentials`.
    auth: Option<Credentials>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for HlsPullRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HlsPullRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl HlsPullRoute {
    /// Build a route descriptor. `url` is the target Media Playlist URL (not
    /// a Multivariant Playlist — this pulls one rendition directly).
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        HlsPullRoute {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }
}

impl Source for HlsPullRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// This session's own request/response identity — see the module doc's
/// "Correlating a fetch response".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HlsFetchId {
    /// A Media Playlist fetch — routes to `HlsClient::on_playlist`.
    Playlist,
    /// An init/part/segment fetch — routes to `HlsClient::on_resource`.
    Resource(ResourceId),
}

/// The sans-IO HLS-pull [`IngestSession`]: a driven [`HlsClient`], no
/// socket. [`run_hls_pull`] owns the real GETs and feeds responses in.
pub struct HlsIngestSession {
    client: HlsClient,
    pending: VecDeque<SessionEvent>,
    /// Set once the first `Output::Init` is recovered — guards against ever
    /// queuing a second `Established`/`NewProgram` pair (see the module doc's
    /// "Known limitation").
    program_announced: bool,
    /// `Output::EndOfStream` reached: the origin's `#EXT-X-ENDLIST` was seen
    /// and every fetch it named is accounted for. Read by [`run_hls_pull`]
    /// via [`media_plane::ingress::IngestDriver::session`] to decide when to call
    /// [`media_plane::ingress::IngestDriver::finish`] — see that method's docs for why this can't
    /// be a [`SessionEvent`] instead.
    ended: bool,
}

impl HlsIngestSession {
    /// Construct a fresh session for `playlist_url` — performs no I/O
    /// (`HlsClient::new` only queues the first `Action::FetchPlaylist`).
    pub fn new(playlist_url: impl Into<String>) -> Self {
        HlsIngestSession {
            client: HlsClient::new(playlist_url),
            pending: VecDeque::new(),
            program_announced: false,
            ended: false,
        }
    }

    /// See [`Self::ended`]'s field doc.
    pub fn ended(&self) -> bool {
        self.ended
    }

    fn drain_outputs(&mut self) -> Result<()> {
        while let Some(out) = self.client.next_output() {
            match out {
                HlsOutput::Init(bytes) => {
                    if self.program_announced {
                        continue; // mid-stream re-Init: see "Known limitation".
                    }
                    let media = Fmp4Demux::new().unpackage(bytes.as_slice())?;
                    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|t| t.spec).collect();
                    self.program_announced = true;
                    self.pending.push_back(SessionEvent::Established);
                    self.pending.push_back(SessionEvent::NewProgram {
                        program: ProgramId(0),
                        tracks: specs,
                    });
                }
                HlsOutput::Samples { track_id, samples } => {
                    for sample in samples {
                        self.pending.push_back(SessionEvent::Sample {
                            program: ProgramId(0),
                            track_id,
                            retention: RetentionClass::Timed,
                            sample,
                        });
                    }
                }
                HlsOutput::Discontinuity => {
                    // No `SessionEvent` routes on this yet — matches
                    // `ts_program::ProgramTracker`'s "metadata-only, nothing
                    // routes on it yet" precedent.
                }
                HlsOutput::EndOfStream => self.ended = true,
                _ => {}
            }
        }
        Ok(())
    }
}

impl Stage for HlsIngestSession {
    type In<'a> = (HlsFetchId, &'a [u8]);
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, (id, bytes): (HlsFetchId, &[u8]), _now: Timestamp) -> Result<()> {
        match id {
            HlsFetchId::Playlist => self.client.on_playlist(bytes)?,
            HlsFetchId::Resource(rid) => self.client.on_resource(rid, bytes)?,
        }
        self.drain_outputs()
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(crate::source::MAX_TS_READ)
    }
}

impl IngestSession for HlsIngestSession {
    type Request = Action;

    fn poll_transmit(&mut self) -> Option<Action> {
        self.client.poll()
    }
}

/// Constructs an [`HlsIngestSession`] — performs **no I/O** (see the module
/// doc's "Establishment is genuinely ordinary driving here").
pub struct HlsPullDialer {
    playlist_url: String,
}

impl Dialer for HlsPullDialer {
    type Session = HlsIngestSession;
    type Error = Infallible;

    fn dial(&mut self) -> core::result::Result<HlsIngestSession, Infallible> {
        Ok(HlsIngestSession::new(self.playlist_url.clone()))
    }
}

/// Performs `GET url` (answering a Digest challenge if `creds` names one),
/// returning an error on any non-2xx status.
async fn fetch_bytes(
    client: &HttpClient,
    url: &str,
    creds: Option<&Credentials>,
) -> Result<Vec<u8>> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if !status.is_success() {
        // 401/403 are permanent (a bad credential or a forbidden resource);
        // every other status is transient and may be retried.
        return Err(
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                MultimuxError::Auth {
                    reason: format!("hls-pull: {status}"),
                }
            } else {
                MultimuxError::Connect {
                    reason: format!("hls-pull: HTTP {status}"),
                }
            },
        );
    }
    crate::source::read_body_capped(response, crate::source::MAX_HTTP_BODY_BYTES, "hls-pull").await
}

/// Opens `route`'s connect-time HTTP client and userinfo-stripped URL —
/// mirrors `ts_http::open_stream`'s split between "parse/strip the URL" and
/// "drive the fetches" so a bad URL fails fast, before [`run_hls_pull`]'s
/// drive loop ever starts.
fn build_client(route: &HlsPullRoute) -> Result<(HttpClient, Url, Option<Credentials>)> {
    let parsed = Url::parse(&route.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad HLS-pull URL {}: {e}",
            crate::redact::redact_url(&route.url)
        ),
    })?;
    let credentials = resolve_credentials(route.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let http = HttpClient::builder()
        .redirect(crate::source::redirect_policy())
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    Ok((http, clean_url, credentials))
}

/// Turns a driver that has reached a terminal [`HealthState`] into this
/// crate's own `Result`, moving the concrete session error out via
/// [`media_plane::ingress::IngestDriver::into_health`].
///
/// **This is why a pull drive loop must check `health()` after every feed at
/// all**: `IngestDriver::feed` records a session error in `health` and
/// returns `()`, so a loop that only ever calls `feed` never observes it. A
/// `smooth_pull` session rejecting a PlayReady-protected manifest, or an
/// `hls_pull` session rejecting a malformed playlist, would otherwise leave
/// the loop spinning against a session that can never make progress.
fn terminal_result<S>(driver: media_plane::ingress::IngestDriver<S>, what: &str) -> Result<()>
where
    S: media_plane::ingress::IngestSession<Error = MultimuxError>,
{
    match driver.into_health() {
        media_plane::ingress::HealthState::Failed(e) => Err(e),
        media_plane::ingress::HealthState::HandshakeTimedOut { deadline } => {
            Err(MultimuxError::Connect {
                reason: format!("{what}: handshake deadline {deadline:?} passed"),
            })
        }
        // `Ended` is a clean finish; the two running states are unreachable
        // here (callers only call this once `health().is_running()` is false)
        // but map to `Ok` rather than panicking on a future variant.
        _ => Ok(()),
    }
}

/// Drives `route` to completion: dial (no I/O), then pump
/// [`media_plane::ingress::IngestDriver::poll_transmit`] → fetch → [`media_plane::ingress::IngestDriver::feed`] until the
/// origin's playlist reports end-of-stream or a fetch fails outright — the
/// new drive loop, replacing the pre-5a `HlsPullSource::connect`/
/// `HlsPullSession::next_samples` pair (and their `TokioClient` wrapper).
///
/// Bounded fan-out: never more than
/// [`crate::source::MAX_INFLIGHT_FETCHES`] concurrent
/// requests (see the module doc); each individual fetch is itself bounded by
/// [`IngestTimeouts::read`], and the handshake (until the first init segment
/// resolves) by `handshake`.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_hls_pull(
    route: &HlsPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> Result<()> {
    let (http, clean_url, credentials) = build_client(route)?;
    let mut dialer = HlsPullDialer {
        playlist_url: clean_url.to_string(),
    };
    let mut driver = run_dial(
        &mut dialer,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    )
    .unwrap_or_else(|never: Infallible| match never {});

    let read_timeout = route.timeouts.read;
    // The fetch/retry/wait engine (SP6.2) owns the backlog, the in-flight
    // bound, the retries and the waits; this loop owns only the `Action`
    // translation and the `feed`.
    let mut scheduler: crate::source::pull::PullScheduler<HlsFetchId> =
        crate::source::pull::PullScheduler::new(crate::source::MAX_INFLIGHT_FETCHES);
    // Tracks resource fetches so a transient failure (a 404 from an eviction
    // race, a dropped connection) can be retried instead of ending the whole
    // session (audit run 7, W14): `HlsFetchId` + resolved URL + attempts so
    // far. A playlist fetch is not tracked — a broken manifest is a
    // route-level failure, not a transient one.
    let mut resource_retries: HashMap<HlsFetchId, (String, u32)> = HashMap::new();
    // The session's own reload-pacing floor (`Action::WaitMs`): the *next*
    // playlist fetch is dispatched no earlier than this. `None` until the
    // engine has paced once. Set to `last playlist dispatch + hint` when a
    // `WaitMs` is seen, so non-LL live HLS reloads at the engine's cadence
    // (RFC 8216 §4.3.3.1), not back-to-back at RTT rate — while a ready
    // resource fetch is still serviced the moment it completes (defect 5).
    let mut playlist_floor: Option<tokio::time::Instant> = None;
    // When the previous playlist fetch was *dispatched* — the origin of the
    // floor's `+ hint` (see `playlist_floor`).
    let mut last_playlist_dispatch: Option<tokio::time::Instant> = None;
    // The most recent `WaitMs` hint, applied to the next playlist dispatch.
    let mut reload_hint: Option<Duration> = None;
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();

    loop {
        while let Some(action) = driver.poll_transmit() {
            match action {
                Action::WaitMs(ms) => {
                    // The engine's own reload-pacing request (drained via
                    // `poll_transmit`, exactly like a fetch). It is a floor on
                    // the *next* playlist dispatch, never a delay on an
                    // already-running fetch's result.
                    reload_hint = Some(Duration::from_millis(ms));
                    playlist_floor = last_playlist_dispatch.map(|t| t + Duration::from_millis(ms));
                }
                Action::FetchPlaylist { .. } => {
                    let url = action
                        .playlist_request_url()
                        .expect("FetchPlaylist always has a request URL");
                    // Dispatched at the pacing floor when one is pending, so a
                    // fast playlist response is not re-fetched immediately.
                    scheduler.push_at(
                        playlist_fetch(http.clone(), credentials.clone(), url, read_timeout),
                        playlist_floor,
                    );
                    last_playlist_dispatch = Some(tokio::time::Instant::now());
                    // The floor is one-shot: the next hint re-arms it.
                    playlist_floor = None;
                }
                Action::FetchResource { id, url, .. } => {
                    // Remember the URL so a transient failure can be retried
                    // (audit run 7, W14). Attempt 1 on the first spawn.
                    let fetch_id = HlsFetchId::Resource(id);
                    resource_retries
                        .entry(fetch_id)
                        .or_insert_with(|| (url.clone(), 1));
                    scheduler.push(resource_fetch(
                        http.clone(),
                        credentials.clone(),
                        fetch_id,
                        url,
                        read_timeout,
                        Duration::ZERO,
                    ));
                }
                // `Action` is `#[non_exhaustive]`: a future variant is simply
                // dropped from the backlog rather than failing the whole
                // route, matching this driver's general "unrecognised ==
                // no-op, not fatal" posture.
                _ => {}
            }
        }

        scheduler.pump();

        if scheduler.is_idle() {
            if driver.session().ended() {
                driver.finish();
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
                return terminal_result(driver, "hls-pull");
            }
            // Nothing running, queued or awaiting a retry: the client has
            // genuinely nothing to do right now. Park briefly rather than
            // spinning — see `IDLE_POLL_INTERVAL`.
            reload_hint = None;
            scheduler.next(None, IDLE_POLL_INTERVAL).await;
            continue;
        }

        let joined = scheduler.next(reload_hint, IDLE_POLL_INTERVAL).await;
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        match joined {
            Some(crate::source::pull::FetchOutcome::Ready(fetch_id, bytes)) => {
                resource_retries.remove(&fetch_id);
                driver.feed((fetch_id, bytes.as_slice()), now);
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
            }
            Some(crate::source::pull::FetchOutcome::Failed(fetch_id, e)) => {
                // A failed **resource** fetch is retried (bounded) rather than
                // ending the session (audit run 7, W14): a single part that
                // 404s on an eviction race, or one dropped connection, must
                // not reconnect the whole route (which disrupts every
                // viewer). A failed **playlist** fetch is still terminal — a
                // broken manifest is not a transient error.
                match fetch_id {
                    HlsFetchId::Resource(id) => {
                        // A permanent auth/permission failure is not retried —
                        // retrying 8 times over ~2 min would only delay the
                        // reconnect for a credential that will never work.
                        if matches!(e, MultimuxError::Auth { .. }) {
                            tracing::error!(
                                resource = ?id,
                                error = %e,
                                "hls-pull: resource fetch failed permanently (auth); failing the session"
                            );
                            resource_retries.remove(&fetch_id);
                            return Err(e);
                        }
                        let attempts = resource_retries
                            .get(&fetch_id)
                            .map(|(_, a)| *a)
                            .unwrap_or(MAX_RESOURCE_RETRY_ATTEMPTS);
                        if attempts < MAX_RESOURCE_RETRY_ATTEMPTS {
                            let url = resource_retries
                                .get(&fetch_id)
                                .map(|(u, _)| u.clone())
                                .expect("resource_retries entry exists for a spawned resource");
                            let next_attempt = attempts.saturating_add(1);
                            tracing::warn!(
                                resource = ?id,
                                attempt = next_attempt,
                                error = %e,
                                "hls-pull: resource fetch failed; queuing a retry"
                            );
                            if let Some(entry) = resource_retries.get_mut(&fetch_id) {
                                entry.1 = next_attempt;
                            }
                            // Queued, not spawned: the spawn loop applies
                            // the in-flight bound and the backoff delay.
                            scheduler.push_retry(resource_fetch(
                                http.clone(),
                                credentials.clone(),
                                fetch_id,
                                url,
                                read_timeout,
                                retry_backoff(next_attempt),
                            ));
                        } else {
                            tracing::error!(
                                resource = ?id,
                                attempts,
                                error = %e,
                                "hls-pull: resource fetch failed after the retry bound; \
                                 giving up on this session"
                            );
                            return Err(e);
                        }
                    }
                    HlsFetchId::Playlist => return Err(e),
                }
            }
            Some(crate::source::pull::FetchOutcome::TaskPanic(detail)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("hls-pull: fetch task failed: {detail}"),
                });
            }
            // Nothing was in flight (the scheduler parked on the idle poll);
            // check the session's own end condition below.
            None => {}
        }

        if !driver.health().is_running() {
            // The feed above drove the session terminal (a rejected
            // playlist/manifest/resource) — see `terminal_result`. Health is
            // already terminal here, so this call's internal terminal-health
            // check flushes every program's trailing partial segment.
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            return terminal_result(driver, "hls-pull");
        }

        if driver.session().ended() {
            driver.finish();
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            return terminal_result(driver, "hls-pull");
        }
    }
}

/// Lexical tripwire, honestly labelled: it asserts only that `run_hls_pull`'s
/// production half contains no `tokio::time::` + `sleep(` — i.e. that no wait
/// was re-inlined at this call site. It does NOT guard the reload-pacing
/// *behaviour* (a `scheduler.next(Some(hint))` that dropped the floor would
/// still pass it); that is what the paused-time pacing tests in
/// `source::pull::tests` (`a_paced_fetch_is_dispatched_at_its_floor_..`,
/// `a_resource_fetch_completes_while_a_pacing_floor_..`) are for. On `main`
/// the production half had THREE such sleeps (`:144`, `:518`, `:576`), so this
/// FAILED pre-migration.
#[test]
fn hls_pull_has_no_inline_sleep_outside_the_scheduler() {
    let src = include_str!("hls_pull.rs");
    let production = src
        .split("#[cfg(test)]")
        .next()
        .expect("split yields one part");
    let hits: Vec<usize> = production
        .match_indices(concat!("tokio::time::", "sleep("))
        .map(|(at, _)| production[..at].matches('\n').count() + 1)
        .collect();
    assert!(
        hits.is_empty(),
        "inline sleep(s) in hls_pull.rs at lines {hits:?}: waits belong in PullScheduler"
    );
}

#[cfg(test)]
mod tests {
    /// Before/after pin (#1141): the shared `Backoff` reproduces the
    /// shift-based schedule `retry_backoff` carried before consolidation.
    /// SP1.5: `retry_backoff` is the pre-consolidation shift series
    /// (0.5 s doubling to a 30 s cap) with `backon`'s jitter on top, so the
    /// band is pinned exactly and the cap saturates.
    #[test]
    fn retry_backoff_matches_the_pre_consolidation_shift_schedule() {
        for attempt in 0..40u32 {
            let shift = attempt.saturating_sub(1).min(16);
            let raw = RESOURCE_RETRY_BASE_DELAY
                .checked_mul(1u32 << shift)
                .unwrap_or(RESOURCE_RETRY_MAX_DELAY)
                .min(RESOURCE_RETRY_MAX_DELAY);
            let got = retry_backoff(attempt);
            // Jittered into [raw, min(2 * raw, cap)].
            assert!(
                got >= raw && got <= (raw * 2).min(RESOURCE_RETRY_MAX_DELAY),
                "attempt {attempt}: {got:?} outside [{raw:?}, {cap:?}]",
                cap = (raw * 2).min(RESOURCE_RETRY_MAX_DELAY)
            );
        }
        // Once raw saturates at the cap the jitter is clamped back to it.
        assert_eq!(retry_backoff(20), RESOURCE_RETRY_MAX_DELAY);
    }

    use super::*;
    use crate::testutil::MockAuthScheme;
    use broadcast_hls::{MediaPlaylist, MediaSegment};
    use media_plane::ingress::HealthState;
    use media_plane::trunk::{SampleCursor, SampleCursorItem, TrunkConfig};
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use transmux::ll_hls::LlHlsSegmenter;
    use transmux::pipeline::Sample;
    use transmux::{
        AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig,
    };

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    /// W16: `fetch_bytes` caps a response body even when the origin lies
    /// about `Content-Length` — the real loopback server declares
    /// `Content-Length: 10` but streams an endless body, and the fetch must
    /// fail on the size cap, not buffer forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fetch_bytes_caps_a_body_that_lies_about_its_length() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // A raw TCP server that declares a `Content-Length` *larger than the
        // cap* (a lying/oversized header claim) and then streams that many
        // bytes — `read_body_capped` must refuse from the header alone, before
        // buffering the body.
        let over = crate::source::MAX_HTTP_BODY_BYTES + 1024;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let header = format!(
                    "HTTP/1.1 200 OK
Content-Length: {over}

"
                );
                let _ = sock.write_all(header.as_bytes()).await;
                // Keep the connection open without sending the declared body:
                // the cap must reject from the header alone, so no body is
                // needed (and we avoid a real multi-MB write).
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        let client = reqwest::Client::new();
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            fetch_bytes(&client, &format!("http://{addr}/x"), None),
        )
        .await
        .expect("fetch must return, not hang");
        server.abort();
        let err = result.expect_err("an over-cap body must be refused");
        assert!(
            err.to_string().contains("exceeds") || err.to_string().contains("cap"),
            "the error must name the cap: {err}"
        );
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
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("must return");
        server.abort();
        assert!(result.is_err(), "a 403 resource must fail the session");
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
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    fn drain(cursor: &mut SampleCursor) -> usize {
        let mut n = 0;
        while let Some(item) = cursor.poll() {
            if matches!(item, SampleCursorItem::Timed { .. }) {
                n += 1;
            }
        }
        n
    }

    const TRACK_ID: u32 = 1;
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
    /// `ts_program::test_support::build_ts_bytes` uses — and renders the
    /// playlist via `broadcast_hls::MediaPlaylist::to_m3u8` (the same real
    /// renderer the workspace's own LL-HLS origin uses), rather than
    /// depending on `multimux`'s own (unrelated, currently-broken — see this
    /// crate's CHANGELOG) `store`/`origin`/`output::llhls` modules the pre-5a
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
                // Test fixture: `segment.duration` is the segmenter's own
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
                return bytes.clone().into_response();
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }

        let state = FixtureState {
            playlist: playlist_text,
            init,
            segments: std::sync::Arc::new(segments),
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

    /// Drives the raw [`HlsIngestSession`] over real HTTP (dial →
    /// `poll_transmit` → fetch → `feed`), returning the `TrackSpec`s it
    /// announced and a per-`track_id` count of the `SessionEvent::Sample`s it
    /// produced.
    ///
    /// **Why the session and not an `IngestDriver`+`SampleCursor` for the
    /// exact count**: `Trunk::subscribe()` starts from *now* and sees no
    /// backlog, and `HlsClient` legitimately flushes a whole batch of
    /// buffered part/segment resources the instant the init segment arrives —
    /// i.e. `NewProgram` (which is what mints the `Trunk`) and that batch's
    /// `Sample`s are drained by the *same* `IngestDriver::feed` call, so no
    /// cursor can exist in time to observe them. Counting `SessionEvent`s
    /// observes the identical property (real CMAF over real HTTP → real
    /// decoded samples, correctly attributed) without racing the subscription.
    /// `Trunk` arrival is asserted separately, by
    /// [`assert_samples_reach_the_trunk`].
    async fn drive_session_and_count(
        route: &HlsPullRoute,
    ) -> Result<(Vec<TrackSpec>, HashMap<u32, usize>)> {
        let (http, clean_url, credentials) = build_client(route)?;
        let mut session = HlsIngestSession::new(clean_url.to_string());
        let mut backlog: VecDeque<Action> = VecDeque::new();
        let mut specs: Vec<TrackSpec> = Vec::new();
        let mut per_track: HashMap<u32, usize> = HashMap::new();
        // HANG GUARD (issue #826): ceiling on the whole session-drive loop.
        // The session fetches playlist + init + segments over real loopback
        // HTTP against a local axum server; each fetch completes in ~ms.
        // Raised to 60s for load tolerance — only job is to fail "never
        // finishes" rather than hang, not a timing claim.
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
            match action {
                Action::WaitMs(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                Action::FetchPlaylist { .. } => {
                    let url = action.playlist_request_url().expect("playlist URL");
                    let bytes = fetch_bytes(&http, &url, credentials.as_ref()).await?;
                    session.feed((HlsFetchId::Playlist, bytes.as_slice()), now)?;
                }
                Action::FetchResource { id, url, .. } => {
                    let bytes = fetch_bytes(&http, &url, credentials.as_ref()).await?;
                    session.feed((HlsFetchId::Resource(id), bytes.as_slice()), now)?;
                }
                _ => {}
            }
        }
        Ok((specs, per_track))
    }

    /// The `Trunk`-side counterpart to [`drive_session_and_count`]: drives the
    /// same route through a real [`media_plane::ingress::IngestDriver`] and
    /// asserts real samples actually land on a real [`SampleCursor`] — the
    /// half a session-level count cannot prove. Deliberately a `> 0`
    /// assertion, not an exact one: the cursor can only see what is published
    /// *after* it subscribes, and the first batch is published in the same
    /// `feed` that mints the `Trunk` (see [`drive_session_and_count`]'s doc).
    async fn assert_samples_reach_the_trunk(route: &HlsPullRoute) {
        let (http, clean_url, credentials) = build_client(route).expect("build client");
        let mut dialer = HlsPullDialer {
            playlist_url: clean_url.to_string(),
        };
        let mut driver = run_dial(
            &mut dialer,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        )
        .expect("dial is infallible");
        assert!(
            matches!(driver.health(), HealthState::Establishing),
            "dial() must not establish the session: {:?}",
            driver.health()
        );

        let mut backlog: VecDeque<Action> = VecDeque::new();
        let mut cursor: Option<SampleCursor> = None;
        let start = std::time::Instant::now();
        let mut total = 0usize;
        // HANG GUARD (issue #826): ceiling on the whole Trunk-drive loop.
        // Same reasoning as `drive_session_and_count`'s deadline — real
        // loopback HTTP, each fetch ~ms, raised for load tolerance, not a
        // timing claim.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            while let Some(a) = driver.poll_transmit() {
                backlog.push_back(a);
            }
            // Deliver the init resource FIRST, ahead of any part/segment.
            //
            // `HlsClient::on_playlist` queues its `FetchResource` actions
            // segments-first, map-last, and buffers every part/segment that
            // arrives before the init — replaying the whole batch the instant
            // the init lands. Delivered in queue order against a fully-known
            // static playlist, that means *every* sample is emitted by the one
            // `feed` that also emits `NewProgram`, so the `Trunk` this test
            // wants to observe is created and filled in the same call and no
            // cursor can exist in time to see any of it. Reordering is not a
            // test cheat: `on_resource`'s own docs state fetches may complete
            // in any order (a real concurrent IO loop routinely finishes the
            // small init before a large segment), and it is the only ordering
            // under which a subscriber can observe this fixture at all.
            let idx = if cursor.is_none() {
                backlog
                    .iter()
                    .position(|a| {
                        matches!(
                            a,
                            Action::FetchResource {
                                id: ResourceId::Init,
                                ..
                            }
                        )
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            let Some(action) = backlog.remove(idx) else {
                if driver.session().ended() || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            };
            let now = Timestamp::from_instant(start, std::time::Instant::now());
            match action {
                Action::WaitMs(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                Action::FetchPlaylist { .. } => {
                    let url = action.playlist_request_url().expect("playlist URL");
                    let bytes = fetch_bytes(&http, &url, credentials.as_ref())
                        .await
                        .expect("fetch");
                    driver.feed((HlsFetchId::Playlist, bytes.as_slice()), now);
                }
                Action::FetchResource { id, url, .. } => {
                    let bytes = fetch_bytes(&http, &url, credentials.as_ref())
                        .await
                        .expect("fetch");
                    driver.feed((HlsFetchId::Resource(id), bytes.as_slice()), now);
                }
                _ => {}
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                total += drain(c);
            }
        }

        assert!(
            matches!(driver.health(), HealthState::Live),
            "the session must have established: {:?}",
            driver.health()
        );
        assert!(
            total > 0,
            "real samples must reach the Trunk through IngestDriver, got {total}"
        );
    }

    /// Biting loopback test: a real axum server serves a real
    /// `LlHlsSegmenter`-built CMAF fixture over real HTTP; asserts the
    /// session recovers the right `TrackSpec` and produces **exactly** the
    /// fixture's own sample count, and (separately) that those samples really
    /// do land in a `Trunk` through a real `IngestDriver`.
    ///
    /// MUTATION-CHECKED: replacing the `session.feed(...)` call in
    /// `drive_session_and_count` with a no-op makes `per_track` empty and
    /// fails the exact-count assertion; replacing `driver.feed(...)` in
    /// `assert_samples_reach_the_trunk` with a no-op fails its `total > 0`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn loopback_hls_pull_lands_real_samples_in_trunk() {
        let (url, server) = start_cmaf_fixture_server(None).await;
        let route = HlsPullRoute::new("pulled-cam", url);

        let (specs, per_track) =
            // HANG GUARD (issue #826): backstop around the session drive
            // against a real axum server. The session's own internal pacing
            // keeps it alive for the content's duration; raised to 60s for
            // load tolerance since it only exists to fail "never finishes".
            tokio::time::timeout(Duration::from_secs(60), drive_session_and_count(&route))
                .await
                .expect("drive timed out")
                .expect("drive");

        assert_eq!(specs.len(), 1, "one video track recovered: {specs:?}");
        assert_eq!(specs[0].track_id, TRACK_ID);
        assert_eq!(specs[0].timescale, VIDEO_TIMESCALE);
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

        assert_samples_reach_the_trunk(&route).await;
        server.abort();
    }

    /// The full `run_hls_pull` drive loop (bounded fan-out, real timeouts)
    /// against the same fixture, asserting it returns cleanly once the
    /// origin's `#EXT-X-ENDLIST` is fully accounted for.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_hls_pull_completes_cleanly_on_a_static_playlist() {
        let (url, server) = start_cmaf_fixture_server(None).await;
        let route = HlsPullRoute::new("pulled-cam", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // HANG GUARD (issue #826): backstop around the full `run_hls_pull`
        // against a static CMAF fixture. `run_hls_pull` returns on its own
        // once the playlist is exhausted; this only exists to fail "never
        // returns" rather than hang CI, not a timing claim.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("run_hls_pull must not hang against a static playlist");
        assert!(
            result.is_ok(),
            "a static playlist must end cleanly: {result:?}"
        );
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
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let handle = std::sync::Arc::clone(&route_handle);
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_hls_pull(&route, trunk_config(), handshake(), &route_handle),
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
        // Real samples reach the trunk on a fresh drive over the same route
        // (subscribing after the first run would miss the backlog — see the
        // module doc).
        assert_samples_reach_the_trunk(&route).await;
        server.abort();
    }

    /// Issue #760: classic MPEG-TS-segment HLS (HLS v3 — no `EXT-X-MAP`,
    /// self-contained `.ts` segments, the dominant legacy/IPTV form) served
    /// from the real, committed `hls-runtime/tests/fixtures/ts-hls/`
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

        let fixture_dir = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../hls-runtime/tests/fixtures/ts-hls"
        ));
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

        let (specs, per_track) =
            // HANG GUARD (issue #826): backstop around the session drive
            // against the TS-HLS fixture. Same reasoning as the CMAF
            // loopback test — real loopback HTTP, raised for load tolerance.
            tokio::time::timeout(Duration::from_secs(60), drive_session_and_count(&route))
                .await
                .expect("drive timed out")
                .expect("drive");

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

        assert_samples_reach_the_trunk(&route).await;
        server.abort();
    }

    // --- issue #663 "Finish client-side multi-scheme auth" ---

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "hls-pull-bearer-token";

    async fn drain_via_run_hls_pull(route: HlsPullRoute) -> Result<()> {
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        run_hls_pull(&route, trunk_config(), handshake(), &route_handle).await
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
        let route = HlsPullRoute::new("pulled-wrong", wrong_creds).with_timeouts(IngestTimeouts {
            connect: IngestTimeouts::default().connect,
            read: Duration::from_secs(2),
        });
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
    /// must fail within `IngestTimeouts::read`, not hang forever.
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
            .with_timeouts(IngestTimeouts {
                connect: IngestTimeouts::default().connect,
                read: Duration::from_secs(2),
            });
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // DISCRIMINATOR (issue #826): must prove the operation returns
        // through the CONFIGURED read timeout, not via any longer system/
        // library default. Gap widened: configured read timeout raised
        // from 150ms to 2s, assertion window raised from 5s to 10s — 10s
        // is still well below any plausible fallback. MUTATION CHECKED:
        // inflating the configured read timeout to 30s makes the 10s
        // outer timeout fire first, producing `Elapsed`.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_hls_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("run_hls_pull must not exceed the assertion window");
        assert!(
            result.is_err(),
            "a stalled playlist fetch must fail, not hang forever"
        );
    }

    // Bounds concurrent fetches: the cap itself is unit-tested at its own
    // definition (`crate::source::inflight_tests`), which is where the
    // decision now lives -- these loops only call
    // the `PullScheduler`'s in-flight cap. What is *not* black-box observable
    // here is that a loop actually consults it: the committed fixtures never
    // reveal more than a handful of resources at once, so removing the gate
    // would still pass every other test in this module. Observing the cap
    // end-to-end would need the fixture server to count concurrent
    // connections. Recorded rather than left implicit.
}
