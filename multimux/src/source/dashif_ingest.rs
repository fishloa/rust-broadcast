//! DASH-IF Live Media Ingest (Interface-1) HTTP listener (increment P2b-1).

use std::convert::Infallible;
use std::sync::Arc;

use parking_lot::Mutex;

use axum::Router;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use broadcast_common::{Demand, Stage, Timestamp};
use bytes::Bytes;
use media_plane::ingress::{HandshakePolicy, IngestSession, SessionEvent};
use media_plane::trunk::TrunkConfig;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::error::MultimuxError;
use crate::origin;
use crate::route::RouteHandle;
use crate::source::DriverProgress;
use dash_runtime::ingest::session::SessionError;
use dash_runtime::ingest::{DashifIngestConfig, DashifIngestSession, IngestObject};
use dash_runtime::ingest::{ManifestGrouping, RequestMeta, classify};

use media_plane::ingress::IngestDriver;

/// Bounded channel between HTTP handlers and the per-publishing-point pump.
const CHANNEL_CAPACITY: usize = 64;

/// Default per-object body timeout of a published object.
const DEFAULT_BODY_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(30);

/// A job handed from an HTTP handler to the pump task.
enum Job {
    /// Buffered (non-segment) object: whole body at once.
    Whole {
        kind: dash_runtime::ingest::ObjectKind,
        bytes: Bytes,
        resp: oneshot::Sender<Result<(), SessionError>>,
    },
    /// Start a streamed segment (P3b).
    Begin {
        kind: dash_runtime::ingest::ObjectKind,
        resp: oneshot::Sender<Result<(), SessionError>>,
    },
    /// One body frame of an in-progress segment.
    Chunk {
        rep_id: String,
        bytes: Bytes,
        resp: oneshot::Sender<Result<(), SessionError>>,
    },
    /// Finalise an in-progress segment.
    Finish {
        rep_id: String,
        resp: oneshot::Sender<Result<(), SessionError>>,
    },
    /// Discard the partial remainder of an in-progress segment (fire-and-forget).
    Abort { rep_id: String },
}

/// The outcome of awaiting a pump reply.
enum Reply<T> {
    /// The pump answered.
    Got(T),
    /// The body deadline elapsed before the pump answered.
    Timeout,
    /// The pump dropped the reply sender.
    Closed,
}

/// Await one pump reply under the route's body deadline.
async fn wait_reply<T>(
    rx: oneshot::Receiver<T>,
    deadline: Option<tokio::time::Duration>,
) -> Reply<T> {
    match deadline {
        Some(d) => match tokio::time::timeout(d, rx).await {
            Ok(Ok(v)) => Reply::Got(v),
            Ok(Err(_)) => Reply::Closed,
            Err(_) => Reply::Timeout,
        },
        None => match rx.await {
            Ok(v) => Reply::Got(v),
            Err(_) => Reply::Closed,
        },
    }
}

/// Relinquishes an in-progress segment handle when the handler future is
/// dropped before `Finish` (a publisher disconnect mid-PUT).
struct SegmentGuard {
    tx: mpsc::Sender<Job>,
    rep_id: String,
    armed: bool,
}

impl SegmentGuard {
    fn new(tx: mpsc::Sender<Job>, rep_id: String) -> Self {
        Self {
            tx,
            rep_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SegmentGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.tx.try_send(Job::Abort {
                rep_id: self.rep_id.clone(),
            });
        }
    }
}

/// One DASH-IF ingest route.
pub struct DashifIngestRoute {
    name: String,
    listen: String,
    prebound: Mutex<Option<TcpListener>>,
    publishing_point: String,
    timeouts: broadcast_common::timeouts::Timeouts,
    auth: Option<Arc<broadcast_auth::Verifier>>,
    ingest: DashifIngestConfig,
    body_timeout: Option<tokio::time::Duration>,
}

impl std::fmt::Debug for DashifIngestRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashifIngestRoute")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("publishing_point", &self.publishing_point)
            .field("ingest", &self.ingest)
            .field("body_timeout", &self.body_timeout)
            .finish()
    }
}

