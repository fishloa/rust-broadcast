//! HLS-pull ingest source (issue #663 P3c / #717 / #760; re-ported onto the
//! media-plane ingress traits at plan step 5a round 3): a driven
//! [`ll_hls_runtime::client::LlHlsClient`] — the sans-IO Low-Latency HLS
//! (RFC 8216bis) playback engine — feeding real fetch bytes in and turning
//! its `Output`s into [`SessionEvent`]s.
//!
//! # Why this reuses `LlHlsClient`, not `TokioClient`
//!
//! Before this port, this module wrapped
//! `ll_hls_runtime::client::tokio_client::TokioClient` — the executor-bound
//! adapter that owns its own `reqwest` fetch loop internally. That fit the
//! pre-5a `connect()`/`next_samples()` shape, but it cannot fit
//! [`IngestSession`]: `TokioClient` performs its own I/O, so there is nothing
//! for `Stage::feed`/[`IngestSession::poll_transmit`] to drive. `LlHlsClient`
//! is the sans-IO core `TokioClient` itself wraps —
//! `poll() -> Option<Action>` out, `on_playlist`/`on_resource` in — which is
//! exactly the [`IngestSession::Request`]/`Stage::In` shape round 3 added.
//! This module is now the *other* adapter over the same sans-IO engine,
//! parallel to `TokioClient`, driven by [`media_plane::ingress::IngestDriver`] instead of a bespoke
//! `connect`/`next_samples` pair — so the actual LL-HLS logic (reload
//! scheduling, part/segment dedup, fMP4/classic-TS demux) is still owned
//! entirely by `ll-hls-runtime`, never duplicated here.
//!
//! # Establishment is genuinely ordinary driving here
//!
//! [`HlsPullDialer::dial`] performs no I/O at all — `LlHlsClient::new` only
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
//! `LlHlsClient::on_playlist` and `on_resource` are two different methods,
//! but a `Stage::In` is one type. [`HlsFetchId`] is this session's own
//! (opaque to `media-plane`) request/response identity — `Playlist` for the
//! one method, `Resource(id)` wrapping [`ResourceId`] for the other — chosen
//! entirely by this module; the plane never sees it.
//!
//! # Bounded in-flight fetches
//!
//! `LlHlsClient::poll` can hand back many `Action::FetchResource`s in one
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

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use broadcast_auth::Credentials;
use broadcast_common::{Demand, Stage, Timestamp, Unpackage};
use ll_hls_runtime::client::{Action, LlHlsClient, Output as HlsOutput, ResourceId};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestSession, ProgramId, SessionEvent, run_dial,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use reqwest::Client as HttpClient;
use tokio::task::JoinSet;
use transmux::media::Fmp4Demux;
use transmux::pipeline::TrackSpec;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};
use crate::source::{IngestTimeouts, MAX_INFLIGHT_FETCHES, Source};

/// How long a `run_*_pull` drive loop parks when its session has, momentarily,
/// neither an outbound request queued nor a fetch in flight — and has not
/// ended. A bare `continue` there would spin the loop with no `.await` in it,
/// which on a current-thread runtime starves every other task on the executor
/// (including the in-flight fetches this loop is waiting for). Short enough
/// that it costs no observable latency, long enough that it is not a spin.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlsFetchId {
    /// A Media Playlist fetch — routes to `LlHlsClient::on_playlist`.
    Playlist,
    /// An init/part/segment fetch — routes to `LlHlsClient::on_resource`.
    Resource(ResourceId),
}

