//! External scheme plugin registry (issue #663): lets a third-party crate add
//! a new input/output/output-auth scheme to the multimux origin **without
//! editing multimux**, wired purely via config JSON.
//!
//! Built-in schemes (RTSP/RTP/TS-UDP/TS-HTTP/HLS-pull inputs; LL-HLS/DASH/
//! LL-DASH outputs; Basic/Digest/Bearer/Forwarded output-auth) stay exactly
//! as they are — the typed, validated fast path [`crate::config`] and
//! [`crate::output`] already implement. Extension is purely additive: a
//! config names an external scheme via a `Custom { type_tag, params }`
//! variant ([`crate::config::InputSpec::Custom`],
//! [`crate::output::OutputKind::Custom`],
//! [`crate::config::OutputAuthSpec::Custom`]), and a [`SchemeRegistry`] —
//! built by the embedding application, never by multimux itself — maps each
//! `type_tag` to a factory closure that knows how to build the real thing
//! from the opaque `params` JSON. [`crate::origin::serve_with_registry`]
//! resolves `Custom` variants through the registry at route-build time;
//! [`crate::origin::serve`] is `serve_with_registry` with an empty registry,
//! so an unregistered `Custom` tag in a config passed to plain `serve`
//! always fails with `MultimuxError::UnknownScheme`, never silently no-ops.
//!
//! # Writing an input factory
//!
//! `crate::source::rtsp::RtspSource`-style connectors implement
//! [`crate::origin::supervisor::SourceConnector`] — an associated-type trait
//! that is not object-safe, since `supervise` is generic over it rather than
//! boxed. An [`InputFactory`] therefore cannot simply return a connector
//! trait object: instead, the factory closure constructs its own concrete
//! `impl SourceConnector`, spawns the re-exported
//! [`crate::origin::supervisor::supervise`] itself, and returns the
//! resulting [`tokio::task::JoinHandle`] — this erases the connector type at
//! the closure boundary instead of at the trait boundary. `Output` (unlike
//! `SourceConnector`) is already object-safe, so an [`OutputFactory`] can
//! return `Arc<dyn crate::output::Output>` directly; `AuthFactory` similarly
//! returns a concrete `broadcast_auth::Verifier` (also not generic).
//!
//! See `examples/custom_scheme.rs` for a complete (if minimal) external
//! input scheme registered with zero multimux edits.

use std::collections::HashMap;
use std::sync::Arc;

/// Everything an [`InputFactory`] needs to build and spawn one route's
/// supervised ingest task for a [`crate::config::InputSpec::Custom`] route.
pub struct InputCtx {
    /// The route's served stream name (`crate::config::Route::name`) — pass
    /// through to [`crate::origin::supervisor::supervise`]'s own `name`
    /// parameter (used only for logging/metrics labels, never a credential).
    pub name: String,
    /// The route's `InputSpec::Custom::params`, opaque to multimux.
    pub params: serde_json::Value,
    /// This route's shared store — the factory's connector feeds
    /// `run_pipeline` (via `supervise`) into this same store the origin
    /// serves reads from.
    pub store: Arc<crate::store::MediaStore>,
    /// `crate::config::Config::target_duration_secs`, forwarded unchanged —
    /// pass straight through to `supervise`.
    pub target_duration_secs: f64,
    /// `crate::config::Config::part_target_ms`, forwarded unchanged.
    pub part_target_ms: u32,
    /// The shared shutdown signal every other route's supervisor task
    /// watches — the factory's spawned task must watch it too, so
    /// [`crate::origin::serve_with_registry`]'s graceful shutdown actually
    /// waits for it instead of leaving it running detached.
    pub shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

/// Builds and spawns one [`crate::config::InputSpec::Custom`] route's ingest
/// task, returning its `JoinHandle` — see this module's docs for why a
/// factory spawns `supervise` itself rather than returning a connector.
pub type InputFactory =
    Arc<dyn Fn(InputCtx) -> crate::Result<tokio::task::JoinHandle<()>> + Send + Sync>;

/// Everything an [`OutputFactory`] needs to build one
/// [`crate::output::OutputKind::Custom`]'s [`crate::output::Output`].
pub struct OutputCtx<'a> {
    /// The `OutputKind::Custom::params`, opaque to multimux.
    pub params: serde_json::Value,
    /// `crate::config::Config::playlist_name` — mirrors
    /// [`crate::output::OutputKind::build_with_playlist_name`]'s own
    /// parameter, for a custom output that also serves a configurable
    /// LL-HLS-style media playlist filename.
    pub playlist_name: &'a str,
}

/// Builds one [`crate::output::OutputKind::Custom`]'s
/// [`crate::output::Output`].
pub type OutputFactory =
    Arc<dyn Fn(&OutputCtx) -> crate::Result<Arc<dyn crate::output::Output>> + Send + Sync>;

/// Everything an [`AuthFactory`] needs to build one
/// [`crate::config::OutputAuthSpec::Custom`]'s `broadcast_auth::Verifier`.
pub struct AuthCtx<'a> {
    /// The `OutputAuthSpec::Custom::params`, opaque to multimux.
    pub params: serde_json::Value,
    /// The realm the shared output-auth `Verifier` challenges with — mirrors
    /// `crate::config::OutputAuthSpec::build_verifier`'s own parameter
    /// (`crate::origin`'s `OUTPUT_AUTH_REALM`).
    pub realm: &'a str,
}