impl DashifIngestRoute {
    pub fn new(
        name: impl Into<String>,
        listen: impl Into<String>,
        publishing_point: impl Into<String>,
    ) -> Self {
        DashifIngestRoute {
            name: name.into(),
            listen: listen.into(),
            prebound: Mutex::new(None),
            publishing_point: publishing_point.into(),
            timeouts: broadcast_common::timeouts::Timeouts::default(),
            auth: None,
            ingest: DashifIngestConfig::default(),
            body_timeout: Some(DEFAULT_BODY_TIMEOUT),
        }
    }

    pub fn with_listener(
        name: impl Into<String>,
        listener: TcpListener,
        publishing_point: impl Into<String>,
    ) -> Self {
        let listen = listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "127.0.0.1:0".to_string());
        DashifIngestRoute {
            name: name.into(),
            listen,
            prebound: Mutex::new(Some(listener)),
            publishing_point: publishing_point.into(),
            timeouts: broadcast_common::timeouts::Timeouts::default(),
            auth: None,
            ingest: DashifIngestConfig::default(),
            body_timeout: Some(DEFAULT_BODY_TIMEOUT),
        }
    }

    #[must_use]
    pub fn with_timeouts(mut self, timeouts: broadcast_common::timeouts::Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    #[must_use]
    pub fn with_auth(mut self, auth: Option<Arc<broadcast_auth::Verifier>>) -> Self {
        self.auth = auth;
        self
    }

    #[must_use]
    pub fn with_max_object_bytes(mut self, n: usize) -> Self {
        self.ingest = self.ingest.with_max_object_bytes(n);
        self
    }

    /// The cap on one buffered (non-segment) object.
    #[must_use]
    pub fn with_max_buffered_object_bytes(mut self, n: usize) -> Self {
        self.ingest = self.ingest.with_max_buffered_object_bytes(n);
        self
    }

    /// The bytes of buffered objects this route holds at once.
    #[must_use]
    pub fn with_buffered_budget_bytes(mut self, n: usize) -> Self {
        self.ingest = self.ingest.with_buffered_budget_bytes(n);
        self
    }

    #[must_use]
    pub fn with_body_timeout(mut self, t: Option<tokio::time::Duration>) -> Self {
        self.body_timeout = t;
        self
    }

    fn take_listener(&self) -> std::io::Result<TcpListener> {
        if let Some(l) = self.prebound.lock().take() {
            return Ok(l);
        }
        let std_listener = std::net::TcpListener::bind(&self.listen)?;
        std_listener.set_nonblocking(true)?;
        tokio::net::TcpListener::from_std(std_listener)
    }
}

/// Wraps [`DashifIngestSession`] so a per-request protocol error (a 400/412/…
/// answer for ONE POST) is handed back to the HTTP handler instead of failing
/// the `IngestDriver`: a driver treats any `Stage::feed` error as the end of
/// the session, which would let one bad request from one sender poison the
/// whole publishing point.
struct TolerantSession {
    inner: DashifIngestSession,
    last_error: Option<SessionError>,
}

impl TolerantSession {
    fn new(config: DashifIngestConfig) -> Self {
        TolerantSession {
            inner: DashifIngestSession::new(config),
            last_error: None,
        }
    }

    /// The error of the most recent `feed`, if it failed (cleared by this call).
    fn take_last_error(&mut self) -> Option<SessionError> {
        self.last_error.take()
    }
}

impl Stage for TolerantSession {
    type In<'a> = IngestObject<'a>;
    type Out = SessionEvent;
    type Error = Infallible;

    fn feed(&mut self, input: IngestObject<'_>, now: Timestamp) -> Result<(), Infallible> {
        self.last_error = self.inner.feed(input, now).err();
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.inner.poll()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        // The inner `finish` is infallible in practice; a failure is not a
        // per-request error and has no request to answer.
        let _ = self.inner.finish();
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        self.inner.next_deadline()
    }

    fn on_deadline(&mut self, now: Timestamp) {
        self.inner.on_deadline(now);
    }

