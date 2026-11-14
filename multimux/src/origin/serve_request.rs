use std::path::Path;
use std::path::PathBuf;

use crate::registry::SchemeRegistry;

#[cfg(feature = "test-seams")]
use super::PreboundListeners;
use super::serve_with_registry_impl;

/// One request to run the multimux origin: the config, the [`SchemeRegistry`]
/// resolving `Custom` schemes, and, when the caller supplies them, the config
/// file `POST /admin/reload` re-reads and the listeners the caller has already
/// bound.
///
/// Build one with [`ServeRequest::new`] (an in-memory config) or
/// [`ServeRequest::from_json_file`] (a config file, so the admin reload keeps
/// working), then set the registry and the pre-bound listeners and hand it to
/// [`crate::Server::new`]. A pre-bound listener pair can only come from a
/// caller that bound `127.0.0.1:0` itself and read the live address back
/// (`test-seams` only): no production caller has a listener before it serves.
#[derive(Default)]
pub struct ServeRequest {
    config: crate::config::Config,
    registry: SchemeRegistry,
    config_path: Option<PathBuf>,
    #[cfg(feature = "test-seams")]
    prebound: Option<PreboundListeners>,
}

impl ServeRequest {
    /// Serve `config` with an empty [`SchemeRegistry`]: an in-memory config has
    /// no file to re-read, so `POST /admin/reload` is unavailable under this
    /// entry point (use [`ServeRequest::from_json_file`] if reload is needed).
    #[must_use]
    pub fn new(config: crate::config::Config) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Serve the JSON config at `path`, remembering it so, if
    /// [`crate::config::Config::admin`] is configured, `POST /admin/reload`
    /// (issue #749) can re-read it later.
    ///
    /// # Errors
    /// The file cannot be read or does not parse as a
    /// [`crate::config::Config`].
    pub fn from_json_file(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let config = crate::config::Config::from_json_file(&path)?;
        Ok(Self {
            config,
            config_path: Some(path),
            ..Self::default()
        })
    }

    /// Resolve `Custom` schemes through `registry` — see
    /// [`crate::registry::SchemeRegistry`]. Without one, a config whose
    /// route/output-auth uses a `Custom` scheme fails setup with
    /// [`crate::MultimuxError::UnknownScheme`].
    #[must_use]
    pub fn with_registry(mut self, registry: SchemeRegistry) -> Self {
        self.registry = registry;
        self
    }

    /// Serve over `media` plus `admin` listeners the caller bound and passed
    /// in (`test-seams` only, SP7.1): a test binds `127.0.0.1:0`, reads the
    /// live address and passes the listener in, so no test reserves a port and
    /// then races to re-bind it.
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    #[must_use]
    pub fn with_prebound_listeners(
        mut self,
        media: tokio::net::TcpListener,
        admin: tokio::net::TcpListener,
    ) -> Self {
        self.prebound = Some(PreboundListeners {
            media,
            admin: Some(admin),
        });
        self
    }

    /// Serve over a `media` listener the caller bound and passed in
    /// (`test-seams` only, SP7.1); the admin path needs
    /// [`Self::with_prebound_listeners`].
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    #[must_use]
    pub fn with_prebound_media(mut self, media: tokio::net::TcpListener) -> Self {
        self.prebound = Some(PreboundListeners { media, admin: None });
        self
    }

    /// Splice caller-bound route sockets into the config loaded from the file
    /// (`test-seams` only, SP7.1). A config file cannot carry
    /// [`crate::config::Config::prebound`] (it is `serde(skip)`), so a test
    /// that serves a *file-defined* route and still wants to hand a pre-bound
    /// listener/socket in uses this setter: the binds are spliced into the
    /// config before it is served.
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    #[must_use]
    pub fn with_prebound_binds(mut self, prebound: crate::config::PreboundBinds) -> Self {
        self.config.prebound = prebound;
        self
    }

    pub(crate) async fn serve(self) -> crate::Result<()> {
        serve_with_registry_impl(self.config, self.registry, self.config_path, {
            #[cfg(feature = "test-seams")]
            {
                self.prebound
            }
            #[cfg(not(feature = "test-seams"))]
            {
                None
            }
        })
        .await
    }
}

/// The multimux origin server.
///
/// Build one with [`Server::new`] from a [`ServeRequest`] and run it with
/// [`Server::serve`]; this is the one entry point that runs the origin.
///
/// Every configured [`crate::config::InputSpec`] variant drives a route
/// (issue #805 tasks 2/4/5 — see `spawn_ingest`, which the run calls) through
/// [`crate::origin::supervisor::supervise_driver`] (a `media_plane::ingress::IngestDriver`-
/// (or, for a push/`Listener`-shaped source like RTMP, `ListenDriver`-)
/// driven `crate::source::*::run_*` entry point — or, for
/// [`crate::config::InputSpec::Custom`], the equivalent driver loop a
/// registered [`crate::registry::InputFactory`] builds itself), publishing
/// each announced program's driver-minted `Trunk` into the route's registry
/// via [`crate::source::advance_route`]. A connect failure, protocol error, or
/// clean end-of-stream reconnects with capped backoff instead of dying — a
/// bad/flaky source degrades that route's [`crate::route::RouteHandle::health`]
/// rather than freezing it forever, and never brings the server (or any other
/// route) down.
pub struct Server {
    request: ServeRequest,
}

impl Server {
    /// The server for `request`; nothing binds until [`Server::serve`].
    #[must_use]
    pub fn new(request: ServeRequest) -> Self {
        Self { request }
    }

    /// Run the origin: one [`crate::route::RouteHandle`] + one ingest task per
    /// `config.routes` entry, then bind `config.bind` and serve them all under
    /// [`crate::origin::router`]. Each route is served by the [`crate::output::Output`]s named in its
    /// [`crate::config::Route::outputs`] (LL-HLS by default — see
    /// [`crate::output::OutputKind`]).
    ///
    /// Installs a graceful-shutdown signal (Ctrl-C, plus SIGTERM on unix): on
    /// receipt, the server stops accepting new connections and drains
    /// in-flight requests (including blocked LL-HLS long-poll reloads) up to
    /// `DRAIN_DEADLINE` via the origin's `serve_hyper_util`
    /// `GracefulShutdown`, the same signal breaks every route's supervise
    /// loop, and `serve` joins each supervisor task (forcibly aborting one
    /// that doesn't return within a short grace period) before returning
    /// `Ok(())`.
    ///
    /// Otherwise returns only on a bind failure or if the HTTP server itself
    /// stops (e.g. a fatal accept-loop I/O error).
    ///
    /// If [`crate::config::Config::admin`] is configured, this delegates whole
    /// to `admin::serve_with_admin` (issue #749) instead of the static
    /// single-`Router` path — see that function's docs for the runtime admin
    /// API (separate listener, mandatory auth, add/remove/list routes +
    /// reload).
    ///
    /// # Errors
    /// The config fails [`crate::config::Config::validate`], a `Custom` scheme
    /// is not registered ([`crate::MultimuxError::UnknownScheme`]), or a
    /// listener cannot bind.
    pub async fn serve(self) -> crate::Result<()> {
        self.request.serve().await
    }
}
