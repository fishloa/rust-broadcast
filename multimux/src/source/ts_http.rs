//! MPEG-2 Transport Stream over HTTP ingest source (issue #663 P3c; ported
//! onto the media-plane ingress traits at plan step 5a): a streaming HTTP
//! GET (chunked/progressive, `reqwest`) feeding the shared
//! [`crate::source::ts_program::TsIngestSession`].
//!
//! Like [`crate::source::ts_udp`], this module owns **only the transport**
//! — all PAT/PMT/PES demuxing and `DemuxEvent`→`SessionEvent` translation
//! (including the B5 mid-stream `NewProgram` handling) lives in
//! [`crate::source::ts_program`], shared verbatim between the three TS
//! transports. Before this port, this file carried its own near-identical
//! copy of that drain loop.
//!
//! # Shared policy core
//!
//! Finite/VOD detection and the post-establishment stall decision come from
//! `tshttp_runtime::TsHttpPolicy` (the demux-free core also under
//! `broadcast-client`); demuxing stays in [`crate::source::ts_program`] and
//! reconnect/backoff stays with the route supervisor, so the core's `Reconnect`
//! actions are ignored here.
//!
//! # Where the I/O boundary falls, and why `dial()` is still no-I/O
//!
//! Opening the HTTP body stream **is** real I/O — one GET (two if a Digest
//! `401` challenge has to be answered; see
//! `crate::source::http_auth::authenticated_get`). But it is
//! *transport-opening*, not a media handshake: it is the same category as
//! [`crate::source::ts_udp`]'s `UdpSocket::bind` and
//! [`crate::source::rtsp`]'s `TcpStream::connect`, and in this design that
//! has always been the driver's job, never the session's. So
//! [`TsHttpDialer::dial`] constructs the sans-IO session and performs no
//! I/O, and [`open_stream`]/[`run_ts_http`] — the tokio-side driver — does
//! the GET and then feeds body chunks in.
//!
//! **`reqwest` has no sans-IO core** (its `Response::bytes_stream()` is
//! bound to a live hyper connection; there is no bytes-in/bytes-out type),
//! so unlike [`crate::source::rtsp`] — which could delegate its whole
//! multi-round-trip handshake to `rtsp-runtime`'s sans-IO `ClientSession` —
//! there is nothing here to delegate the GET to. That is fine precisely
//! because the GET is not a handshake: nothing about it needs to be
//! expressed through `poll_transmit`/`feed`. Once the body is open, the
//! *entire* remaining protocol is "bytes arrive, demux them", which is
//! exactly `Stage::feed`.
//!
//! Auth itself **is** sans-IO (`broadcast_auth::Authenticator` is pure
//! challenge-string in, `Authorization`-header out) and is unchanged by this
//! port. Credentials come from [`TsHttpRoute::with_auth`] (config-supplied,
//! e.g. a Bearer token — the only way to supply one, since it has no
//! URL-userinfo form) if set, else the connect URL's own userinfo — see
//! `crate::source::http_auth::resolve_credentials`.
//!
//! # End-of-stream is a real, distinct outcome here
//!
//! Unlike UDP (connectionless, never signals end-of-stream), an HTTP
//! response body *does* end. [`recv_and_feed`] reports that as
//! [`StreamStatus::Ended`], which [`run_ts_http`] turns into
//! `IngestDriver::finish()` → [`media_plane::ingress::HealthState::Ended`]
//! — genuinely distinct from [`media_plane::ingress::HealthState::Failed`]
//! for a read stall or a transport error. That distinction is exactly the
//! `HealthState` fix plan step 3c made producible, and this is the first
//! ported source that can actually produce both.

use std::convert::Infallible;
use std::time::Duration;
#[cfg(test)]
use transmux::TrackId;

use broadcast_common::Timestamp;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use media_plane::ingress::{Dialer, HandshakePolicy, IngestDriver, ProgramId};
use media_plane::trunk::TrunkConfig;
use reqwest::Client;
use tshttp_runtime::{Action, Config as PolicyConfig, Event as PolicyEvent, TsHttpPolicy};
use url::Url;

use broadcast_auth::Credentials;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, scope_credentials, strip_userinfo,
};
use crate::source::ts_program::TsIngestSession;
use crate::source::{IngestTimeouts, Source};