/// Builds one [`crate::config::OutputAuthSpec::Custom`]'s
/// `broadcast_auth::Verifier`.
pub type AuthFactory =
    Arc<dyn Fn(&AuthCtx) -> crate::Result<broadcast_auth::Verifier> + Send + Sync>;

/// Maps a `Custom` variant's `type_tag` to the factory that builds the real
/// thing. Empty by default ([`SchemeRegistry::new`]) — built-ins are the
/// typed config fast path and need no registration; only an application
/// embedding multimux that actually uses a `Custom` scheme needs to build and
/// populate one, then pass it to [`crate::origin::serve_with_registry`].
#[derive(Clone, Default)]
pub struct SchemeRegistry {
    inputs: HashMap<String, InputFactory>,
    outputs: HashMap<String, OutputFactory>,
    auths: HashMap<String, AuthFactory>,
}

impl SchemeRegistry {
    /// An empty registry — every `Custom` tag resolves to
    /// [`crate::MultimuxError::UnknownScheme`] until registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an input-scheme factory under `tag` (an
    /// `InputSpec::Custom::type_tag` this factory answers), replacing any
    /// previously registered factory for the same tag. Returns `&mut Self` so
    /// registrations can be chained.
    pub fn register_input(&mut self, tag: impl Into<String>, factory: InputFactory) -> &mut Self {
        self.inputs.insert(tag.into(), factory);
        self
    }

    /// Registers an output-scheme factory under `tag`, replacing any
    /// previously registered factory for the same tag.
    pub fn register_output(&mut self, tag: impl Into<String>, factory: OutputFactory) -> &mut Self {
        self.outputs.insert(tag.into(), factory);
        self
    }

    /// Registers an output-auth-scheme factory under `tag`, replacing any
    /// previously registered factory for the same tag.
    pub fn register_auth(&mut self, tag: impl Into<String>, factory: AuthFactory) -> &mut Self {
        self.auths.insert(tag.into(), factory);
        self
    }

    /// Looks up the input factory registered for `tag`, if any.
    pub fn input(&self, tag: &str) -> Option<&InputFactory> {
        self.inputs.get(tag)
    }

    /// Looks up the output factory registered for `tag`, if any.
    pub fn output(&self, tag: &str) -> Option<&OutputFactory> {
        self.outputs.get(tag)
    }

    /// Looks up the auth factory registered for `tag`, if any.
    pub fn auth(&self, tag: &str) -> Option<&AuthFactory> {
        self.auths.get(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh registry resolves nothing — every lookup is `None`.
    #[test]
    fn new_registry_has_no_factories() {
        let registry = SchemeRegistry::new();
        assert!(registry.input("anything").is_none());
        assert!(registry.output("anything").is_none());
        assert!(registry.auth("anything").is_none());
    }

    /// Biting test: a registered input factory is found by its exact tag,
    /// and an unrelated tag still misses — proves the lookup is keyed, not a
    /// "registered at all" flag. The factory is never invoked (no tokio
    /// runtime in this plain `#[test]`); presence/absence is what's under
    /// test here, mirroring `SchemeRegistry::input`'s own contract.
    #[test]
    fn registered_input_factory_is_found_by_exact_tag_only() {
        let mut registry = SchemeRegistry::new();
        let factory: InputFactory =
            Arc::new(|_ctx: InputCtx| unreachable!("factory is not invoked by this test"));
        registry.register_input("silence", factory);

        assert!(registry.input("silence").is_some());
        assert!(registry.input("nope").is_none());
    }

    /// Same property for `register_output`/`output`.
    #[test]
    fn registered_output_factory_is_found_by_exact_tag_only() {
        let mut registry = SchemeRegistry::new();
        let factory: OutputFactory =
            Arc::new(|_ctx: &OutputCtx| unreachable!("factory is not invoked by this test"));
        registry.register_output("webrtc", factory);

        assert!(registry.output("webrtc").is_some());
        assert!(registry.output("nope").is_none());
    }

    /// Same property for `register_auth`/`auth`.
    #[test]
    fn registered_auth_factory_is_found_by_exact_tag_only() {
        let mut registry = SchemeRegistry::new();
        let factory: AuthFactory =
            Arc::new(|_ctx: &AuthCtx| unreachable!("factory is not invoked by this test"));
        registry.register_auth("hmac", factory);

        assert!(registry.auth("hmac").is_some());
        assert!(registry.auth("nope").is_none());
    }

    /// `register_*` returns `&mut Self`, so registrations chain.
    #[test]
    fn register_calls_chain() {
        let mut registry = SchemeRegistry::new();
        registry
            .register_input("a", Arc::new(|_ctx: InputCtx| unreachable!("not invoked")))
            .register_input("b", Arc::new(|_ctx: InputCtx| unreachable!("not invoked")));
        assert!(registry.input("a").is_some());
        assert!(registry.input("b").is_some());
    }
}
