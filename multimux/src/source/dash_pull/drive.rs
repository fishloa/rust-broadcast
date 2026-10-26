//! The tokio drive loop of `dash_pull`: the wire half. It performs the GETs the
//! core asks for (reqwest, timeouts, byte ranges, conditional MPD refreshes),
//! turns what came back into [`DashInput`]s, paces retries and the host clock
//! anchor, and leaves every DASH decision to the session.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::{Stage, Timestamp};
use dash_runtime::client::{Action, Failure, ResourceId, ResponseMeta, UtcMethod};
use headers::{ContentRange, HeaderMapExt, Range};
use media_plane::ingress::{HandshakePolicy, IngestDriver, run_dial};
use media_plane::trunk::TrunkConfig;
use reqwest::header::{
    DATE, ETAG, HeaderMap, HeaderName, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};
use reqwest::{Client as HttpClient, Method, StatusCode};

use super::session::{DashIngestSession, DashInput, FetchTarget, RetryTarget};
use super::{DashPullDialer, DashPullRoute, build_client};
use crate::clock::HostClock;
use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{ScopedCredentials, authenticated_get_with};
use crate::source::pull::{FetchOutcome, FetchResult, PendingFetch, PullScheduler};
use crate::source::{BodyError, read_body_bounded};

/// How long the loop parks when nothing is in flight and nothing is due.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// The least the loop waits for a core wake that is already due, so a wake the
/// core cannot satisfy yet never becomes a spin.
const MIN_WAKE_STEP: Duration = Duration::from_millis(1);
/// First retry delay after a failed fetch; doubles per consecutive failure.
const RETRY_BASE: Duration = Duration::from_millis(500);
/// Longest retry delay.
const RETRY_CAP: Duration = Duration::from_secs(4);
/// The most the loop reads of a `UTCTiming` answer.
const UTC_BODY_LIMIT: usize = 4 * 1024;
const NANOS_PER_MICRO: u64 = 1_000;

/// Where the loop publishes what the driver announced, and how it reports the
/// end of a connection attempt.
pub(super) trait RouteSink: Send {
    /// Publish whatever the driver has announced since the last call.
    fn advance<'a>(
        &'a mut self,
        driver: &'a IngestDriver<DashIngestSession>,
    ) -> impl Future<Output = ()> + Send + 'a;

    /// Pass the attempt's terminal result through.
    fn finish(&mut self, result: Result<()>) -> Result<()>;
}

#[derive(Debug)]
enum Outcome {
    Body,
    Unchanged,
    Failed(Failure),
    Unauthorized,
}

/// The scheduler key: which fetch finished and what the exchange measured.
#[derive(Debug)]
struct DashFetch {
    target: FetchTarget,
    sent_us: u64,
    received_us: u64,
    date: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    final_url: Option<String>,
    outcome: Outcome,
}

impl DashFetch {
    fn new(target: FetchTarget, sent_us: u64) -> Self {
        Self {
            target,
            sent_us,
            received_us: sent_us,
            date: None,
            etag: None,
            last_modified: None,
            final_url: None,
            outcome: Outcome::Body,
        }
    }
}

#[derive(Clone)]
struct Net {
    http: HttpClient,
    credentials: Option<ScopedCredentials>,
    clock: Arc<dyn HostClock>,
    read_timeout: Duration,
    utc_timeout: Duration,
    mpd_cap: usize,
    segment_cap: usize,
}

struct Request {
    target: FetchTarget,
    method: Method,
    url: String,
    byte_range: Option<(u64, u64)>,
    conditional: Vec<(HeaderName, String)>,
    with_credentials: bool,
    what: &'static str,
    timeout: Duration,
    cap: usize,
}

