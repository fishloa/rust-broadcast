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

/// Outcome of [`ConfigStore::load`] — distinguishes "nothing has been
/// stored yet" from "the backend itself is broken". Issue #955: the old
/// `load()` discarded the backend's error and returned
/// [`Config::default`] either way, so an axparameter store that had never
/// worked on any camera looked byte-for-byte identical to one nobody had
/// configured yet. A caller that only wants "the config to run with" should
/// use [`LoadOutcome::into_config`]; a caller that also wants to know
/// whether the store is actually broken (to surface e.g. via
/// `/admin/status`'s `last_error`) should check [`LoadOutcome::error`]
/// first.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LoadOutcome {
    /// A previously stored config was loaded and parsed successfully.
    Stored(Config),
    /// Nothing has been stored yet (a fresh install); the caller should use
    /// [`Config::default`].
    Unset,
    /// The backend failed to produce a config — a `get`/`add` failure or a
    /// stored value that didn't parse — carrying the failure reason. This is
    /// NOT equivalent to [`LoadOutcome::Unset`]: it means the store is
    /// broken, not merely empty.
    Broken(String),
}

impl LoadOutcome {
    /// The config to actually run with: the stored value for
    /// [`LoadOutcome::Stored`], or [`Config::default`] for
    /// [`LoadOutcome::Unset`]/[`LoadOutcome::Broken`] — a broken backend
    /// still needs *some* config to boot with, but callers that discard the
    /// distinction here should also check [`LoadOutcome::error`].
    pub fn into_config(self) -> Config {
        match self {
            LoadOutcome::Stored(c) => c,
            LoadOutcome::Unset | LoadOutcome::Broken(_) => Config::default(),
        }
    }

    /// The failure reason, if and only if the backend is broken (not just
    /// unset).
    pub fn error(&self) -> Option<&str> {
        match self {
            LoadOutcome::Broken(reason) => Some(reason),
            LoadOutcome::Stored(_) | LoadOutcome::Unset => None,
        }
    }
}

/// Loads and persists [`Config`]. Host builds use [`DefaultStore`]; device
/// builds use `#[cfg(feature = "device")]` `AxParameterStore`.
pub trait ConfigStore: Send + Sync + 'static {
    /// Load the current config. See [`LoadOutcome`] — "nothing stored yet"
    /// and "the backend is broken" are distinct outcomes, both of which
    /// currently run on [`Config::default`], but only the latter is a real
    /// failure worth surfacing.
    fn load(&self) -> LoadOutcome;
    /// Persist `c` as the new config.
    fn store(&self, c: &Config) -> crate::Result<()>;
}

/// Host + fallback [`ConfigStore`]: `load` always returns
/// [`LoadOutcome::Unset`] (there genuinely is no backend, so "nothing
/// stored" is accurate, not a masked failure), `store` is a no-op. Used on
/// host builds (including these tests) and as a device fallback before
/// axparameter is wired up.
pub struct DefaultStore;

impl ConfigStore for DefaultStore {
    fn load(&self) -> LoadOutcome {
        LoadOutcome::Unset
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
    /// `None` when the backend could not be opened at all — the store then
    /// reports [`LoadOutcome::Broken`] and refuses writes, instead of the
    /// process refusing to start. See `unavailable`.
    inner: Option<axparameter::parameter::Parameter>,
    /// Why the backend is unavailable, when `inner` is `None`.
    open_error: Option<String>,
}

/// The ACAP `appName` from `manifest.json`, which libaxparameter uses to
/// locate `/etc/dynamic/param/<appName>.conf`.
///
/// This MUST match `manifest.json`'s `appName` exactly. It is **not**
/// `"acap-multimux"`: ACAP rejects a hyphen in `appName` (fixed in #669),
/// and this string was missed at the time. Passing the hyphenated form makes
/// libaxparameter look for a `.conf` that does not exist, so every `add`/`get`
/// fails with "Failed to get real path for symlink
/// /etc/dynamic/param/acap-multimux.conf" — which is exactly how #955
/// presented on a real camera.
#[cfg(feature = "device")]
pub const ACAP_APP_NAME: &str = "acapmultimux";

#[cfg(feature = "device")]
impl AxParameterStore {
    /// The single axparameter parameter name the whole [`Config`] is
    /// serialized under (as JSON).
    const PARAM_NAME: &'static str = "Config";

    /// Open the `acap-multimux` axparameter handle, creating the `Config`
    /// parameter if this is the first run on this camera.
    ///
    /// Issue #955: `store` called `Parameter::set("Config", …)` on a
    /// parameter that was never `add`ed, so persisting a config failed on
    /// every camera (`axparameter set: Failed to set parameter Config`) —
    /// confirmed on-device: `param.cgi?action=list&group=acap-multimux`
    /// returned "Error -1 getting param in group". [`Self::ensure_parameter`]
    /// registers the parameter (with [`Config::default`] as its initial
    /// value) exactly once per camera, then every subsequent `new()` finds
    /// it already there.
    pub fn new() -> crate::Result<Self> {
        let inner = axparameter::parameter::Parameter::new(ACAP_APP_NAME)
            .map_err(|e| crate::AcapError::Config(format!("axparameter open: {e}")))?;
        let store = AxParameterStore {
            inner: Some(inner),
            open_error: None,
        };
        store.ensure_parameter()?;
        Ok(store)
    }