    fn demand(&self) -> Demand {
        self.inner.demand()
    }
}

impl IngestSession for TolerantSession {
    type Request = Infallible;
}

/// Why reading a request body stopped.
enum ReadBodyError {
    /// More than the cap.
    TooLarge,
    /// The route's buffered-bytes budget is spent by other requests.
    OverBudget,
    /// The client's body failed or was cut short.
    Broken,
}

/// The buffered bytes a request holds against its route's budget; dropping it
/// returns them.
struct BudgetHold(Option<OwnedSemaphorePermit>);

impl BudgetHold {
    /// Reserve `n` more bytes, or `false` when the route's budget is spent.
    fn reserve(&mut self, budget: &Arc<Semaphore>, n: usize) -> bool {
        let Ok(n) = u32::try_from(n) else {
            return false;
        };
        let Ok(more) = Arc::clone(budget).try_acquire_many_owned(n) else {
            return false;
        };
        match self.0.as_mut() {
            Some(held) => held.merge(more),
            None => self.0 = Some(more),
        }
        true
    }
}

/// Collect `body` into one buffer, refusing more than `max` bytes — checked
/// per frame, so an oversized chunked body is rejected without buffering it —
/// and more than `budget` allows across this route's concurrent requests.
/// The returned hold keeps the bytes reserved until it is dropped.
async fn read_limited(
    body: axum::body::Body,
    max: usize,
    budget: &Arc<Semaphore>,
) -> Result<(Bytes, BudgetHold), ReadBodyError> {
    let mut body = std::pin::pin!(body);
    let mut hold = BudgetHold(None);
    let mut buf: Vec<u8> = Vec::new();
    while let Some(frame) =
        std::future::poll_fn(|cx| http_body::Body::poll_frame(body.as_mut(), cx)).await
    {
        let frame = frame.map_err(|_| ReadBodyError::Broken)?;
        if let Ok(data) = frame.into_data() {
            if buf.len().saturating_add(data.len()) > max {
                return Err(ReadBodyError::TooLarge);
            }
            if !hold.reserve(budget, data.len()) {
                return Err(ReadBodyError::OverBudget);
            }
            buf.extend_from_slice(&data);
        }
    }
    Ok((Bytes::from(buf), hold))
}

#[derive(Clone)]
struct HandlerState {
    tx: mpsc::Sender<Job>,
    publishing_point: String,
    grouping: Arc<Mutex<Option<ManifestGrouping>>>,
    auth: Option<Arc<broadcast_auth::Verifier>>,
    max_object_bytes: usize,
    max_buffered_object_bytes: usize,
    buffered_budget: Arc<Semaphore>,
    body_timeout: Option<tokio::time::Duration>,
}