impl Request {
    fn from_action(action: Action, net: &Net) -> Option<Self> {
        let resource = |id: ResourceId, url: String, byte_range, what| Request {
            target: FetchTarget::Resource(id),
            method: Method::GET,
            url,
            byte_range,
            conditional: Vec::new(),
            with_credentials: true,
            what,
            timeout: net.read_timeout,
            cap: net.segment_cap,
        };
        match action {
            Action::FetchMpd { url, validators } => {
                let mut conditional = Vec::new();
                if let Some(etag) = validators.etag {
                    conditional.push((IF_NONE_MATCH, etag));
                }
                if let Some(modified) = validators.last_modified {
                    conditional.push((IF_MODIFIED_SINCE, modified));
                }
                Some(Request {
                    target: FetchTarget::Mpd,
                    method: Method::GET,
                    url,
                    byte_range: None,
                    conditional,
                    with_credentials: true,
                    what: "mpd",
                    timeout: net.read_timeout,
                    cap: net.mpd_cap,
                })
            }
            Action::FetchUtcTiming {
                id, url, method, ..
            } => Some(Request {
                target: FetchTarget::Utc(id),
                method: match method {
                    UtcMethod::Head => Method::HEAD,
                    _ => Method::GET,
                },
                url,
                byte_range: None,
                conditional: Vec::new(),
                with_credentials: false,
                what: "utc",
                timeout: net.utc_timeout,
                cap: UTC_BODY_LIMIT,
            }),
            Action::FetchInit {
                id,
                url,
                byte_range,
            } => Some(resource(id, url, byte_range, "init")),
            Action::FetchIndex {
                id,
                url,
                byte_range,
            } => Some(resource(id, url, byte_range, "index")),
            Action::FetchSegment {
                id,
                url,
                byte_range,
            } => Some(resource(id, url, byte_range, "segment")),
            _ => None,
        }
    }

    fn headers(&self) -> Vec<(HeaderName, String)> {
        let mut headers = self.conditional.clone();
        if let Some((offset, length)) = self.byte_range
            && let Ok(range) = Range::bytes(offset..offset.saturating_add(length))
        {
            let mut map = HeaderMap::new();
            map.typed_insert(range);
            for (name, value) in &map {
                if let Ok(value) = value.to_str() {
                    headers.push((name.clone(), value.to_owned()));
                }
            }
        }
        headers
    }
}

fn failed(
    mut fetch: DashFetch,
    failure: Failure,
    error: MultimuxError,
) -> (DashFetch, FetchResult) {
    fetch.outcome = Outcome::Failed(failure);
    (fetch, FetchResult::Failed(error))
}

fn header_text(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_owned())
}

async fn send(net: &Net, req: &Request) -> Result<reqwest::Response> {
    if req.with_credentials {
        authenticated_get_with(
            &net.http,
            &req.url,
            net.credentials.as_ref(),
            &req.headers(),
        )
        .await
    } else {
        net.http
            .request(req.method.clone(), &req.url)
            .send()
            .await
            .map_err(|e| MultimuxError::Connect {
                reason: format!(
                    "{} {}: {e}",
                    req.method,
                    crate::redact::redact_url(&req.url)
                ),
            })
    }
}

