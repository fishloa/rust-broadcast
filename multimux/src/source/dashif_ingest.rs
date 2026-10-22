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
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::dashif_ingest::session::SessionError;
use crate::dashif_ingest::{DashifIngestSession, IngestObject};
use crate::dashif_ingest::{ManifestGrouping, RequestMeta, classify};
use crate::error::MultimuxError;
use crate::origin;
use crate::route::RouteHandle;
use crate::source::DriverProgress;

use media_plane::ingress::IngestDriver;

/// Hard cap on a single ingested object's body (P2b-1; P2b-2 makes it
/// configurable).
pub const MAX_OBJECT_BYTES: usize = 64 * 1024 * 1024;

/// Bounded channel between HTTP handlers and the per-publishing-point pump.
const CHANNEL_CAPACITY: usize = 64;

/// A job handed from an HTTP handler to the pump task.
struct Job {
    kind: crate::dashif_ingest::ObjectKind,
    bytes: Bytes,
    resp: oneshot::Sender<Result<(), SessionError>>,
}

/// One DASH-IF ingest route.
pub struct DashifIngestRoute {
    name: String,
    listen: String,
    prebound: Mutex<Option<TcpListener>>,
    publishing_point: String,
    timeouts: crate::source::IngestTimeouts,
}

impl std::fmt::Debug for DashifIngestRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashifIngestRoute")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("publishing_point", &self.publishing_point)
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
            timeouts: crate::source::IngestTimeouts::default(),
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
            timeouts: crate::source::IngestTimeouts::default(),
        }
    }

    #[must_use]
    pub fn with_timeouts(mut self, timeouts: crate::source::IngestTimeouts) -> Self {
        self.timeouts = timeouts;
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
    fn new() -> Self {
        TolerantSession {
            inner: DashifIngestSession::new(),
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
    /// The client's body failed or was cut short.
    Broken,
}

/// Collect `body` into one buffer, refusing more than `max` bytes — checked
/// per frame, so an oversized chunked body is rejected without buffering it.
async fn read_limited(body: axum::body::Body, max: usize) -> Result<Bytes, ReadBodyError> {
    let mut body = std::pin::pin!(body);
    let mut buf: Vec<u8> = Vec::new();
    while let Some(frame) =
        std::future::poll_fn(|cx| http_body::Body::poll_frame(body.as_mut(), cx)).await
    {
        let frame = frame.map_err(|_| ReadBodyError::Broken)?;
        if let Ok(data) = frame.into_data() {
            if buf.len().saturating_add(data.len()) > max {
                return Err(ReadBodyError::TooLarge);
            }
            buf.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(buf))
}

#[derive(Clone)]
struct HandlerState {
    tx: mpsc::Sender<Job>,
    publishing_point: String,
    grouping: Arc<Mutex<Option<ManifestGrouping>>>,
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

    let pump_handle = tokio::spawn(async move {
        let mut driver = IngestDriver::new(
            TolerantSession::new(),
            trunk_config,
            handshake,
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut progress = DriverProgress::new();
        let mut rx = rx;
        while let Some(job) = rx.recv().await {
            let obj = IngestObject {
                kind: job.kind,
                bytes: &job.bytes,
            };
            driver.feed(obj, Timestamp::ZERO);
            // Publish the manifest grouping the handlers classify against.
            if let Some(g) = driver.session().inner.grouping() {
                *grouping_pump.lock() = Some(g);
            }
            let result = match driver.session_mut().take_last_error() {
                Some(e) => Err(e),
                None if driver.health().is_running() => Ok(()),
                None => Err(SessionError::Ingest(
                    crate::dashif_ingest::IngestError::Internal,
                )),
            };
            // Hand any events the object produced to the Trunk BEFORE the
            // sender sees its 200: an acknowledged object is a published one.
            crate::source::advance_route(&driver, &route_handle_pump, &mut progress).await;
            let _ = job.resp.send(result);
        }
    });

    let state = HandlerState {
        tx: tx.clone(),
        publishing_point: route.publishing_point.clone(),
        grouping: Arc::clone(&grouping),
    };

    let app = Router::new().fallback(dashif_handler).with_state(state);

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
        crate::dashif_ingest::IngestMethod::Post
    } else {
        crate::dashif_ingest::IngestMethod::Put
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
    if content_length.is_some_and(|n| n > MAX_OBJECT_BYTES as u64) {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let chunked = content_length.is_none()
        && req
            .headers()
            .get(axum::http::header::TRANSFER_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_ascii_lowercase().contains("chunked"))
            .unwrap_or(false);

    let body = match read_limited(req.into_body(), MAX_OBJECT_BYTES).await {
        Ok(b) => b,
        Err(ReadBodyError::TooLarge) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(ReadBodyError::Broken) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let body_empty = body.is_empty();

    let relative = if relative == "/" {
        relative
    } else {
        relative.trim_start_matches('/').to_string()
    };
    let meta = RequestMeta {
        method: ingest_method,
        path: &relative,
        content_length,
        chunked,
        body_empty,
    };

    let grouping = state.grouping.lock().clone();
    let kind = match classify(&meta, grouping.as_ref()) {
        Ok(k) => k,
        Err(e) => return e.status().into_response(),
    };

    let (tx, rx) = oneshot::channel();
    let job = Job {
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
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let ok = read_limited(body_of(vec![b"abc", b"de"]), 5).await;
        assert!(matches!(ok, Ok(ref b) if b.as_ref() == b"abcde"));
        let too_big = read_limited(body_of(vec![b"abc", b"def"]), 5).await;
        assert!(matches!(too_big, Err(ReadBodyError::TooLarge)));
        let empty = read_limited(body_of(vec![]), 5).await;
        assert!(matches!(empty, Ok(ref b) if b.is_empty()));
    }

    #[tokio::test]
    async fn read_limited_reports_a_broken_body() {
        let stream = futures_util::stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"ab")),
            Err(std::io::Error::other("reset")),
        ]);
        let r = read_limited(axum::body::Body::from_stream(stream), 100).await;
        assert!(matches!(r, Err(ReadBodyError::Broken)));
    }
}
