//! Admin config + status HTTP routes for acap-multimux.
//!
//! `GET /admin/config` and `POST /admin/config` read/update the app's
//! [`Config`] through a pluggable [`ConfigStore`]: [`DefaultStore`] (host
//! builds, and the device fallback) always round-trips `Config::default()`;
//! `#[cfg(feature = "device")]` `AxParameterStore` persists it via the ACAP
//! `axparameter` parameter store. `GET /admin/status` reports the running
//! pipeline's [`Status`] through a shared [`StatusHandle`] the pipeline
//! updates as it runs.
//!
//! The routes and `Config` (de)serialization are plain std + serde + axum,
//! so this whole module — including its tests — builds and runs on the host;
//! only `AxParameterStore` is device-gated.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use serde::{Deserialize, Serialize};

/// Default VDO channel index (single-sensor cameras use channel 0).
const DEFAULT_CHANNEL: u32 = 0;
/// Default capture width, pixels.
const DEFAULT_WIDTH: u32 = 1920;
/// Default capture height, pixels.
const DEFAULT_HEIGHT: u32 = 1080;
/// Default capture frame rate, fps.
const DEFAULT_FRAMERATE: u32 = 30;
/// Default codec: "h264" or "h265".
const DEFAULT_CODEC: &str = "h264";
/// Default LL-HLS target segment duration, seconds.
const DEFAULT_TARGET_DURATION_SECS: f64 = 4.0;
/// Default LL-HLS target part duration, milliseconds.
const DEFAULT_PART_TARGET_MS: u32 = 500;
/// Default number of segments kept in the LL-HLS media playlist window.
const DEFAULT_WINDOW_SEGMENTS: usize = 8;
/// Default HTTP bind port (the manifest's `reverseProxy` targets this).
const DEFAULT_PORT: u16 = 2999;

/// The app's persisted configuration: VDO capture parameters, the codec,
/// LL-HLS tuning, and the HTTP bind port. Round-tripped through a
/// [`ConfigStore`]; changes via `POST /admin/config` take effect on the next
/// app restart (the running pipeline is not reconfigured live).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// VDO channel index to capture from.
    pub channel: u32,
    /// Capture width, pixels.
    pub width: u32,
    /// Capture height, pixels.
    pub height: u32,
    /// Capture frame rate, fps.
    pub framerate: u32,
    /// Encoded video codec: `"h264"` or `"h265"`.
    pub codec: String,
    /// LL-HLS target segment duration, seconds.
    pub target_duration_secs: f64,
    /// LL-HLS target part duration, milliseconds.
    pub part_target_ms: u32,
    /// Number of segments kept in the LL-HLS media playlist window.
    pub window_segments: usize,
    /// HTTP bind port.
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            channel: DEFAULT_CHANNEL,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            framerate: DEFAULT_FRAMERATE,
            codec: DEFAULT_CODEC.to_string(),
            target_duration_secs: DEFAULT_TARGET_DURATION_SECS,
            part_target_ms: DEFAULT_PART_TARGET_MS,
            window_segments: DEFAULT_WINDOW_SEGMENTS,
            port: DEFAULT_PORT,
        }
    }
}

impl Config {
    /// Reject configs the pipeline/origin could not run with: an unknown
    /// codec, or a non-positive timing/window/port value.
    fn validate(&self) -> Result<(), String> {
        if self.codec != "h264" && self.codec != "h265" {
            return Err(format!(
                "codec must be \"h264\" or \"h265\", got {:?}",
                self.codec
            ));
        }
        if self.target_duration_secs <= 0.0 {
            return Err("target_duration_secs must be positive".to_string());
        }
        if self.part_target_ms == 0 {
            return Err("part_target_ms must be positive".to_string());
        }
        if self.window_segments == 0 {
            return Err("window_segments must be positive".to_string());
        }
        if self.port == 0 {
            return Err("port must be positive".to_string());
        }
        Ok(())
    }
}

/// Loads and persists [`Config`]. Host builds use [`DefaultStore`]; device
/// builds use `#[cfg(feature = "device")]` `AxParameterStore`.
pub trait ConfigStore: Send + Sync + 'static {
    /// Load the current config, falling back to [`Config::default`] if none
    /// is stored yet or the stored value can't be parsed.
    fn load(&self) -> Config;
    /// Persist `c` as the new config.
    fn store(&self, c: &Config) -> crate::Result<()>;
}

/// Host + fallback [`ConfigStore`]: `load` always returns
/// [`Config::default`], `store` is a no-op. Used on host builds (including
/// these tests) and as a device fallback before axparameter is wired up.
pub struct DefaultStore;

impl ConfigStore for DefaultStore {
    fn load(&self) -> Config {
        Config::default()
    }

    fn store(&self, _c: &Config) -> crate::Result<()> {
        Ok(())
    }
}

/// ACAP `axparameter`-backed [`ConfigStore`]: round-trips the whole [`Config`]
/// as one JSON string parameter on the app's `axparameter::Parameter` handle.
/// Device builds only — `axparameter` is an optional, `device`-feature-gated
/// dependency (see `Cargo.toml`).
#[cfg(feature = "device")]
pub struct AxParameterStore {
    inner: axparameter::parameter::Parameter,
}

#[cfg(feature = "device")]
impl AxParameterStore {
    /// The single axparameter parameter name the whole [`Config`] is
    /// serialized under (as JSON).
    const PARAM_NAME: &'static str = "Config";

    /// Open (or create) the `acap-multimux` axparameter handle.
    pub fn new() -> crate::Result<Self> {
        let inner = axparameter::parameter::Parameter::new("acap-multimux")
            .map_err(|e| crate::AcapError::Config(format!("axparameter open: {e}")))?;
        Ok(AxParameterStore { inner })
    }
}