/// The part of a ranged response the caller asked for, or `None` when the
/// server's answer does not match the request.
pub(super) fn ranged_body(
    status: StatusCode,
    content_range: Option<(u64, u64)>,
    body: Vec<u8>,
    (offset, length): (u64, u64),
) -> Option<Vec<u8>> {
    let length = usize::try_from(length).ok()?;
    if status == StatusCode::PARTIAL_CONTENT {
        let last = offset
            .checked_add(u64::try_from(length).ok()?)?
            .checked_sub(1)?;
        let honoured = content_range.is_none_or(|range| range == (offset, last));
        return (honoured && body.len() == length).then_some(body);
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(length)?;
    body.get(start..end).map(<[u8]>::to_vec)
}

async fn exchange(net: &Net, req: &Request, sent_us: u64) -> (DashFetch, FetchResult) {
    let mut fetch = DashFetch::new(req.target, sent_us);
    let response = match send(net, req).await {
        Ok(response) => response,
        Err(error) => return failed(fetch, Failure::Transport, error),
    };
    fetch.received_us = net.clock.mono_us();
    let status = response.status();
    let headers = response.headers();
    fetch.date = header_text(headers, &DATE);
    fetch.etag = header_text(headers, &ETAG);
    fetch.last_modified = header_text(headers, &LAST_MODIFIED);
    fetch.final_url = (response.url().as_str() != req.url).then(|| response.url().to_string());
    let content_range = headers
        .typed_get::<ContentRange>()
        .and_then(|r| r.bytes_range());
    if status == StatusCode::UNAUTHORIZED {
        fetch.outcome = Outcome::Unauthorized;
        let error = MultimuxError::Auth {
            reason: format!("dash-pull {}: {status}", req.what),
        };
        return (fetch, FetchResult::Failed(error));
    }
    if status == StatusCode::NOT_MODIFIED && req.target == FetchTarget::Mpd {
        fetch.outcome = Outcome::Unchanged;
        return (fetch, FetchResult::Ready(Vec::new()));
    }
    if !status.is_success() {
        let error = MultimuxError::Connect {
            reason: format!("dash-pull {}: HTTP {status}", req.what),
        };
        return failed(fetch, Failure::Status(status.as_u16()), error);
    }
    let body = match read_body_bounded(response, req.cap).await {
        Ok(body) => body,
        Err(BodyError::Read(e)) => {
            let error = MultimuxError::Connect {
                reason: format!("dash-pull {} read: {e}", req.what),
            };
            return failed(fetch, Failure::Transport, error);
        }
        Err(_) => {
            let error = MultimuxError::Connect {
                reason: format!(
                    "dash-pull {}: body exceeds the {}-byte cap",
                    req.what, req.cap
                ),
            };
            return failed(fetch, Failure::TooLarge, error);
        }
    };
    let body = match req.byte_range {
        None => body,
        Some(range) => match ranged_body(status, content_range, body, range) {
            Some(body) => body,
            None => {
                let error = MultimuxError::Connect {
                    reason: format!(
                        "dash-pull {}: the response does not match the requested byte range",
                        req.what
                    ),
                };
                return failed(fetch, Failure::Transport, error);
            }
        },
    };
    (fetch, FetchResult::Ready(body))
}

fn spawn_fetch(net: &Net, req: Request) -> PendingFetch<DashFetch> {
    let net = net.clone();
    PendingFetch {
        delay: Duration::ZERO,
        fut: Box::pin(async move {
            let sent_us = net.clock.mono_us();
            match tokio::time::timeout(req.timeout, exchange(&net, &req, sent_us)).await {
                Ok(done) => done,
                Err(_) => {
                    let error = MultimuxError::Connect {
                        reason: format!("dash-pull {} read exceeded {:?}", req.what, req.timeout),
                    };
                    failed(DashFetch::new(req.target, sent_us), Failure::Timeout, error)
                }
            }
        }),
    }
}

pub(super) fn backoff(attempt: u32) -> Duration {
    let doublings = attempt.saturating_sub(1).min(8);
    RETRY_BASE.saturating_mul(1u32 << doublings).min(RETRY_CAP)
}

fn stamp(now_us: u64) -> Timestamp {
    Timestamp::from_nanos(now_us.saturating_mul(NANOS_PER_MICRO))
}

fn wake_us(driver: &IngestDriver<DashIngestSession>) -> Option<u64> {
    driver
        .session()
        .next_deadline()
        .map(|t| t.as_nanos() / NANOS_PER_MICRO)
}

/// Turns a driver that has reached a terminal health state into this crate's
/// own `Result`; see `hls_pull` for why a drive loop must check health after
/// every feed.
fn terminal_result(driver: IngestDriver<DashIngestSession>) -> Result<()> {
    match driver.into_health() {
        media_plane::ingress::HealthState::Failed(e) => Err(e),
        media_plane::ingress::HealthState::HandshakeTimedOut { deadline } => {
            Err(MultimuxError::Connect {
                reason: format!("dash-pull: handshake deadline {deadline:?} passed"),
            })
        }
        _ => Ok(()),
    }
}

/// Drive `route` until a static presentation has played to its end, a fetch
/// the policy cannot absorb fails, or the core stops the presentation. A live
/// presentation never ends on its own.
pub(super) async fn run<S: RouteSink>(
    route: &DashPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    clock: Arc<dyn HostClock>,
    mut sink: S,
) -> Result<()> {
    let (http, clean_url, credentials) = build_client(route)?;
    let dash = super::session::dash_config(route.allow_cross_origin);
    let limits = super::session::pull_limits();
    let net = Net {
        http,
        credentials,
        clock: Arc::clone(&clock),
        read_timeout: route.timeouts.read,
        utc_timeout: Duration::from_micros(dash.utc_timeout_us),
        mpd_cap: limits.limits.max_bytes,
        segment_cap: limits.max_segment_bytes,
    };
    let resync_us = dash.utc_resync_us;
    let mut dialer = DashPullDialer::new(clean_url, dash, limits);
    let mut driver = run_dial(
        &mut dialer,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    )?;

    let mut scheduler: PullScheduler<DashFetch> =
        PullScheduler::new(crate::source::MAX_INFLIGHT_FETCHES);
    let mut retries: BTreeMap<RetryTarget, u64> = BTreeMap::new();
    let mut next_anchor_us = 0u64;

    loop {
        let now_us = clock.mono_us();
        driver.session_mut().set_now(now_us);
        if now_us >= next_anchor_us {
            let unix_us = clock.unix_us();
            driver
                .session_mut()
                .set_wall_clock(clock.mono_us(), unix_us);
            next_anchor_us = now_us.saturating_add(resync_us);
        }
        let due: Vec<RetryTarget> = retries
            .iter()
            .filter(|(_, at)| **at <= now_us)
            .map(|(target, _)| *target)
            .collect();
        for target in due {
            retries.remove(&target);
            driver.session_mut().retry(target);
        }
        while let Some(action) = driver.poll_transmit() {
            if let Some(req) = Request::from_action(action, &net) {
                scheduler.push(spawn_fetch(&net, req));
            }
        }
        if wake_us(&driver).is_some_and(|wake| wake <= now_us) {
            driver.on_deadline(stamp(now_us));
            sink.advance(&driver).await;
        }
        scheduler.pump();

        if !driver.health().is_running() {
            sink.advance(&driver).await;
            return sink.finish(terminal_result(driver));
        }
        if driver.session().failed() {
            driver.finish();
            sink.advance(&driver).await;
            continue;
        }
        if driver.session().ended() && scheduler.is_idle() {
            driver.finish();
            sink.advance(&driver).await;
            return sink.finish(terminal_result(driver));
        }

        let wake = [
            wake_us(&driver),
            retries.values().min().copied(),
            Some(next_anchor_us),
        ]
        .into_iter()
        .flatten()
        .min();
        let hint =
            wake.map(|at| Duration::from_micros(at.saturating_sub(now_us)).max(MIN_WAKE_STEP));

        let joined = scheduler.next(hint, IDLE_POLL_INTERVAL).await;
        let now_us = clock.mono_us();
        match joined {
            Some(FetchOutcome::Ready(fetch, body)) => {
                feed_response(&mut driver, fetch, &body, now_us);
            }
            Some(FetchOutcome::Failed(fetch, error) | FetchOutcome::NotReady(fetch, error)) => {
                match fetch.outcome {
                    Outcome::Unauthorized => return Err(error),
                    Outcome::Failed(failure) => {
                        if fetch.target == FetchTarget::Mpd && !driver.session().mpd_accepted() {
                            return Err(error);
                        }
                        let ts = stamp(now_us);
                        driver.feed(
                            DashInput::Failed {
                                target: fetch.target,
                                failure,
                            },
                            ts,
                        );
                    }
                    _ => return Err(error),
                }
            }
            Some(FetchOutcome::TaskPanic(detail)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("dash-pull: fetch task failed: {detail}"),
                });
            }
            None => continue,
        }
        for retry in driver.session_mut().take_retries() {
            let at = now_us.saturating_add(
                u64::try_from(backoff(retry.attempt).as_micros()).unwrap_or(u64::MAX),
            );
            retries.insert(retry.target, at);
        }
        sink.advance(&driver).await;
    }
}