/// An MPEG-2 TS-over-HTTP route: an `http(s)://` URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`). Replaces the old (pre-5a)
/// `TsHttpSource`; [`run_ts_http`] is the new drive loop.
#[derive(Clone)]
pub struct TsHttpRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo.
    auth: Option<Credentials>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for TsHttpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsHttpRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl TsHttpRoute {
    /// Build a route descriptor.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        TsHttpRoute {
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

impl Source for TsHttpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Constructs a [`TsIngestSession`] — performs **no I/O** (see the module
/// doc). The GET lives in [`open_stream`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TsHttpDialer;

impl Dialer for TsHttpDialer {
    type Session = TsIngestSession;
    /// Construction cannot fail — URL parsing and the GET both belong to
    /// [`open_stream`], the I/O side.
    type Error = Infallible;

    fn dial(&mut self) -> core::result::Result<TsIngestSession, Infallible> {
        Ok(TsIngestSession::new())
    }
}

/// The open HTTP response body a [`TsIngestSession`] is fed from.
pub type TsHttpStream = BoxStream<'static, reqwest::Result<Vec<u8>>>;

/// Opens the streaming GET for `route` (answering a `401` Digest challenge
/// if credentials were supplied), returning the response body stream —
/// **transport-opening I/O, deliberately outside `dial()`**; see the module
/// doc.
///
/// # Errors
/// A URL that will not parse, a transport failure, or a non-2xx status
/// (`401` maps to [`MultimuxError::Auth`], anything else to
/// [`MultimuxError::Connect`]).
pub async fn open_stream(route: &TsHttpRoute) -> Result<TsHttpStream> {
    open_response(route).await.map(|(stream, _)| stream)
}

/// The response head the policy core needs: status and header pairs.
pub(crate) struct ResponseHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

pub(crate) async fn open_response(route: &TsHttpRoute) -> Result<(TsHttpStream, ResponseHead)> {
    let parsed = Url::parse(&route.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad TS-over-HTTP URL {}: {e}",
            crate::redact::redact_url(&route.url)
        ),
    })?;
    let credentials = resolve_credentials(route.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let credentials = scope_credentials(credentials, &clean_url);

    let client = Client::builder()
        .redirect(crate::source::redirect_policy())
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    let response = authenticated_get(&client, clean_url.as_str(), credentials.as_ref()).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
            MultimuxError::Auth {
                reason: format!("ts/http: {status}"),
            }
        } else {
            MultimuxError::Connect {
                reason: format!("ts/http: HTTP {status}"),
            }
        });
    }
    let head = ResponseHead {
        status: status.as_u16(),
        headers: response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_owned(), v.to_owned()))
            })
            .collect(),
    };
    let stream = response
        .bytes_stream()
        .map(|item| item.map(|b| b.to_vec()))
        .boxed();
    Ok((stream, head))
}

/// What one [`recv_and_feed`] call observed on the body stream.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamStatus {
    /// A chunk was read and fed to the driver — carries the chunk's own
    /// bytes so the caller can also feed the route's DVR EIT tracker
    /// (`RouteHandle::feed_si_ts`, crate-private — issue #903) without a
    /// second read.
    Fed(Vec<u8>),
    /// The response body ended cleanly — not an error. See the module doc's
    /// "End-of-stream is a real, distinct outcome here".
    Ended,
}

/// Reads the next body chunk (bounded by `read_timeout`) and feeds it to
/// `driver`.
///
/// # Errors
/// A read stall or a transport-level chunk error — both genuine failures,
/// distinct from the clean [`StreamStatus::Ended`].
pub async fn recv_and_feed(
    stream: &mut TsHttpStream,
    driver: &mut IngestDriver<TsIngestSession>,
    read_timeout: Duration,
    now: Timestamp,
) -> Result<StreamStatus> {
    let Some(chunk) = tokio::time::timeout(read_timeout, stream.next())
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!("ts/http stream read: no data within {read_timeout:?}"),
        })?
    else {
        return Ok(StreamStatus::Ended);
    };
    let chunk = chunk.map_err(|e| MultimuxError::Connect {
        reason: format!("ts/http stream read: {e}"),
    })?;
    driver.feed(&chunk, now);
    Ok(StreamStatus::Fed(chunk))
}