pub async fn run_dashif_ingest(
    route: &DashifIngestRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<RouteHandle>,
) -> MultimuxError {
    let listener = match route.take_listener() {
        Ok(l) => l,
        Err(e) => {
            return MultimuxError::Connect {
                reason: format!("bind: {e}"),
            };
        }
    };

    let (tx, rx) = mpsc::channel::<Job>(CHANNEL_CAPACITY);
    let grouping = Arc::new(Mutex::new(None::<ManifestGrouping>));

    let cancel = CancellationToken::new();
    let route_handle_pump = Arc::clone(route_handle);
    let grouping_pump = Arc::clone(&grouping);

    let ingest_config = route.ingest;
    let pump_handle = tokio::spawn(async move {
        let mut driver = IngestDriver::new(
            TolerantSession::new(ingest_config),
            media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
        );
        let mut progress = DriverProgress::new();
        let mut rx = rx;
        // In-progress segment handles keyed by representation id (P3b).
        let mut handles: std::collections::HashMap<String, dash_runtime::ingest::SegmentHandle> =
            std::collections::HashMap::new();
        while let Some(job) = rx.recv().await {
            match job {
                Job::Whole { kind, bytes, resp } => {
                    let obj = IngestObject {
                        kind,
                        bytes: &bytes,
                    };
                    driver.feed(obj, Timestamp::ZERO);
                    if let Some(g) = driver.session().inner.grouping() {
                        *grouping_pump.lock() = Some(g);
                    }
                    let result = match driver.session_mut().take_last_error() {
                        Some(e) => Err(e),
                        None if driver.health().is_running() => Ok(()),
                        None => Err(SessionError::Ingest(
                            dash_runtime::ingest::IngestError::Internal,
                        )),
                    };
                    crate::source::advance_route(&driver, &route_handle_pump, &mut progress).await;
                    let _ = resp.send(result);
                }
                Job::Begin { kind, resp } => {
                    let rep_id = match &kind {
                        dash_runtime::ingest::ObjectKind::Segment {
                            representation_id, ..
                        } => representation_id.clone(),
                        _ => {
                            let _ = resp.send(Err(SessionError::Ingest(
                                dash_runtime::ingest::IngestError::BadRequest,
                            )));
                            continue;
                        }
                    };
                    if handles.contains_key(&rep_id) {
                        let _ = resp.send(Err(SessionError::Ingest(
                            dash_runtime::ingest::IngestError::SegmentInProgress,
                        )));
                        continue;
                    }
                    let (number, time) = match &kind {
                        dash_runtime::ingest::ObjectKind::Segment { number, time, .. } => {
                            (*number, *time)
                        }
                        _ => (None, None),
                    };
                    let res = driver
                        .session_mut()
                        .inner
                        .begin_segment(&rep_id, number, time);
                    match res {
                        Ok(h) => {
                            handles.insert(rep_id, h);
                            if let Some(g) = driver.session().inner.grouping() {
                                *grouping_pump.lock() = Some(g);
                            }
                            // Drain events emitted by begin (none) / keep parity.
                            driver.feed(
                                IngestObject {
                                    kind: dash_runtime::ingest::ObjectKind::Probe,
                                    bytes: &[],
                                },
                                Timestamp::ZERO,
                            );
                            crate::source::advance_route(
                                &driver,
                                &route_handle_pump,
                                &mut progress,
                            )
                            .await;
                            let _ = resp.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = resp.send(Err(e));
                        }
                    }
                }
                Job::Chunk {
                    rep_id,
                    bytes,
                    resp,
                } => {
                    let h = match handles.get_mut(&rep_id) {
                        Some(h) => h,
                        None => {
                            let _ = resp.send(Err(SessionError::Ingest(
                                dash_runtime::ingest::IngestError::Internal,
                            )));
                            continue;
                        }
                    };
                    let res = driver.session_mut().inner.push_segment_bytes(h, &bytes);
                    if let Some(g) = driver.session().inner.grouping() {
                        *grouping_pump.lock() = Some(g);
                    }
                    driver.feed(
                        IngestObject {
                            kind: dash_runtime::ingest::ObjectKind::Probe,
                            bytes: &[],
                        },
                        Timestamp::ZERO,
                    );
                    crate::source::advance_route(&driver, &route_handle_pump, &mut progress).await;
                    let _ = resp.send(res);
                }
                Job::Finish { rep_id, resp } => {
                    let h = handles.remove(&rep_id);
                    let res = match h {
                        Some(h) => driver.session_mut().inner.finish_segment(h),
                        None => Err(SessionError::Ingest(
                            dash_runtime::ingest::IngestError::Internal,
                        )),
                    };
                    if let Some(g) = driver.session().inner.grouping() {
                        *grouping_pump.lock() = Some(g);
                    }
                    driver.feed(
                        IngestObject {
                            kind: dash_runtime::ingest::ObjectKind::Probe,
                            bytes: &[],
                        },
                        Timestamp::ZERO,
                    );
                    crate::source::advance_route(&driver, &route_handle_pump, &mut progress).await;
                    let _ = resp.send(res);
                }
                Job::Abort { rep_id } => {
                    if let Some(h) = handles.remove(&rep_id) {
                        driver.session_mut().inner.abort_segment(h);
                    }
                }
            }
        }
    });

    let state = HandlerState {
        tx: tx.clone(),
        publishing_point: route.publishing_point.clone(),
        grouping: Arc::clone(&grouping),
        auth: route.auth.clone(),
        max_object_bytes: route.ingest.max_object_bytes,
        max_buffered_object_bytes: route.ingest.max_buffered_object_bytes,
        buffered_budget: Arc::new(Semaphore::new(route.ingest.buffered_budget_bytes)),
        body_timeout: route.body_timeout,
    };

    let app = Router::new().fallback(dashif_handler).with_state(state);
    let app = app.layer(tower_http::decompression::RequestDecompressionLayer::new());

    let serve_res = origin::serve_hyper_util(listener, app, cancel.clone()).await;
    pump_handle.abort();
    let _ = serve_res;
    MultimuxError::Connect {
        reason: "dashif ingest listener stopped".into(),
    }
}