    /// A store whose backend could not be opened.
    ///
    /// Every read reports [`LoadOutcome::Broken`] and every write fails, but
    /// the app still starts and serves media on [`Config::default`]. Exiting
    /// instead produced a respawn loop on a real camera (#955).
    #[must_use]
    pub fn unavailable(reason: String) -> Self {
        AxParameterStore {
            inner: None,
            open_error: Some(reason),
        }
    }

    /// Register [`Self::PARAM_NAME`] via `axparameter::Parameter::add` if it
    /// doesn't already exist.
    ///
    /// Idempotent by design, not just by accident: a second app start (every
    /// restart, forever, since `runMode: "respawn"` in `manifest.json`) must
    /// not fail just because the parameter is now there. `add` reports that
    /// case as `ParameterError::ParamAdded` ("already added") — matching the
    /// vendored `axparameter_example` app's own `add`-then-ignore-ParamAdded
    /// pattern — so that specific error is swallowed; any other error means
    /// the backend itself is broken and is propagated.
    fn ensure_parameter(&self) -> crate::Result<()> {
        let initial = serde_json::to_string(&Config::default())
            .map_err(|e| crate::AcapError::Config(format!("config serialize: {e}")))?;
        let Some(inner) = self.inner.as_ref() else {
            return Ok(());
        };
        match inner.add(Self::PARAM_NAME, None, initial) {
            Ok(()) => Ok(()),
            Err(e)
                if e.matches::<axparameter::error::ParameterError>(
                    axparameter::error::ParameterError::ParamAdded,
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(crate::AcapError::Config(format!("axparameter add: {e}"))),
        }
    }
}

#[cfg(feature = "device")]
impl ConfigStore for AxParameterStore {
    fn load(&self) -> LoadOutcome {
        // `ensure_parameter` (run once in `new()`) guarantees the parameter
        // exists with a JSON value by the time `load` can ever be called on
        // a live `AxParameterStore` — so a `get` failure here means the
        // backend is broken (dbus/file-level failure), never "nothing
        // stored yet". Keeping `Unset` reachable only through `DefaultStore`
        // is deliberate: it is the honest description of *that* store,
        // whereas an `AxParameterStore` `get` error is a real fault to
        // surface, not a design-accepted absence.
        let Some(inner) = self.inner.as_ref() else {
            return LoadOutcome::Broken(
                self.open_error
                    .clone()
                    .unwrap_or_else(|| "config backend unavailable".to_string()),
            );
        };
        match inner.get::<String>(Self::PARAM_NAME) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(cfg) => LoadOutcome::Stored(cfg),
                Err(e) => LoadOutcome::Broken(format!("stored config is not valid JSON: {e}")),
            },
            Err(e) => LoadOutcome::Broken(format!("axparameter get: {e}")),
        }
    }