const MIN_START_TIMEOUT_MS: u64 = 1;
const STALL_NOTICE_DIVISOR: u64 = 2;
const MIN_STALL_RECONNECT_MS: u64 = 2;

fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// The policy-core configuration for a route: the post-establishment stall
/// threshold is the route's read timeout, the core's start timeout is the
/// route's connect budget (the driver is `Live` from the first byte, so the
/// handshake deadline alone never bounds a connection that sends bytes but no
/// media), and backoff is unused because the supervisor owns reconnect.
pub(crate) fn policy_config(timeouts: IngestTimeouts) -> PolicyConfig {
    let reconnect = duration_ms(timeouts.read).max(MIN_STALL_RECONNECT_MS);
    PolicyConfig {
        start_timeout_ms: duration_ms(timeouts.connect).max(MIN_START_TIMEOUT_MS),
        stall_notice_ms: reconnect / STALL_NOTICE_DIVISOR,
        stall_reconnect_ms: reconnect,
        ..PolicyConfig::default()
    }
}

/// The ts/http source's [`IngestStep`](crate::source::driver::IngestStep):
/// one bounded body-chunk read, feeding the driver, the route's DVR EIT
/// tracker and the shared TS-over-HTTP policy core (`tshttp-runtime`).
pub(crate) struct TsHttpStep {
    pub stream: TsHttpStream,
    pub start: tokio::time::Instant,
    pub route_handle: std::sync::Arc<crate::route::RouteHandle>,
    pub policy: TsHttpPolicy,
    read: Duration,
    connect: Duration,
    media_seen: bool,
}

enum Verdict {
    Continue,
    Ended,
    Stalled,
    Truncated,
}

impl TsHttpStep {
    pub(crate) fn new(
        stream: TsHttpStream,
        head: &ResponseHead,
        timeouts: IngestTimeouts,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
    ) -> Self {
        let start = tokio::time::Instant::now();
        let mut policy = TsHttpPolicy::new("", policy_config(timeouts), 0);
        policy.on_response(head.status, &head.headers, 0);
        let mut step = TsHttpStep {
            stream,
            start,
            route_handle,
            policy,
            read: timeouts.read,
            connect: timeouts.connect,
            media_seen: false,
        };
        step.verdict();
        step
    }

    fn now_ms(&self) -> u64 {
        duration_ms(self.start.elapsed())
    }

    fn verdict(&mut self) -> Verdict {
        let mut verdict = Verdict::Continue;
        while let Some(event) = self.policy.poll_event() {
            match event {
                PolicyEvent::Ended => verdict = Verdict::Ended,
                PolicyEvent::Reconnecting { .. } if self.policy.is_finite() => {
                    verdict = Verdict::Truncated;
                }
                PolicyEvent::StallNotice { secs } => {
                    tracing::debug!(secs, "ts/http stalled");
                }
                PolicyEvent::Finite { expected_bytes } => {
                    tracing::debug!(expected_bytes, "ts/http finite response");
                }
                _ => {}
            }
        }
        while let Some(action) = self.policy.poll() {
            if matches!(action, Action::Abort) {
                verdict = Verdict::Stalled;
            }
        }
        verdict
    }

    fn no_media(&self) -> MultimuxError {
        MultimuxError::Connect {
            reason: format!(
                "ts/http handshake timed out: no media within {:?}",
                self.connect
            ),
        }
    }

    fn capped_window(&self, window: Duration) -> Duration {
        match self.policy.next_deadline() {
            Some(deadline) => window.min(Duration::from_millis(
                deadline.saturating_sub(self.now_ms()),
            )),
            None => window,
        }
    }
}