fn feed_response(
    driver: &mut IngestDriver<DashIngestSession>,
    fetch: DashFetch,
    body: &[u8],
    now_us: u64,
) {
    let mut meta = ResponseMeta::new(fetch.sent_us, fetch.received_us);
    if let Some(date) = fetch.date.as_deref() {
        meta = meta.with_date(date);
    }
    if let Some(etag) = fetch.etag.as_deref() {
        meta = meta.with_etag(etag);
    }
    if let Some(modified) = fetch.last_modified.as_deref() {
        meta = meta.with_last_modified(modified);
    }
    if let Some(url) = fetch.final_url.as_deref() {
        meta = meta.with_final_url(url);
    }
    let input = match (fetch.target, &fetch.outcome) {
        (FetchTarget::Mpd, Outcome::Unchanged) => DashInput::MpdUnchanged { meta },
        (FetchTarget::Mpd, Outcome::Body) => DashInput::Mpd { body, meta },
        (FetchTarget::Utc(id), Outcome::Body) => DashInput::Utc {
            id,
            response: utc_response(&fetch, body),
        },
        (FetchTarget::Resource(id), Outcome::Body) => DashInput::Resource { id, body },
        _ => return,
    };
    driver.feed(input, stamp(now_us));
}

fn utc_response<'a>(fetch: &'a DashFetch, body: &'a [u8]) -> dash_runtime::client::UtcResponse<'a> {
    let mut response =
        dash_runtime::client::UtcResponse::new(fetch.sent_us, fetch.received_us, body);
    if let Some(date) = fetch.date.as_deref() {
        response = response.with_date(date);
    }
    response
}