    fn store(&self, c: &Config) -> crate::Result<()> {
        let s = serde_json::to_string(c)
            .map_err(|e| crate::AcapError::Config(format!("config serialize: {e}")))?;
        let Some(inner) = self.inner.as_ref() else {
            return Err(crate::AcapError::Config(
                self.open_error
                    .clone()
                    .unwrap_or_else(|| "config backend unavailable".to_string()),
            ));
        };
        inner
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

/// Internal state behind [`StatusHandle`]: the served [`Status`] plus a
/// config-backend error tracked in a slot separate from the pipeline's own
/// `last_error`. Kept apart so the capture pipeline's routine
/// "clear the previous attempt's error at the start of a new one"
/// (`run_vdo_capture` in the `acap-multimux` binary) can never silently wipe
/// out evidence that the config store itself is broken — exactly the kind
/// of masked failure issue #955 diagnosed in `ConfigStore::load` itself.
#[derive(Default)]
struct StatusState {
    status: Status,
    config_error: Option<String>,
}

/// Shared, cloneable handle to a [`Status`], read by the admin routes and
/// updated by the running pipeline.
#[derive(Clone)]
pub struct StatusHandle(Arc<Mutex<StatusState>>);

impl StatusHandle {
    /// A fresh handle around [`Status::default`] (not running, no frames).
    pub fn new() -> Self {
        StatusHandle(Arc::new(Mutex::new(StatusState::default())))
    }

    /// The current status, cloned out from behind the lock. `last_error`
    /// prefers the pipeline's own error (something actionable is failing
    /// right now); if the pipeline currently reports none, it falls back to
    /// the persistent config-backend error set via
    /// [`StatusHandle::set_config_error`] — so a broken config store stays
    /// visible even while the capture pipeline itself is running cleanly on
    /// defaults.
    pub fn snapshot(&self) -> Status {
        let state = self.0.lock().expect("status mutex poisoned");
        let mut status = state.status.clone();
        if status.last_error.is_none() {
            status.last_error = state.config_error.clone();
        }
        status
    }

    /// Mark the pipeline as running or stopped.
    pub fn set_running(&self, running: bool) {
        self.0.lock().expect("status mutex poisoned").status.running = running;
    }

    /// Update the current segment/part position.
    pub fn set_position(&self, current_segment: u32, current_part: u32) {
        let mut state = self.0.lock().expect("status mutex poisoned");
        state.status.current_segment = current_segment;
        state.status.current_part = current_part;
    }

    /// Add `n` to the processed-frame counter.
    pub fn add_frames(&self, n: u64) {
        self.0.lock().expect("status mutex poisoned").status.frames += n;
    }

    /// Record (or clear) the pipeline's own most recent error. This is the
    /// pipeline's slot, not the config store's — see
    /// [`StatusHandle::set_config_error`] for why the two are kept apart.
    pub fn set_last_error(&self, err: Option<String>) {
        self.0
            .lock()
            .expect("status mutex poisoned")
            .status
            .last_error = err;
    }

    /// Record (or clear) a config-backend error (issue #955:
    /// [`ConfigStore::load`]'s [`LoadOutcome::error`]) in a slot the capture
    /// pipeline never touches, so it survives every pipeline retry's own
    /// `set_last_error(None)`/`set_last_error(Some(..))` churn. There is no
    /// live path that clears this today — the config store is only loaded
    /// once at boot — matching the honest state of a `respawn`-mode ACAP
    /// app: fixing the backend requires a restart anyway.
    pub fn set_config_error(&self, err: Option<String>) {
        self.0.lock().expect("status mutex poisoned").config_error = err;
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
    let outcome = state.store.load();
    // A broken backend is real evidence, not noise to discard the way the
    // old `load()` did (issue #955) — surface it through `/admin/status`'s
    // `last_error` so an operator watching the settings page sees it even
    // though this endpoint still has to answer with *some* `Config`. Uses
    // the config-specific slot (`set_config_error`), not the pipeline's
    // `set_last_error`, so a later pipeline restart's own error handling
    // can't silently clear it.
    if let Some(reason) = outcome.error() {
        state
            .status
            .set_config_error(Some(format!("config load: {reason}")));
    }
    Json(outcome.into_config())
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

    #[test]
    fn load_outcome_unset_and_broken_both_fall_back_to_default_config() {
        assert_eq!(LoadOutcome::Unset.into_config(), Config::default());
        assert_eq!(
            LoadOutcome::Broken("simulated".to_string()).into_config(),
            Config::default()
        );
    }

    #[test]
    fn load_outcome_error_distinguishes_broken_from_unset_and_stored() {
        assert_eq!(LoadOutcome::Unset.error(), None);
        assert_eq!(LoadOutcome::Stored(Config::default()).error(), None);
        assert_eq!(
            LoadOutcome::Broken("axparameter get: boom".to_string()).error(),
            Some("axparameter get: boom")
        );
    }

    /// A [`ConfigStore`] test double standing in for a broken axparameter
    /// backend (issue #955): `load` always reports [`LoadOutcome::Broken`],
    /// the same shape `AxParameterStore::load` now returns instead of
    /// silently falling back to defaults.
    struct BrokenStore;

    impl ConfigStore for BrokenStore {
        fn load(&self) -> LoadOutcome {
            LoadOutcome::Broken("axparameter get: simulated backend failure".to_string())
        }

        fn store(&self, _c: &Config) -> crate::Result<()> {
            Ok(())
        }
    }

    /// A [`ConfigStore`] test double for a backend that has a real stored
    /// value, distinguishing [`LoadOutcome::Stored`] from the
    /// default-shaped [`LoadOutcome::Unset`]/[`LoadOutcome::Broken`] cases
    /// `DefaultStore`/`BrokenStore` cover.
    struct StoredStore;

    impl ConfigStore for StoredStore {
        fn load(&self) -> LoadOutcome {
            LoadOutcome::Stored(Config {
                codec: "h265".to_string(),
                ..Config::default()
            })
        }

        fn store(&self, _c: &Config) -> crate::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn get_config_returns_the_actually_stored_value_when_present() {
        let response = admin_router(Arc::new(StoredStore), StatusHandle::new())
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
        assert_eq!(cfg.codec, "h265");
    }

    #[tokio::test]
    async fn get_config_on_broken_backend_still_serves_defaults_but_records_last_error() {
        let status = StatusHandle::new();
        let router = admin_router(Arc::new(BrokenStore), status.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/admin/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The response itself is indistinguishable from "unconfigured" —
        // that's expected, the app still has to boot on *something* — but
        // unlike the bug in #955, the failure is no longer invisible: it
        // must now be visible through `/admin/status`.
        assert_eq!(response.status(), StatusCode::OK);
        let cfg: Config = serde_json::from_value(body_json(response).await).unwrap();
        assert_eq!(cfg, Config::default());

        let last_error = status.snapshot().last_error;
        assert!(
            last_error
                .as_deref()
                .is_some_and(|e| e.contains("simulated backend failure")),
            "expected last_error to surface the broken backend, got {last_error:?}"
        );
    }
}