impl crate::source::driver::IngestStep<TsIngestSession> for TsHttpStep {
    async fn step(
        &mut self,
        driver: &mut IngestDriver<TsIngestSession>,
        window: Duration,
    ) -> crate::source::driver::StepOutcome {
        use crate::source::driver::StepOutcome;
        let wait = self.capped_window(window);
        let chunk = match tokio::time::timeout(wait, self.stream.next()).await {
            Ok(Some(Ok(chunk))) => chunk,
            Ok(Some(Err(e))) => {
                return StepOutcome::Failed(MultimuxError::Connect {
                    reason: format!("ts/http stream read: {e}"),
                });
            }
            Ok(None) => {
                let now_ms = self.now_ms();
                self.policy.on_eof(now_ms);
                return match self.verdict() {
                    Verdict::Truncated => StepOutcome::Failed(MultimuxError::Connect {
                        reason: "ts/http stream read: truncated transfer".into(),
                    }),
                    _ => StepOutcome::Eof,
                };
            }
            Err(_) => {
                let now_ms = self.now_ms();
                self.policy.poll_timeout(now_ms);
                return match self.verdict() {
                    Verdict::Stalled if self.media_seen => {
                        StepOutcome::Failed(self.stalled(self.read))
                    }
                    Verdict::Stalled => StepOutcome::Failed(self.no_media()),
                    _ => StepOutcome::Idle,
                };
            }
        };
        let now = Timestamp::from_instant(
            self.start.into_std(),
            tokio::time::Instant::now().into_std(),
        );
        driver.feed(&chunk, now);
        self.route_handle.feed_si_ts(&chunk);
        self.media_seen |= driver.trunk(ProgramId(0)).is_some();
        let now_ms = self.now_ms();
        self.policy.on_bytes(chunk.len(), now_ms, self.media_seen);
        self.verdict();
        StepOutcome::Received
    }

    fn stalled(&self, read: Duration) -> MultimuxError {
        MultimuxError::Connect {
            reason: format!("ts/http stream read: no data within {read:?}"),
        }
    }
}