#[cfg(feature = "device")]
impl ConfigStore for AxParameterStore {
    fn load(&self) -> Config {
        self.inner
            .get::<String>(Self::PARAM_NAME)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn store(&self, c: &Config) -> crate::Result<()> {
        let s = serde_json::to_string(c)
            .map_err(|e| crate::AcapError::Config(format!("config serialize: {e}")))?;
        self.inner
            .set(Self::PARAM_NAME, s, true)
            .map_err(|e| crate::AcapError::Config(format!("axparameter set: {e}")))
    }
}

/// Live pipeline status, updated by the running pipeline and read by
/// `GET /admin/status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    /// Whether the capture/mux pipeline is currently running.
    pub running: bool,
    /// The LL-HLS media sequence number currently being written.
    pub current_segment: u32,
    /// The part index within `current_segment` currently being written.
    pub current_part: u32,
    /// Total frames processed since the pipeline started.
    pub frames: u64,
    /// The most recent pipeline error, if any, as its `Display` text.
    pub last_error: Option<String>,
}

/// Shared, cloneable handle to a [`Status`], read by the admin routes and
/// updated by the running pipeline.
#[derive(Clone)]
pub struct StatusHandle(Arc<Mutex<Status>>);

impl StatusHandle {
    /// A fresh handle around [`Status::default`] (not running, no frames).
    pub fn new() -> Self {
        StatusHandle(Arc::new(Mutex::new(Status::default())))
    }

    /// The current status, cloned out from behind the lock.
    pub fn snapshot(&self) -> Status {
        self.0.lock().expect("status mutex poisoned").clone()
    }

    /// Mark the pipeline as running or stopped.
    pub fn set_running(&self, running: bool) {
        self.0.lock().expect("status mutex poisoned").running = running;
    }

    /// Update the current segment/part position.
    pub fn set_position(&self, current_segment: u32, current_part: u32) {
        let mut status = self.0.lock().expect("status mutex poisoned");
        status.current_segment = current_segment;
        status.current_part = current_part;
    }

    /// Add `n` to the processed-frame counter.
    pub fn add_frames(&self, n: u64) {
        self.0.lock().expect("status mutex poisoned").frames += n;
    }

    /// Record (or clear) the most recent pipeline error.
    pub fn set_last_error(&self, err: Option<String>) {
        self.0.lock().expect("status mutex poisoned").last_error = err;
    }
}

impl Default for StatusHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Admin router state: the [`ConfigStore`] plus the shared [`StatusHandle`].
struct AdminState<S: ConfigStore> {
    store: Arc<S>,
    status: StatusHandle,
}

// Manual `Clone` (rather than `#[derive]`) so cloning `AdminState<S>` never
// requires `S: Clone` — only `Arc<S>` and `StatusHandle` need to be cloned.
impl<S: ConfigStore> Clone for AdminState<S> {
    fn clone(&self) -> Self {
        AdminState {
            store: Arc::clone(&self.store),
            status: self.status.clone(),
        }
    }
}

/// Build the admin router: `GET`/`POST /admin/config` against `store`, and
/// `GET /admin/status` reading `status`. Fully applies its state, so the
/// returned [`Router`] merges directly with `multimux::origin::router`'s.
pub fn admin_router<S: ConfigStore>(store: Arc<S>, status: StatusHandle) -> Router {
    let state = AdminState { store, status };
    Router::new()
        .route("/admin/config", get(get_config).post(post_config))
        .route("/admin/status", get(get_status))
        .with_state(state)
}

async fn get_config<S: ConfigStore>(State(state): State<AdminState<S>>) -> Json<Config> {
    Json(state.store.load())
}

async fn post_config<S: ConfigStore>(
    State(state): State<AdminState<S>>,
    Json(cfg): Json<Config>,
) -> impl IntoResponse {
    if let Err(reason) = cfg.validate() {
        return (StatusCode::BAD_REQUEST, reason).into_response();
    }
    match state.store.store(&cfg) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "note": "takes effect on restart",
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_status<S: ConfigStore>(State(state): State<AdminState<S>>) -> Json<Status> {
    Json(state.status.snapshot())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    use super::*;

    fn router() -> Router {
        admin_router(Arc::new(DefaultStore), StatusHandle::new())
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("parse json body")
    }

    #[tokio::test]
    async fn get_config_returns_defaults() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/admin/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cfg: Config = serde_json::from_value(body_json(response).await).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[tokio::test]
    async fn post_config_valid_returns_200() {
        let cfg = Config {
            codec: "h265".to_string(),
            ..Config::default()
        };

        let response = router()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/admin/config")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&cfg).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_config_invalid_codec_returns_400() {
        let cfg = Config {
            codec: "vp9".to_string(),
            ..Config::default()
        };

        let response = router()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/admin/config")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&cfg).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_config_invalid_window_returns_400() {
        let cfg = Config {
            window_segments: 0,
            ..Config::default()
        };

        let response = router()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/admin/config")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&cfg).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_status_returns_expected_fields() {
        let status = StatusHandle::new();
        status.set_running(true);
        status.set_position(3, 2);
        status.add_frames(42);
        status.set_last_error(Some("boom".to_string()));

        let response = admin_router(Arc::new(DefaultStore), status)
            .oneshot(
                Request::builder()
                    .uri("/admin/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["running"], serde_json::json!(true));
        assert_eq!(value["current_segment"], serde_json::json!(3));
        assert_eq!(value["current_part"], serde_json::json!(2));
        assert_eq!(value["frames"], serde_json::json!(42));
        assert_eq!(value["last_error"], serde_json::json!("boom"));
    }
}