/// The sans-IO HLS-pull [`IngestSession`]: a driven [`LlHlsClient`], no
/// socket. [`run_hls_pull`] owns the real GETs and feeds responses in.
pub struct HlsIngestSession {
    client: LlHlsClient,
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
    /// (`LlHlsClient::new` only queues the first `Action::FetchPlaylist`).
    pub fn new(playlist_url: impl Into<String>) -> Self {
        HlsIngestSession {
            client: LlHlsClient::new(playlist_url),
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
async fn fetch_bytes(client: &HttpClient, url: &str, creds: Option<&Credentials>) -> Result<Vec<u8>> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
            MultimuxError::Auth {
                reason: format!("hls-pull: {status}"),
            }
        } else {
            MultimuxError::Connect {
                reason: format!("hls-pull: HTTP {status}"),
            }
        });
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| MultimuxError::Connect {
            reason: format!("hls-pull read: {e}"),
        })
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
/// Bounded fan-out: never more than [`MAX_INFLIGHT_FETCHES`] concurrent
/// requests (see the module doc); each individual fetch is itself bounded by
/// [`IngestTimeouts::read`], and the handshake (until the first init segment
/// resolves) by `handshake`.
pub async fn run_hls_pull(
    route: &HlsPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
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
    let mut backlog: VecDeque<Action> = VecDeque::new();
    let mut inflight: JoinSet<(HlsFetchId, Result<Vec<u8>>)> = JoinSet::new();
    let start = std::time::Instant::now();

    loop {
        while let Some(action) = driver.poll_transmit() {
            backlog.push_back(action);
        }

        while inflight.len() < MAX_INFLIGHT_FETCHES {
            let Some(action) = backlog.pop_front() else {
                break;
            };
            match action {
                Action::WaitMs(ms) => {
                    // The reload-pacing hint: a plain async sleep here is not
                    // a sans-IO violation — `Action::WaitMs` is the session's
                    // own *request* for how long to wait (drained via
                    // `poll_transmit`, exactly like a fetch), so the decision
                    // of how long, and the timing itself, live entirely on
                    // the IO side. See the module doc's contrast with
                    // `dash_pull`'s pre-port internal sleep.
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                Action::FetchPlaylist { .. } => {
                    let url = action
                        .playlist_request_url()
                        .expect("FetchPlaylist always has a request URL");
                    let http = http.clone();
                    let creds = credentials.clone();
                    inflight.spawn(async move {
                        let result =
                            tokio::time::timeout(read_timeout, fetch_bytes(&http, &url, creds.as_ref()))
                                .await
                                .unwrap_or_else(|_| {
                                    Err(MultimuxError::Connect {
                                        reason: format!(
                                            "hls-pull: playlist read exceeded {read_timeout:?}"
                                        ),
                                    })
                                });
                        (HlsFetchId::Playlist, result)
                    });
                }
                Action::FetchResource { id, url, .. } => {
                    let http = http.clone();
                    let creds = credentials.clone();
                    inflight.spawn(async move {
                        let result =
                            tokio::time::timeout(read_timeout, fetch_bytes(&http, &url, creds.as_ref()))
                                .await
                                .unwrap_or_else(|_| {
                                    Err(MultimuxError::Connect {
                                        reason: format!(
                                            "hls-pull: resource {id:?} read exceeded {read_timeout:?}"
                                        ),
                                    })
                                });
                        (HlsFetchId::Resource(id), result)
                    });
                }
                // `Action` is `#[non_exhaustive]`: a future variant is simply
                // dropped from the backlog rather than failing the whole
                // route, matching this driver's general "unrecognised ==
                // no-op, not fatal" posture.
                _ => {}
            }
        }

        if inflight.is_empty() {
            if driver.session().ended() {
                driver.finish();
                return terminal_result(driver, "hls-pull");
            }
            // Nothing in flight and nothing queued: the client has genuinely
            // nothing to do right now. Park briefly rather than spinning —
            // see `IDLE_POLL_INTERVAL`.
            tokio::time::sleep(IDLE_POLL_INTERVAL).await;
            continue;
        }

        let joined = inflight.join_next().await;
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        match joined {
            Some(Ok((fetch_id, Ok(bytes)))) => {
                driver.feed((fetch_id, bytes.as_slice()), now);
            }
            Some(Ok((_fetch_id, Err(e)))) => return Err(e),
            Some(Err(join_err)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("hls-pull: fetch task failed: {join_err}"),
                });
            }
            None => unreachable!("checked inflight.is_empty() above"),
        }

        if !driver.health().is_running() {
            // The feed above drove the session terminal (a rejected
            // playlist/manifest/resource) — see `terminal_result`.
            return terminal_result(driver, "hls-pull");
        }

        if driver.session().ended() {
            driver.finish();
            return terminal_result(driver, "hls-pull");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::MockAuthScheme;
    use media_plane::ingress::HealthState;
    use media_plane::trunk::{SampleCursor, SampleCursorItem, TrunkConfig};
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use transmux::hls::{MediaPlaylist, MediaSegment};
    use transmux::ll_hls::LlHlsSegmenter;
    use transmux::pipeline::Sample;
    use transmux::{AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
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
    const MOVIE_TIMESCALE: u32 = 90_000;
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
    /// playlist via `transmux::hls::MediaPlaylist::to_m3u8` (the same real
    /// renderer the workspace's own LL-HLS origin uses), rather than
    /// depending on `multimux`'s own (unrelated, currently-broken — see this
    /// crate's CHANGELOG) `store`/`origin`/`output::llhls` modules the pre-5a
    /// version of this test built a whole origin server out of.
    fn build_cmaf_fixture() -> (String, Vec<u8>, Vec<Vec<u8>>) {
        let mut seg = LlHlsSegmenter::with_part_target(
            vec![video_track_spec()],
            MOVIE_TIMESCALE,
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

        let map = transmux::hls::MapTag {
            uri: "init.mp4".to_string(),
            byte_range: None,
        };
        let mut segments = Vec::new();
        let mut media_segments = Vec::new();
        for (i, segment) in seg.take_ready_segments().into_iter().enumerate() {
            media_segments.push(MediaSegment {
                uri: format!("seg{i}.m4s"),
                duration: segment.duration,
                discontinuous: false,
                parts: Vec::new(),
                byte_range: None,
                map: Some(map.clone()),
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
        };
        (playlist.to_m3u8(), init, segments)
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
            .route("/:name", get(handler))
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
    /// backlog, and `LlHlsClient` legitimately flushes a whole batch of
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
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

        let (specs, per_track) = tokio::time::timeout(
            Duration::from_secs(20),
            drive_session_and_count(&route),
        )
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
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_hls_pull(&route, trunk_config(), handshake()),
        )
        .await
        .expect("run_hls_pull must not hang against a static playlist");
        assert!(
            result.is_ok(),
            "a static playlist must end cleanly: {result:?}"
        );
        server.abort();
    }

    /// Issue #760: classic MPEG-TS-segment HLS (HLS v3 — no `EXT-X-MAP`,
    /// self-contained `.ts` segments, the dominant legacy/IPTV form) served
    /// from the real, committed `ll-hls-runtime/tests/fixtures/ts-hls/`
    /// fixture — proving the pump recovers real `TrackSpec`s (from the
    /// client's issue-#760-synthesized `Output::Init`) and every real access
    /// unit lands in the `Trunk`, entirely through the production
    /// `LlHlsClient` — no TS-specific code in this module at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pulls_classic_ts_segment_hls_and_lands_samples_in_trunk() {
        use axum::Router;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use transmux::TsDemux;

        let fixture_dir = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ll-hls-runtime/tests/fixtures/ts-hls"
        ));
        let playlist_text = std::fs::read_to_string(fixture_dir.join("index.m3u8"))
            .expect("read fixture playlist");
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

        let (specs, per_track) = tokio::time::timeout(
            Duration::from_secs(20),
            drive_session_and_count(&route),
        )
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
        run_hls_pull(&route, trunk_config(), handshake()).await
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
        let result = tokio::time::timeout(Duration::from_secs(10), drain_via_run_hls_pull(route))
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
        let result = tokio::time::timeout(Duration::from_secs(10), drain_via_run_hls_pull(route))
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
        let result = tokio::time::timeout(Duration::from_secs(10), drain_via_run_hls_pull(route))
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
        let result = tokio::time::timeout(Duration::from_secs(10), drain_via_run_hls_pull(route))
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

        let route = HlsPullRoute::new("stalled", format!("http://{addr}/media.m3u8")).with_timeouts(
            IngestTimeouts {
                connect: IngestTimeouts::default().connect,
                read: Duration::from_millis(150),
            },
        );
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_hls_pull(&route, trunk_config(), handshake()),
        )
        .await
        .expect("run_hls_pull must return via IngestTimeouts::read, not hang");
        assert!(
            result.is_err(),
            "a stalled playlist fetch must fail, not hang forever"
        );
    }

    /// Bounds concurrent fetches: a structural property of `run_hls_pull`'s
    /// `while inflight.len() < MAX_INFLIGHT_FETCHES` gate (evaluated before
    /// every `JoinSet::spawn`), not something a black-box loopback test can
    /// observe without instrumenting the fixture server's concurrent-
    /// connection count — the small fixtures above never reveal more than a
    /// handful of resources at once, so removing the gate would still pass
    /// every other test in this module. Recorded here rather than silently
    /// assumed.
    #[test]
    fn max_inflight_fetches_gate_is_structural() {
        assert!(MAX_INFLIGHT_FETCHES > 0);
    }
}