/// Opens `route`'s GET and drives a fresh [`TsIngestSession`] through
/// [`IngestDriver`] until the body ends or a read fails — the new drive
/// loop, replacing the pre-5a `TsHttpSource`/`TsHttpSession` pair.
///
/// Returns `Ok(())` on a clean end-of-body (the driver is left
/// [`media_plane::ingress::HealthState::Ended`]) and `Err` on a genuine
/// failure — the distinction the route supervisor should act on differently.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_ts_http(
    route: &TsHttpRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    _cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let (stream, head) = open_response(route).await?;
    let mut dialer = TsHttpDialer;
    let session = dialer
        .dial()
        .unwrap_or_else(|never: Infallible| match never {});
    let driver = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let mut step = TsHttpStep::new(
        stream,
        &head,
        route.timeouts,
        std::sync::Arc::clone(route_handle),
    );
    let mut progress = crate::source::DriverProgress::new();
    let mut sink = tokio::io::sink();
    let err = crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        &mut progress,
        _cancel,
        route.timeouts,
        crate::source::driver::DEFAULT_WRITE_TIMEOUT,
        &mut sink,
        &mut step,
        |never: Infallible| match never {},
    )
    .await;
    // A clean end-of-body ends the session `Ended`; the scaffold reports it
    // as a `Connect { "session ended .." }` error, so map it back to `Ok`.
    match err {
        MultimuxError::Connect { reason } if reason.starts_with("session ended") => Ok(()),
        other => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use axum::Router;
    use axum::body::Body;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use media_plane::ingress::{HealthState, ProgramId};

    /// Starts a tiny axum server streaming `body` in fixed-size chunks (a
    /// real chunked-transfer HTTP response, not a single buffered body) at
    /// `/stream.ts`, returning its base URL. `auth`, if given, gates every
    /// request behind that scheme (see `crate::testutil::require_auth`) —
    /// used by the auth-scheme biting tests below; `None` (the plain
    /// `start_chunked_ts_server` case) mirrors the original no-auth server.
    async fn start_chunked_ts_server_with_auth(
        body: Vec<u8>,
        auth: Option<crate::testutil::MockAuthScheme>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(body: axum::extract::State<Vec<u8>>) -> AxumResponse {
            // Stream in small chunks so the client genuinely reads the body
            // incrementally (biting on `bytes_stream()`, not just a single
            // buffered read).
            let chunks: Vec<std::result::Result<Vec<u8>, std::io::Error>> =
                body.0.chunks(7 * 188).map(|c| Ok(c.to_vec())).collect();
            let stream = futures_util::stream::iter(chunks);
            let body = Body::from_stream(stream);
            ([(axum::http::header::CONTENT_TYPE, "video/mp2t")], body).into_response()
        }
        let mut app = Router::new()
            .route("/stream.ts", get(handler))
            .with_state(body);
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
        (format!("http://{addr}/stream.ts"), server)
    }

    /// Starts a plain (no-auth) chunked TS server — see
    /// [`start_chunked_ts_server_with_auth`].
    async fn start_chunked_ts_server(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        start_chunked_ts_server_with_auth(body, None).await
    }

    /// Loopback biting test: a real axum server streams a real muxed TS
    /// fixture over chunked HTTP; asserts the ported route resolves the track
    /// set and yields real depayloaded samples.
    #[tokio::test]
    async fn loopback_http_ts_yields_samples_after_pmt_resolves() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server(ts_bytes).await;

        let route = TsHttpRoute::new("cam-ts-http", url);
        // HANG GUARD (issue #826): real loopback TCP open against an axum
        // server that is already listening before this line, so this
        // normally resolves in low ms. Only job is to fail "never connects"
        // rather than hang, not a timing claim.
        let mut stream = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .expect("open_stream timed out")
            .expect("open_stream");

        let mut dialer = TsHttpDialer;
        let session = dialer.dial().expect("dial is infallible and does no I/O");
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        assert!(matches!(driver.health(), HealthState::Establishing));

        let mut cursor = None;
        let mut saw_sample = false;
        let mut ended = false;
        for i in 0..200u64 {
            // HANG GUARD (issue #826): per-iteration read timeout within
            // a 200-iteration loopback test. Loopback TCP resolves in ~ms;
            // this only prevents fast spinning on an empty stream, not a
            // timing claim.
            match recv_and_feed(
                &mut stream,
                &mut driver,
                Duration::from_secs(60),
                Timestamp::from_nanos(i),
            )
            .await
            {
                Ok(StreamStatus::Fed(_)) => {}
                Ok(StreamStatus::Ended) => {
                    driver.finish();
                    ended = true;
                }
                Err(_) => break,
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                while let Some(item) = c.poll() {
                    if matches!(item, media_plane::trunk::SampleCursorItem::Timed { .. }) {
                        saw_sample = true;
                    }
                }
            }
            if ended {
                break;
            }
        }
        assert!(
            saw_sample,
            "expected at least one sample from the muxed TS stream over HTTP in the Trunk"
        );
        // The axum helper closes the body at end-of-stream, so this route
        // ends cleanly rather than failing — the `Ended` vs `Failed`
        // distinction step 3c made producible, now actually produced.
        assert!(ended, "the chunked server's body must end cleanly");
        assert!(
            matches!(driver.health(), HealthState::Ended),
            "a cleanly-ended HTTP body must be Ended, not Failed: {:?}",
            driver.health()
        );

        server.abort();
    }

    /// A `404` (or any non-2xx) must fail `connect`, not silently proceed as
    /// if a track set would eventually resolve.
    #[tokio::test]
    async fn connect_fails_on_non_success_status() {
        let app = Router::new().route(
            "/nope.ts",
            get(|| async { axum::http::StatusCode::NOT_FOUND }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });

        let route = TsHttpRoute::new("cam-ts-http", format!("http://{addr}/nope.ts"));
        // HANG GUARD (issue #826): the axum 404 handler returns immediately,
        // so `open_stream` fails in ~ms on the HTTP response. Only job is
        // to fail "never fails" rather than hang, not a timing claim.
        let result = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .expect("open_stream timed out");
        assert!(result.is_err(), "a 404 must fail open_stream()");

        server.abort();
    }

    /// Biting test (issue #663 P5, audit-ingest #3): a server that resolves
    /// the track set and then goes silent — never sends another chunk, never
    /// closes the connection — must fail `next_samples()` within the
    /// configured [`IngestTimeouts::read`], not hang forever (the exact
    /// wedged/half-open failure mode a "no read timeout" `TsHttpSession`
    /// used to hang on). A raw TCP listener (not the axum chunked helper
    /// above, which always closes at end-of-body) plays the "accepts then
    /// stalls mid-body" server: valid headers + the PMT-resolving TS bytes,
    /// promised via a `Content-Length` far larger than what's ever actually
    /// written, so the client's body stream genuinely waits for more.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_mid_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");

        let _server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await; // drain the request, unparsed
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\n\r\n",
                ts_bytes.len() * 10 // promise far more than will ever be sent
            );
            stream
                .write_all(header.as_bytes())
                .await
                .expect("write header");
            stream
                .write_all(&ts_bytes)
                .await
                .expect("write body prefix");
            // Go silent forever: never write again, never close — the
            // stalled/wedged-server failure mode.
            std::future::pending::<()>().await;
        });

        let route = TsHttpRoute::new("stalled", format!("http://{addr}/stream.ts")).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_secs(2),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // DISCRIMINATOR (issue #826): must prove the operation returns
        // through the CONFIGURED read timeout (above), not via any longer
        // system default. Gap widened: the configured read timeout was
        // raised from 100ms to 2s (load-tolerant scheduling) and the
        // assertion window from 5s to 10s — 10s is still well below any
        // plausible fallback, so a broken implementation that ignores the
        // configured timeout is still caught. MUTATION CHECKED: inflating
        // the configured read timeout to 30s makes the 10s outer timeout
        // fire first, producing `Elapsed` instead of `run_ts_http`'s own
        // error, proving this bound discriminates.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_ts_http(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect(
            "run_ts_http must return via IngestTimeouts::read, \
             not via the outer assertion window",
        );
        assert!(
            result.is_err(),
            "a server that goes silent mid-body must fail, not hang forever"
        );
    }

    const TS_PACKET_LEN: usize = 188;
    const HANDSHAKE_BUDGET: Duration = Duration::from_secs(1);
    const HANDSHAKE_MARGIN: Duration = Duration::from_secs(4);
    const LONG_READ: Duration = Duration::from_secs(30);
    const TRICKLE_PERIOD: Duration = Duration::from_millis(50);

    async fn pre_pmt_server(trickle: bool) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let pat_only = build_ts_bytes(TrackId::new(1), 0xAB, 10)[..TS_PACKET_LEN].to_vec();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .expect("write header");
            let chunk = [
                format!("{:x}\r\n", pat_only.len()).as_bytes(),
                &pat_only,
                b"\r\n",
            ]
            .concat();
            loop {
                if stream.write_all(&chunk).await.is_err() || !trickle {
                    break;
                }
                tokio::time::sleep(TRICKLE_PERIOD).await;
            }
            std::future::pending::<()>().await;
        });
        addr
    }

    async fn run_against(addr: std::net::SocketAddr) -> (Result<()>, Duration) {
        let route = TsHttpRoute::new("pre-pmt", format!("http://{addr}/stream.ts")).with_timeouts(
            IngestTimeouts {
                connect: HANDSHAKE_BUDGET,
                read: LONG_READ,
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            HANDSHAKE_BUDGET + HANDSHAKE_MARGIN,
            run_ts_http(
                &route,
                trunk_config(),
                crate::source::handshake_policy(HANDSHAKE_BUDGET),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("the handshake deadline must abort the route, not the outer window");
        (result, started.elapsed())
    }

    fn assert_handshake_timeout(result: &Result<()>) {
        match result {
            Err(MultimuxError::Connect { reason }) => {
                assert!(reason.contains("handshake timed out"), "{reason}");
            }
            other => panic!("expected a handshake timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_server_that_goes_silent_before_the_pmt_fails_at_the_handshake_deadline() {
        let addr = pre_pmt_server(false).await;
        let (result, elapsed) = run_against(addr).await;
        assert_handshake_timeout(&result);
        assert!(elapsed < LONG_READ, "{elapsed:?}");
    }

    #[tokio::test]
    async fn a_server_that_keeps_sending_without_ever_sending_a_pmt_fails_at_the_handshake_deadline()
     {
        let addr = pre_pmt_server(true).await;
        let (result, elapsed) = run_against(addr).await;
        assert_handshake_timeout(&result);
        assert!(elapsed < LONG_READ, "{elapsed:?}");
    }

    fn head(content_length: Option<usize>) -> ResponseHead {
        ResponseHead {
            status: 200,
            headers: content_length
                .map(|n| vec![("content-length".to_string(), n.to_string())])
                .unwrap_or_default(),
        }
    }

    fn fresh_driver() -> IngestDriver<TsIngestSession> {
        let mut dialer = TsHttpDialer;
        IngestDriver::new(
            dialer.dial().expect("infallible"),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        )
    }

    fn scripted_step(chunks: Vec<Vec<u8>>, head: &ResponseHead, read: Duration) -> TsHttpStep {
        let items: Vec<reqwest::Result<Vec<u8>>> = chunks.into_iter().map(Ok).collect();
        TsHttpStep::new(
            futures_util::stream::iter(items).boxed(),
            head,
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read,
            },
            std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4)),
        )
    }

    /// Mutation check: dropping the `policy.on_eof` call (or the finite
    /// bookkeeping) turns this into `Eof` regardless of the byte count.
    #[tokio::test]
    async fn eof_before_declared_length_is_a_truncated_failure() {
        use crate::source::driver::{IngestStep, StepOutcome};
        let ts = build_ts_bytes(TrackId::new(1), 0xAB, 4);
        let declared = ts.len() + 1;
        let mut step = scripted_step(vec![ts], &head(Some(declared)), Duration::from_secs(30));
        let mut driver = fresh_driver();
        assert!(matches!(
            step.step(&mut driver, Duration::from_secs(5)).await,
            StepOutcome::Received
        ));
        match step.step(&mut driver, Duration::from_secs(5)).await {
            StepOutcome::Failed(MultimuxError::Connect { reason }) => {
                assert!(reason.contains("truncated transfer"), "{reason}");
            }
            _ => panic!("EOF before the declared Content-Length must fail"),
        }
    }

    #[tokio::test]
    async fn eof_after_declared_length_is_a_clean_end() {
        use crate::source::driver::{IngestStep, StepOutcome};
        let ts = build_ts_bytes(TrackId::new(1), 0xAB, 4);
        let declared = ts.len();
        let mut step = scripted_step(vec![ts], &head(Some(declared)), Duration::from_secs(30));
        let mut driver = fresh_driver();
        assert!(matches!(
            step.step(&mut driver, Duration::from_secs(5)).await,
            StepOutcome::Received
        ));
        assert!(matches!(
            step.step(&mut driver, Duration::from_secs(5)).await,
            StepOutcome::Eof
        ));
    }

    #[tokio::test]
    async fn live_eof_without_length_stays_a_clean_end() {
        use crate::source::driver::{IngestStep, StepOutcome};
        let ts = build_ts_bytes(TrackId::new(1), 0xAB, 4);
        let mut step = scripted_step(vec![ts], &head(None), Duration::from_secs(30));
        let mut driver = fresh_driver();
        let _ = step.step(&mut driver, Duration::from_secs(5)).await;
        assert!(matches!(
            step.step(&mut driver, Duration::from_secs(5)).await,
            StepOutcome::Eof
        ));
    }

    /// The core, not the scaffold's window, decides the stall: `step` is given
    /// a 60 s window but the core's threshold is 300 ms, so it must fail well
    /// inside the window. Mutation check: removing the `poll_timeout` call or
    /// the `capped_window` cap leaves this returning `Idle` for the whole
    /// window and the outer 20 s bound fires instead.
    #[tokio::test]
    async fn stall_is_decided_by_the_policy_core_inside_the_window() {
        use crate::source::driver::{IngestStep, StepOutcome};
        let mut step = TsHttpStep::new(
            futures_util::stream::pending().boxed(),
            &head(None),
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_millis(300),
            },
            std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4)),
        );
        step.policy.on_bytes(1, 0, true);
        step.media_seen = true;
        let mut driver = fresh_driver();
        let outcome = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                match step.step(&mut driver, Duration::from_secs(60)).await {
                    StepOutcome::Idle => {}
                    other => return other,
                }
            }
        })
        .await
        .expect("the policy core must fail the stall inside the 60 s window");
        match outcome {
            StepOutcome::Failed(MultimuxError::Connect { reason }) => {
                assert!(reason.contains("no data within"), "{reason}");
            }
            _ => panic!("a stalled playing stream must fail"),
        }
    }

    // --- issue #663 "Finish client-side multi-scheme auth": Basic/Digest/
    // Bearer/wrong-creds against a real mock auth server ---

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "ts-http-bearer-token";

    /// Drives a `TsHttpRoute` to open and pull every sample the server
    /// serves, returning the sample count — the common "auth worked, real
    /// media came out" assertion shared by the Basic/Digest/Bearer tests
    /// below.
    async fn connect_and_drain(route: TsHttpRoute) -> Result<usize> {
        // HANG GUARD (issue #826): loopback TCP open against a server that
        // is already listening; normally resolves in ~ms. Only job is to
        // fail "never connects" rather than hang, not a timing claim.
        let mut stream = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: "open_stream timed out".into(),
            })??;
        let mut dialer = TsHttpDialer;
        let session = dialer.dial().expect("infallible");
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut cursor = None;
        let mut total = 0usize;
        for i in 0..200u64 {
            // HANG GUARD (issue #826): per-iteration read timeout within a
            // 200-iteration loopback test. Loopback TCP resolves in ~ms;
            // this only prevents fast spinning on an empty stream.
            match recv_and_feed(
                &mut stream,
                &mut driver,
                Duration::from_secs(60),
                Timestamp::from_nanos(i),
            )
            .await
            {
                Ok(StreamStatus::Fed(_)) => {}
                Ok(StreamStatus::Ended) | Err(_) => break,
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                while c.poll().is_some() {
                    total += 1;
                }
            }
        }
        Ok(total)
    }

    /// Basic (RFC 7617), credentials from URL userinfo: the server issues a
    /// Basic challenge, the route answers it via
    /// `source::http_auth::authenticated_get`'s retry path, and real samples
    /// come out.
    #[tokio::test]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Basic {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let route = TsHttpRoute::new("cam-basic", credentialed);
        let total = connect_and_drain(route)
            .await
            .expect("Basic auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Basic auth");

        server.abort();
    }

    /// Digest (RFC 7616), credentials from URL userinfo: the server issues a
    /// real Digest challenge (nonce/realm/qop=auth) via a real
    /// `broadcast_auth::Verifier` (`crate::testutil::require_auth`) that
    /// independently recomputes the expected response — a client that can't
    /// answer it gets nothing back, so this proves the route actually
    /// computed a correct Digest response via `broadcast-auth`, not just
    /// echoed something Digest-shaped.
    #[tokio::test]
    async fn digest_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let route = TsHttpRoute::new("cam-digest", credentialed);
        let total = connect_and_drain(route)
            .await
            .expect("Digest auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Digest auth");

        server.abort();
    }

    /// Bearer (RFC 6750), config-supplied (the only way to supply one — it
    /// has no URL-userinfo form): `TsHttpRoute::with_auth` overrides the
    /// (bare, no-userinfo) connect URL.
    #[tokio::test]
    async fn bearer_auth_config_supplied_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Bearer {
                token: BEARER_TOKEN.into(),
            }),
        )
        .await;

        let route =
            TsHttpRoute::new("cam-bearer", url).with_auth(Some(Credentials::bearer(BEARER_TOKEN)));
        let total = connect_and_drain(route)
            .await
            .expect("config-supplied Bearer token must authenticate");
        assert!(total > 0, "expected real samples after Bearer auth");

        server.abort();
    }

    /// Config-supplied auth takes precedence over URL userinfo: the URL
    /// carries a *wrong* password, but `TsHttpRoute::with_auth` supplies the
    /// correct one — connect must succeed on the config auth, proving
    /// `resolve_credentials` really overrides rather than merely falling
    /// back.
    #[tokio::test]
    async fn config_auth_overrides_wrong_url_userinfo() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let wrong_url_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);

        let route = TsHttpRoute::new("cam-override", wrong_url_creds)
            .with_auth(Some(Credentials::new(AUTH_USER, AUTH_PASS)));
        let total = connect_and_drain(route)
            .await
            .expect("config auth must override the URL's wrong userinfo password");
        assert!(
            total > 0,
            "expected real samples via config-overridden auth"
        );

        server.abort();
    }

    /// Wrong credentials must fail `connect()` (stay `401`), not hang or
    /// silently proceed — the negative counterpart to the three tests above,
    /// proving they actually bite (a client answering with the wrong
    /// password gets rejected exactly like a client with none).
    #[tokio::test]
    async fn wrong_credentials_stay_401_and_connect_errors() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let wrong_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);

        let route = TsHttpRoute::new("cam-wrong", wrong_creds);
        // HANG GUARD (issue #826): the mock server returns 401
        // immediately (no body to parse), so `open_stream` fails in ~ms.
        // Only job is to fail "never fails" rather than hang.
        let result = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .expect("open_stream must not hang against a persistent 401");
        assert!(
            result.is_err(),
            "wrong credentials must fail open_stream(), not silently proceed"
        );

        server.abort();
    }
}
