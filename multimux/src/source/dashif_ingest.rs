//! DASH-IF Live Media Ingest (Interface-1) HTTP listener (increment P2b-1).

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use bytes::Bytes;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::TrunkConfig;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::dashif_ingest::session::SessionError;
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
        if let Some(l) = self.prebound.lock().unwrap().take() {
            return Ok(l);
        }
        let std_listener = std::net::TcpListener::bind(&self.listen)?;
        std_listener.set_nonblocking(true)?;
        tokio::net::TcpListener::from_std(std_listener)
    }
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
    let pump_cancel = cancel.clone();
    let route_handle_pump = Arc::clone(route_handle);
    let grouping_pump = Arc::clone(&grouping);
    let publishing_point_pump = route.publishing_point.clone();
    let _ = publishing_point_pump;

    let pump_handle = tokio::spawn(async move {
        let mut driver = IngestDriver::new(
            crate::dashif_ingest::DashifIngestSession::new(),
            trunk_config,
            handshake,
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut progress = DriverProgress::new();
        let mut rx = rx;
        while let Some(job) = rx.recv().await {
            let obj = crate::dashif_ingest::IngestObject {
                kind: job.kind,
                bytes: &job.bytes,
            };
            driver.feed(obj, broadcast_common::Timestamp::ZERO);
            // Update shared grouping on successful manifest.
            if let Some(g) = driver.session().grouping() {
                *grouping_pump.lock().unwrap() = Some(g);
            }
            let result = if driver.health().is_running() {
                Ok(())
            } else {
                match driver.health() {
                    media_plane::ingress::HealthState::Failed(e) => Err(e.clone()),
                    _ => Err(SessionError::Ingest(
                        crate::dashif_ingest::IngestError::Internal,
                    )),
                }
            };
            let _ = job.kind;
            crate::source::advance_route(&driver, &route_handle_pump, &mut progress).await;
            let _ = job.resp.send(result);
        }
        let _ = pump_cancel;
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
    let chunked = content_length.is_none()
        && req
            .headers()
            .get(axum::http::header::TRANSFER_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_ascii_lowercase().contains("chunked"))
            .unwrap_or(false);

    let body = match axum::body::to_bytes(req.into_body(), MAX_OBJECT_BYTES).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
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

    let grouping = state.grouping.lock().unwrap().clone();
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