async fn dashif_handler(
    State(state): State<HandlerState>,
    req: Request<axum::body::Body>,
) -> impl IntoResponse {
    // Authenticate FIRST — before method/path/body handling. No unauthenticated
    // object may be read, classified or fed.
    if let Some(verifier) = &state.auth
        && let Some(resp) = crate::origin::check_output_auth(verifier, &req)
    {
        return resp;
    }

    let method = req.method().clone();
    if method != Method::POST && method != Method::PUT {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [("allow", "POST, PUT")],
            Bytes::new(),
        )
            .into_response();
    }
    let ingest_method = if method == Method::POST {
        dash_runtime::ingest::IngestMethod::Post
    } else {
        dash_runtime::ingest::IngestMethod::Put
    };

    let path = req.uri().path().to_string();
    let pp = state.publishing_point.clone();
    // prefix check
    let relative = if pp == "/" {
        if !path.starts_with('/') {
            return StatusCode::NOT_FOUND.into_response();
        }
        path.clone()
    } else {
        let base = pp.trim_end_matches('/');
        if path != pp && !path.starts_with(&format!("{}/", base)) {
            return StatusCode::NOT_FOUND.into_response();
        }
        let rel = &path[pp.len()..];
        if rel.is_empty() {
            "/".to_string()
        } else {
            rel.to_string()
        }
    };

    let content_length = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    if content_length.is_some_and(|n| n > state.max_object_bytes as u64) {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let chunked = content_length.is_none()
        && req
            .headers()
            .get(axum::http::header::TRANSFER_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_ascii_lowercase().contains("chunked"))
            .unwrap_or(false);

    let relative = if relative == "/" {
        relative
    } else {
        relative.trim_start_matches('/').to_string()
    };

    // Early classification (body assumed non-empty) to decide whether this is
    // a streamed Segment object (P3b). Non-segment objects keep the buffered path.
    let grouping_early = state.grouping.lock().clone();
    let mut early_meta = RequestMeta::new(ingest_method, &relative);
    early_meta.content_length = content_length;
    early_meta.chunked = chunked;
    let early_kind = classify(&early_meta, grouping_early.as_ref()).ok();
    let segment = match early_kind {
        Some(kind @ dash_runtime::ingest::ObjectKind::Segment { .. }) => Some(kind),
        _ => None,
    };

    if let Some(kind) = segment {
        let rep_id = match &kind {
            dash_runtime::ingest::ObjectKind::Segment {
                representation_id, ..
            } => representation_id.clone(),
            // Unreachable by construction (matched above); a typed refusal
            // rather than a panic if that ever changes.
            _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };

        let timeout = state.body_timeout;
        let tx = state.tx.clone();
        // Begin (wait for its Result).
        let (btx, brx) = oneshot::channel();
        let begin_job = Job::Begin {
            kind: kind.clone(),
            resp: btx,
        };
        if tx.try_send(begin_job).is_err() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", "1")],
                Bytes::new(),
            )
                .into_response();
        }
        match wait_reply(brx, timeout).await {
            Reply::Got(Ok(())) => {}
            Reply::Got(Err(SessionError::Ingest(e))) => return e.status().into_response(),
            Reply::Got(Err(SessionError::Demux(_))) => {
                return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
            }
            Reply::Got(Err(_)) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Reply::Timeout => return StatusCode::REQUEST_TIMEOUT.into_response(),
            Reply::Closed => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
        let mut guard = SegmentGuard::new(tx.clone(), rep_id.clone());

        // Stream the body frame by frame.
        let max = state.max_object_bytes;
        let mut body = std::pin::pin!(req.into_body());
        let start = tokio::time::Instant::now();
        let mut total: usize = 0;
        let mut read_result: Result<(), StatusCode> = Ok(());
        loop {
            let frame_fut =
                std::future::poll_fn(|cx| http_body::Body::poll_frame(body.as_mut(), cx));
            let frame_opt = if let Some(d) = timeout {
                let remaining = d.saturating_sub(start.elapsed());
                match tokio::time::timeout(remaining, frame_fut).await {
                    Ok(v) => v,
                    Err(_) => {
                        read_result = Err(StatusCode::REQUEST_TIMEOUT);
                        break;
                    }
                }
            } else {
                frame_fut.await
            };
            let Some(frame) = frame_opt else {
                break;
            };
            let frame = match frame {
                Ok(f) => f,
                Err(_) => {
                    read_result = Err(StatusCode::BAD_REQUEST);
                    break;
                }
            };
            if let Ok(data) = frame.into_data() {
                if total.saturating_add(data.len()) > max {
                    read_result = Err(StatusCode::PAYLOAD_TOO_LARGE);
                    break;
                }
                total += data.len();
                let (ctx, crx) = oneshot::channel();
                let chunk_job = Job::Chunk {
                    rep_id: rep_id.clone(),
                    bytes: data,
                    resp: ctx,
                };
                if tx.try_send(chunk_job).is_err() {
                    read_result = Err(StatusCode::SERVICE_UNAVAILABLE);
                    break;
                }
                let remaining = timeout.map(|d| d.saturating_sub(start.elapsed()));
                match wait_reply(crx, remaining).await {
                    Reply::Got(Ok(())) => {}
                    Reply::Got(Err(SessionError::Ingest(e))) => {
                        read_result = Err(e.status());
                        break;
                    }
                    Reply::Got(Err(SessionError::Demux(_))) => {
                        read_result = Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
                        break;
                    }
                    Reply::Got(Err(_)) => {
                        read_result = Err(StatusCode::INTERNAL_SERVER_ERROR);
                        break;
                    }
                    Reply::Timeout => {
                        read_result = Err(StatusCode::REQUEST_TIMEOUT);
                        break;
                    }
                    Reply::Closed => {
                        read_result = Err(StatusCode::INTERNAL_SERVER_ERROR);
                        break;
                    }
                }
            }
        }

        if let Err(status) = read_result {
            return status.into_response();
        }

        // Finish.
        let (ftx, frx) = oneshot::channel();
        let finish_job = Job::Finish {
            rep_id: rep_id.clone(),
            resp: ftx,
        };
        if tx.try_send(finish_job).is_err() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", "1")],
                Bytes::new(),
            )
                .into_response();
        }
        let remaining = timeout.map(|d| d.saturating_sub(start.elapsed()));
        let reply = wait_reply(frx, remaining).await;
        if let Reply::Got(_) = &reply {
            guard.disarm();
        }
        match reply {
            Reply::Got(Ok(())) => StatusCode::OK.into_response(),
            Reply::Got(Err(SessionError::Ingest(e))) => e.status().into_response(),
            Reply::Got(Err(SessionError::Demux(_))) => {
                StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response()
            }
            Reply::Got(Err(_)) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Reply::Timeout => StatusCode::REQUEST_TIMEOUT.into_response(),
            Reply::Closed => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    } else {
        // Buffered path (non-segment objects).
        let body = {
            let max = state.max_object_bytes.min(state.max_buffered_object_bytes);
            let timeout = state.body_timeout;
            let fut = read_limited(req.into_body(), max, &state.buffered_budget);
            let res = if let Some(d) = timeout {
                match tokio::time::timeout(d, fut).await {
                    Ok(r) => r,
                    Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
                }
            } else {
                fut.await
            };
            match res {
                Ok(held) => held,
                Err(ReadBodyError::TooLarge) => {
                    return StatusCode::PAYLOAD_TOO_LARGE.into_response();
                }
                Err(ReadBodyError::OverBudget) => {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [("retry-after", "1")],
                        Bytes::new(),
                    )
                        .into_response();
                }
                Err(ReadBodyError::Broken) => return StatusCode::BAD_REQUEST.into_response(),
            }
        };
        let (body, _budget_hold) = body;
        let body_empty = body.is_empty();

        let mut meta = RequestMeta::new(ingest_method, &relative);
        meta.content_length = content_length;
        meta.chunked = chunked;
        meta.body_empty = body_empty;

        let grouping = state.grouping.lock().clone();
        let kind = match classify(&meta, grouping.as_ref()) {
            Ok(k) => k,
            Err(e) => return e.status().into_response(),
        };

        let (tx, rx) = oneshot::channel();
        let job = Job::Whole {
            kind,
            bytes: body.clone(),
            resp: tx,
        };
        if state.tx.try_send(job).is_err() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", "1")],
                Bytes::new(),
            )
                .into_response();
        }
        match rx.await {
            Ok(Ok(())) => StatusCode::OK.into_response(),
            Ok(Err(SessionError::Ingest(e))) => e.status().into_response(),
            Ok(Err(SessionError::Demux(_))) => StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
            Ok(Err(_)) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ample() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(1024))
    }

    /// The tolerant wrapper forwards the inner demand exactly, so a driver
    /// asking how much input the session wants gets the real answer.
    #[test]
    fn a_tolerant_session_forwards_the_inner_demand() {
        let session = TolerantSession::new(DashifIngestConfig::default());
        assert_eq!(session.demand(), Demand::new(65_536));
    }

    /// The inner DASH-IF session has no timer of its own; the wrapper must
    /// not invent one.
    #[test]
    fn a_tolerant_session_has_no_deadline() {
        let session = TolerantSession::new(DashifIngestConfig::default());
        assert!(session.next_deadline().is_none());
    }

    fn body_of(frames: Vec<&'static [u8]>) -> axum::body::Body {
        axum::body::Body::from_stream(futures_util::stream::iter(
            frames
                .into_iter()
                .map(|f| Ok::<Bytes, std::io::Error>(Bytes::from_static(f))),
        ))
    }

    /// `read_limited` caps the body per frame: exactly the limit passes, one
    /// byte more is `TooLarge` — including when the body is chunked into many
    /// small frames (no `Content-Length` to pre-check).
    #[tokio::test]
    async fn read_limited_enforces_the_cap_across_frames() {
        let ok = read_limited(body_of(vec![b"abc", b"de"]), 5, &ample()).await;
        assert!(matches!(ok, Ok((ref b, _)) if b.as_ref() == b"abcde"));
        let too_big = read_limited(body_of(vec![b"abc", b"def"]), 5, &ample()).await;
        assert!(matches!(too_big, Err(ReadBodyError::TooLarge)));
        let empty = read_limited(body_of(vec![]), 5, &ample()).await;
        assert!(matches!(empty, Ok((ref b, _)) if b.is_empty()));
    }

    #[tokio::test]
    async fn read_limited_reports_a_broken_body() {
        let stream = futures_util::stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"ab")),
            Err(std::io::Error::other("reset")),
        ]);
        let r = read_limited(axum::body::Body::from_stream(stream), 100, &ample()).await;
        assert!(matches!(r, Err(ReadBodyError::Broken)));
    }

    /// Concurrent requests share one route's budget: a body that does not fit
    /// beside the bytes another request holds is refused, and the bytes return
    /// when that request is done.
    #[tokio::test]
    async fn the_route_budget_is_shared_across_requests() {
        let budget = Arc::new(Semaphore::new(8));
        let Ok(first) = read_limited(body_of(vec![b"abcde"]), 100, &budget).await else {
            panic!("5 of 8 bytes fit");
        };
        assert_eq!(budget.available_permits(), 3);
        let second = read_limited(body_of(vec![b"abcd"]), 100, &budget).await;
        assert!(matches!(second, Err(ReadBodyError::OverBudget)));
        assert_eq!(
            budget.available_permits(),
            3,
            "a refused request holds nothing"
        );
        drop(first);
        assert_eq!(budget.available_permits(), 8);
        let third = read_limited(body_of(vec![b"abcd"]), 100, &budget).await;
        assert!(third.is_ok());
    }

    fn grouping() -> dash_runtime::ingest::ManifestGrouping {
        dash_runtime::ingest::ManifestGrouping {
            init_template: "$RepresentationID$/init.mp4".to_string(),
            media_template: "$RepresentationID$/seg-$Number$.m4s".to_string(),
            addressing: dash_runtime::ingest::Addressing::Number,
            switching_sets: vec![vec!["v1".to_string()]],
            availability_start_time: None,
            period_start: None,
        }
    }

    fn handler_state(
        tx: mpsc::Sender<Job>,
        body_timeout: Option<tokio::time::Duration>,
    ) -> HandlerState {
        HandlerState {
            tx,
            publishing_point: "/".to_string(),
            grouping: Arc::new(Mutex::new(Some(grouping()))),
            auth: None,
            max_object_bytes: 1 << 20,
            max_buffered_object_bytes: 1 << 20,
            buffered_budget: ample(),
            body_timeout,
        }
    }

    fn segment_request() -> Request<axum::body::Body> {
        Request::builder()
            .method(Method::PUT)
            .uri("/v1/seg-1.m4s")
            .header(axum::http::header::CONTENT_LENGTH, "4")
            .body(body_of(vec![b"abcd"]))
            .expect("request builds")
    }

    /// A publisher disconnect drops the handler future while its segment is
    /// still open; the pump must see `Job::Abort`, or every later segment for
    /// that representation is refused with `SegmentInProgress`.
    #[tokio::test]
    async fn a_dropped_handler_aborts_the_open_segment() {
        let (tx, mut rx) = mpsc::channel::<Job>(CHANNEL_CAPACITY);
        let state = handler_state(tx, None);
        let task = tokio::spawn(async move {
            dashif_handler(State(state), segment_request())
                .await
                .into_response()
        });

        let job = rx.recv().await.expect("begin job");
        let Job::Begin { resp, .. } = job else {
            panic!("expected Begin");
        };
        let _ = resp.send(Ok(()));
        let job = rx.recv().await.expect("chunk job");
        let Job::Chunk { resp, .. } = job else {
            panic!("expected Chunk");
        };
        task.abort();
        let _ = task.await;
        drop(resp);

        let Job::Abort { rep_id } =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("abort observed")
                .expect("channel open")
        else {
            panic!("expected Abort");
        };
        assert_eq!(rep_id, "v1");
    }

    /// An unanswered pump reply must not park the handler forever: the body
    /// deadline turns it into 408 and the open segment is aborted.
    #[tokio::test]
    async fn an_unanswered_pump_reply_times_out_and_aborts() {
        let (tx, mut rx) = mpsc::channel::<Job>(CHANNEL_CAPACITY);
        let state = handler_state(tx, Some(tokio::time::Duration::from_millis(50)));
        let task = tokio::spawn(async move {
            dashif_handler(State(state), segment_request())
                .await
                .into_response()
        });

        let job = rx.recv().await.expect("begin job");
        let Job::Begin { resp, .. } = job else {
            panic!("expected Begin");
        };
        let _ = resp.send(Ok(()));
        let job = rx.recv().await.expect("chunk job");
        let Job::Chunk { resp, .. } = job else {
            panic!("expected Chunk");
        };

        let response = tokio::time::timeout(tokio::time::Duration::from_secs(5), task)
            .await
            .expect("handler returns")
            .expect("handler joins");
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        drop(resp);

        let Job::Abort { rep_id } =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("abort observed")
                .expect("channel open")
        else {
            panic!("expected Abort");
        };
        assert_eq!(rep_id, "v1");
    }
}
