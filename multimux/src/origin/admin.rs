//! Runtime admin API (issue #749): add/remove/list routes, and reload the
//! config file, without restarting the origin — restarting drops every live
//! viewer on every OTHER route, not just the one being changed.
//!
//! # Security posture (decided, non-negotiable — see this crate's README)
//!
//! - **Separate listener.** The admin API binds its own address
//!   ([`crate::config::AdminSpec::bind`]), distinct from the media listener
//!   ([`crate::config::Config::bind`]) — [`crate::config::Config::validate`]
//!   rejects a config where the two are equal. It is never reachable on the
//!   public media port.
//! - **Mandatory auth.** [`crate::config::AdminSpec::auth`] is a plain
//!   `OutputAuthSpec`, not `Option<OutputAuthSpec>` — a config that enables
//!   the admin API without naming a scheme fails to *deserialize*, before
//!   the process ever binds a socket. There is no unauthenticated admin
//!   listener.
//! - **Opt-in.** [`crate::config::Config::admin`] defaults to `None`: no
//!   admin field configured means no admin listener, no admin routes, at
//!   all — `serve_with_admin` is only ever reached when it is `Some`.
//!
//! # Endpoints
//!
//! | Method | Path | |
//! | --- | --- | --- |
//! | `GET` | `/admin/routes` | List every route + live status. |
//! | `GET` | `/admin/routes/{name}` | One route's status, or `404`. |
//! | `POST` | `/admin/routes` | Add a route (JSON body: [`crate::config::Route`]). `409` if `name` already exists. |
//! | `DELETE` | `/admin/routes/{name}` | Remove a route (`404` if unknown). Stops its supervisor and drains — see `RouteRegistry::remove_route`. |
//! | `POST` | `/admin/reload` | Re-read the config file this process was started with and converge to it — see `RouteRegistry::reload`. |
//!
//! Every mutation is validated (`crate::config::Route::validate_standalone`)
//! before it is applied: a malformed `POST /admin/routes` body is rejected
//! whole (`400`), the route list is left exactly as it was.
//!
//! # Concurrency
//!
//! `RouteRegistry` holds every live route behind a `std::sync::RwLock`,
//! mirroring [`crate::route::RouteHandle`]'s own `RwLock<HashMap<..>>`
//! program registry: many concurrent readers (every HTTP request against a
//! route that ISN'T being mutated) must never block on an admin operation
//! elsewhere. The lock is only ever held across synchronous work (building a
//! route's `Output`s and spawning its supervisor task is non-blocking —
//! `tokio::spawn` returns immediately) — **never across an `.await`**.
//! `RouteRegistry::remove_route`/`RouteRegistry::reload` follow a
//! two-phase shape for this reason: remove the entry (and rebuild the router
//! so new requests stop resolving it) under a brief write lock, THEN
//! `.await` that route's supervisor shutdown/drain with no lock held at all
//! — so a slow-to-drain route never stalls a concurrent `GET`/`POST`
//! against any other route.
//!
//! The axum [`Router`] serving media (manifests + resource routes, nested
//! per stream name — see [`super::router`]) is rebuilt whole on every
//! mutation and hot-swapped behind `RouteRegistry::router_slot` — an axum
//! `Router` is cheap to clone (internally reference-counted), so rebuilding
//! on the rare admin mutation and cloning-on-every-request
//! (`RouteRegistry::current_router`) is far cheaper than mutating a live
//! route tree in place, which axum's `matchit`-backed router does not
//! support at all (nests are fixed at build time — see [`super::router`]'s
//! own docs on the "multi-output nest collision" this same fact caused
//! elsewhere).
//!
//! # What an in-flight viewer sees on `DELETE`
//!
//! The moment `DELETE /admin/routes/{name}` returns, the media `Router` has
//! already been rebuilt without that route's nest — a *new* request for
//! `/{name}/...` 404s immediately, exactly like a name that was never
//! configured. A request that was already being served *from* that route
//! (an open LL-HLS blocking-reload long-poll, e.g.) is not aborted: its
//! `Arc<RouteHandle>`/`Trunk` stays alive (nothing holding an `Arc` to it is
//! forced to drop), so it completes normally against whatever media had
//! already landed — it just never sees a segment/part produced after the
//! ingest supervisor is told to stop. This is a graceful drain, not an
//! abrupt disconnect: no in-flight response is ever cut off mid-body.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, Path as AxumPath, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use broadcast_auth::{AuthResult, Verifier};
use tokio::sync::watch;
use tower::Service;

use crate::config::{Config, Route};
use crate::output::{Output, OutputKind};
use crate::registry::SchemeRegistry;
use crate::route::RouteHandle;

use super::{AppState, HttpLimits, SUPERVISOR_SHUTDOWN_GRACE, StreamRoute, build_output, router};

/// Realm the admin API's mandatory `Verifier` challenges with — distinct
/// from [`super::OUTPUT_AUTH_REALM`] (which names the media output-auth
/// realm) since these are two independent credentials.
const ADMIN_AUTH_REALM: &str = "multimux-admin";

/// One live route's runtime state, as [`RouteRegistry`] tracks it: the
/// [`Route`] config it was built from (compared for equality on
/// [`RouteRegistry::reload`], and reported back by `GET /admin/routes`), the
/// serving state every request resolves against, and the handle needed to
/// stop its supervisor task on removal.
struct RouteRuntime {
    route: Route,
    store: Arc<RouteHandle>,
    outputs: Vec<Arc<dyn Output>>,
    /// The route's one shutdown token — cancels the ingest supervisor AND
    /// every push/WHEP egress task (SP1.4: one token, no `watch<bool>`).
    cancel: tokio_util::sync::CancellationToken,
    handle: tokio::task::JoinHandle<()>,
    push_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for RouteRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteRuntime")
            .field("route", &self.route.name)
            .finish_non_exhaustive()
    }
}

/// A [`RouteRuntime`] that is *not yet installed in the registry*. Dropping
/// one — because a later step in the same `add_route`/`reload` failed, or a
/// panic unwound — cancels its push/WHEP tasks and aborts its supervisor, so
/// a half-built route can never leave a dialer or a bound listen port behind
/// (audit run 7, W6/A2). [`Self::into_installed`] disarms the guard and takes
/// the runtime out, so an installed runtime is *not* torn down on drop; the
/// registry owns it from then on and tears it down via [`drain_route`].
struct PendingRuntime {
    runtime: Option<RouteRuntime>,
    armed: bool,
}

impl PendingRuntime {
    fn new(runtime: RouteRuntime) -> Self {
        PendingRuntime {
            runtime: Some(runtime),
            armed: true,
        }
    }

    /// The guarded runtime's store — for spawning push/WHEP outputs against
    /// it *before* the runtime is fully assembled (item 7).
    fn store(&self) -> Arc<RouteHandle> {
        Arc::clone(&self.runtime.as_ref().expect("armed").store)
    }

    /// Attach the push/WHEP handles spawned against this guard's store.
    fn with_push_handles(mut self, handles: Vec<tokio::task::JoinHandle<()>>) -> Self {
        self.runtime
            .as_mut()
            .expect("armed")
            .push_handles
            .extend(handles);
        self
    }

    /// Take the assembled runtime out (disarming the rollback) — used by
    /// [`RouteRegistry::spawn_route`], whose result the caller re-wraps.
    fn into_runtime(self) -> RouteRuntime {
        self.into_installed()
    }

    /// Take the runtime out for installation, disarming the rollback.
    fn into_installed(mut self) -> RouteRuntime {
        self.armed = false;
        self.runtime
            .take()
            .expect("PendingRuntime::into_installed called twice")
    }
}

impl Drop for PendingRuntime {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        runtime.cancel.cancel();
        runtime.handle.abort();
        for h in runtime.push_handles {
            h.abort();
        }
    }
}

/// Everything [`RouteRegistry`] needs to build/rebuild a route or the media
/// [`Router`] over it, gathered once at startup — process-wide settings that
/// [`RouteRegistry::reload`] does not change (see that method's own docs:
/// only `routes` are converged by reload, everything else needs a restart).
struct RegistryContext {
    /// A clone of the process's [`Config`] with `routes`/`admin` cleared —
    /// [`super::spawn_ingest`] reads `target_duration_secs`/`part_target_ms`/
    /// `window_segments`/the ingest timeouts from it; `routes` and `admin`
    /// are ignored (kept empty so nothing accidentally reads a stale route
    /// list from here instead of [`RouteRegistry::inner`]).
    base_config: Config,
    scheme_registry: SchemeRegistry,
    http_limits: HttpLimits,
    output_auth: Option<Arc<Verifier>>,
}

/// The shared, mutable heart of the runtime admin API: every live route,
/// behind a lock cheap enough to read on every admin request and mutate
/// rarely — see this module's own "Concurrency" docs.
pub(crate) struct RouteRegistry {
    ctx: RegistryContext,
    /// Serialises every registry **mutation** (`add_route`/`remove_route`/
    /// `reload`/`shutdown_all`, audit run 7, W6). Without it, two concurrent
    /// `POST /admin/routes`/`POST /admin/reload` calls each read a snapshot,
    /// build their runtime, then `insert` — the loser's `RouteRuntime` is
    /// silently overwritten and dropped with no `drain_route`, so its
    /// supervisor task detaches (still serving) and any listen port
    /// (RTMP/SRT/WHIP/TS-UDP/WHEP) stays bound forever. An async mutex, not
    /// a `std` one: the critical section awaits `drain_route` (which awaits
    /// the displaced supervisor's shutdown), so it must not block the
    /// executor thread.
    ///
    /// Only mutations take it; reads (`resolve`/`current_router`) stay
    /// lock-free against it, so admin traffic never blocks media serving.
    mutation_lock: tokio::sync::Mutex<()>,
    inner: parking_lot::RwLock<HashMap<String, RouteRuntime>>,
    /// The currently-active media [`Router`], rebuilt whole on every
    /// mutation (see this module's own docs) and read by
    /// [`DynamicMediaService`] once per accepted request. `ArcSwap` makes
    /// that read lock-free: a rebuild never blocks a request already in
    /// flight, and a request never takes a lock the rebuild path wants.
    router_slot: arc_swap::ArcSwap<Router>,
    /// The config file this process was loaded from, if any — `None` when
    /// the embedding caller built its [`Config`] in memory (e.g.
    /// `serve_with_registry` directly): [`Self::reload`] has nothing to
    /// re-read in that case and fails clearly rather than silently no-oping.
    config_path: Option<PathBuf>,
}

impl RouteRegistry {
    /// Builds an empty registry (no routes yet) and its (empty) initial
    /// media router — [`serve_with_admin`] populates it via [`Self::add_route`]
    /// immediately after, one call per `config.routes` entry, exactly like a
    /// live `POST /admin/routes` would.
    fn new(ctx: RegistryContext, config_path: Option<PathBuf>) -> Arc<Self> {
        let registry = Arc::new(RouteRegistry {
            ctx,
            mutation_lock: tokio::sync::Mutex::new(()),
            inner: parking_lot::RwLock::new(HashMap::new()),
            router_slot: arc_swap::ArcSwap::from_pointee(Router::new()),
            config_path,
        });
        registry.rebuild_router();
        registry
    }

    fn streams_snapshot(&self) -> HashMap<String, StreamRoute> {
        self.inner
            .read()
            .iter()
            .map(|(name, rt)| (name.clone(), (Arc::clone(&rt.store), rt.outputs.clone())))
            .collect()
    }

    /// Builds the media [`Router`] over exactly `streams`. Pure: touches no
    /// registry state, so [`Self::add_route`] can build the *prospective*
    /// router (current routes plus a not-yet-inserted one) and only commit it
    /// once it is known to have succeeded (audit run 7, W5 — a router build
    /// panics on e.g. a `Custom` output mounting an overlapping path, and
    /// `add_route` must not have already mutated the registry when it does).
    fn build_router(&self, streams: HashMap<String, StreamRoute>) -> Router {
        let mut app_state = AppState::new(streams).with_limits(self.ctx.http_limits);
        if let Some(verifier) = &self.ctx.output_auth {
            app_state = app_state.with_output_auth(Arc::clone(verifier));
        }
        // The concurrency bound (and the ops/reload budgets) is applied
        // inside `router()` itself, so it is present here too.
        router(Arc::new(app_state))
    }

    /// Rebuilds the whole media [`Router`] from the current route set and
    /// hot-swaps it into [`Self::router_slot`] — see this module's own
    /// "Concurrency" docs for why a whole rebuild (rather than an in-place
    /// edit, which axum's router does not support) is the right cost to pay
    /// here.
    fn rebuild_router(&self) {
        let new_router = self.build_router(self.streams_snapshot());
        self.router_slot.store(std::sync::Arc::new(new_router));
    }

    /// A cheap clone of the currently-active media [`Router`] — read once
    /// per accepted request by [`DynamicMediaService`].
    pub(crate) fn current_router(&self) -> Router {
        self.router_slot.load().as_ref().clone()
    }

    /// Builds `route`'s [`Output`]s and spawns its supervised ingest task —
    /// the same two steps [`super::serve_with_registry`]'s own per-route
    /// loop performs, pulled out so [`Self::add_route`]/[`Self::reload`]
    /// share exactly one implementation. Does not touch [`Self::inner`] or
    /// rebuild the router — callers do that once they decide to keep the
    /// result.
    ///
    /// `archive_floor` is the highest sequence number the route's DVR archive
    /// already holds (`crate::dvr::archive_seq_floor`, computed by the async
    /// caller since it reads the archive from disk).
    fn spawn_route(&self, route: &Route, archive_floor: u32) -> crate::Result<RouteRuntime> {
        let outputs: Vec<Arc<dyn Output>> = route
            .outputs
            .iter()
            // `!k.is_whep()` as well as `!k.is_push()` — the startup path in
            // `origin::mod` filters both, and this one did not. A WHEP output
            // survived to `build_output`, which hits `unreachable!()` because
            // WHEP is driven by its own listen socket rather than an
            // `Arc<dyn Output>`. `add_route` holds the registry WRITE LOCK
            // across this call, so the unwind poisoned the lock and every
            // later admin op and router rebuild panicked on
            // `.expect("...poisoned")` — one request, permanent admin lockout.
            .filter(|k| !k.is_push() && !k.is_whep())
            .map(|k| {
                build_output(
                    k,
                    &self.ctx.base_config.playlist_name,
                    &self.ctx.scheme_registry,
                )
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let store = Arc::new(
            RouteHandle::new(
                self.ctx.base_config.target_duration_secs,
                self.ctx.base_config.part_target_ms,
                self.ctx.base_config.window_segments,
            )
            .with_archive_floor(archive_floor)
            .with_name(route.name.clone())
            .with_container(super::route_container(route))
            // Issue #900 — see the identical fix (and its own doc) in
            // `super::serve_with_registry_impl`'s per-route loop; this is
            // the admin-API (`POST /admin/routes`/reload) equivalent.
            .with_dvr(route.dvr.clone()),
        );
        let route_cancel = tokio_util::sync::CancellationToken::new();
        // Spawn the FALLIBLE ingest task first (audit run 7, A2): it is the
        // only step here that can return `Err` (an unknown `Custom` tag, or a
        // factory that returns `Err`). Doing it after the push/WHEP spawns
        // meant an `Err` dropped their handles without cancelling them, so a
        // push dialer kept dialling and a WHEP listener kept its port bound
        // forever, for a route that was never installed.
        let handle = super::spawn_ingest(
            route,
            Arc::clone(&store),
            &self.ctx.base_config,
            &self.ctx.scheme_registry,
            route_cancel.clone(),
        )?;
        // Guard the ingest handle BEFORE spawning push/WHEP (item 7): if
        // either spawn panics (a `Custom` output factory, say), the guard's
        // Drop aborts the ingest task too, so a half-built route cannot leak
        // a running supervisor. `push_cancel` is shared, so it is created
        // and installed before the spawns as well.
        let push_cancel = route_cancel.clone();
        let guarded = PendingRuntime::new(RouteRuntime {
            route: route.clone(),
            store,
            outputs,
            cancel: route_cancel,
            handle,
            push_handles: Vec::new(),
        });

        let mut push_handles = super::spawn_push_outputs(route, guarded.store(), &push_cancel);
        // Audit run 7, W6: the startup path (`serve_with_registry`) spawns
        // WHEP egress here too, but this admin path never did — so with the
        // admin API enabled (every startup route goes through `add_route`)
        // a route's `whep` output was silently never started. Spawn it, and
        // fold its handles into the same cancellation/abort group as push.
        push_handles.extend(super::spawn_whep_outputs(
            route,
            guarded.store(),
            &push_cancel,
            self.ctx.output_auth.clone(),
            &self.ctx.base_config,
        ));
        Ok(guarded.with_push_handles(push_handles).into_runtime())
    }

    /// `POST /admin/routes`: validates `route`, rejects a duplicate `name`
    /// with [`crate::MultimuxError::RouteExists`] (`409` — the existing
    /// route is left completely untouched), otherwise builds and spawns it
    /// and rebuilds the media router so it starts serving immediately.
    ///
    /// Serialised by [`Self::mutation_lock`] (audit run 7, W6) and — the
    /// plugin-factory point (W2) — `spawn_route`, which invokes a `Custom`
    /// input/output factory, runs with **no registry lock held**, so a
    /// panicking factory can neither poison `inner` into a permanent admin
    /// lockout nor block a concurrent reader.
    pub(crate) async fn add_route(&self, route: Route) -> crate::Result<RouteStatus> {
        route.validate_standalone(&self.ctx.base_config.playlist_name)?;
        let _guard = self.mutation_lock.lock().await;
        // Exact-name collision, plus a case-insensitive one (audit run 7,
        // A4): two names differing only in case share one DVR archive
        // directory on a case-insensitive filesystem, and the router nests
        // them under distinct URL segments, which is confusing and wrong.
        {
            let key = route.name.to_lowercase();
            let inner = self.inner.read();
            if inner.contains_key(&route.name) || inner.keys().any(|k| k.to_lowercase() == key) {
                return Err(crate::MultimuxError::RouteExists { name: route.name });
            }
        }
        let archive_floor = archive_floor_of(&route).await;
        let runtime = self.spawn_route(&route, archive_floor)?;
        let status = RouteStatus::from_runtime(&runtime);
        // The runtime is NOT installed yet. Wrap it so any early return or
        // unwind from here on cancels its push/WHEP tasks and aborts its
        // supervisor (audit run 7, A2) — a route that never reaches the
        // registry must not leave a dialer or a bound port behind.
        let pending = PendingRuntime::new(runtime);

        // Build-before-insert (audit run 7, W5/A1): `Router::merge`/`nest`
        // can PANIC (a `Custom` output mounting an overlapping path, e.g.),
        // and `rebuild_router` builds from whatever is in `inner`. Building
        // the prospective router — the current snapshot PLUS this route —
        // *before* inserting means a panic here happens with `inner`
        // untouched: nothing is registered, and the `PendingRuntime` guard
        // tears the half-built route down as it unwinds. Built inside
        // `catch_unwind` so the panic becomes a clean `500` for this one
        // request rather than poisoning anything or leaving a route that
        // every later rebuild also panics on.
        let prospective = {
            let mut streams = self.streams_snapshot();
            streams.insert(
                route.name.clone(),
                (
                    Arc::clone(&pending.runtime.as_ref().expect("armed").store),
                    pending.runtime.as_ref().expect("armed").outputs.clone(),
                ),
            );
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.build_router(streams)))
        };
        let new_router = match prospective {
            Ok(router) => router,
            Err(_) => {
                tracing::error!(
                    route = %route.name,
                    "building the router for a new route panicked; the route was not added"
                );
                return Err(crate::MultimuxError::ConfigInvalid {
                    field: "routes",
                    reason: format!(
                        "route {:?} could not be mounted (its outputs produce a conflicting  \
                        router path)",
                        route.name
                    ),
                });
            }
        };

        // Commit: install the router and the runtime together.
        let runtime = pending.into_installed();
        self.router_slot.store(std::sync::Arc::new(new_router));
        let displaced = {
            let mut guard = self.inner.write();
            guard.insert(route.name.clone(), runtime)
        };
        // A concurrent insert (or a reload) could not have raced us — the
        // mutation lock is held — so a displaced runtime here is defensive
        // only. Drain it anyway: dropping a `RouteRuntime` without draining
        // leaks its supervisor task and listen ports (W6).
        if let Some(old) = displaced {
            tracing::warn!(
                route = %route.name,
                "add_route displaced an existing runtime; draining it"
            );
            drain_route(old).await;
        }
        Ok(status)
    }

    /// `DELETE /admin/routes/{name}`: `404` (`RouteNotFound`) if unknown.
    /// Otherwise removes it from the registry and rebuilds the router
    /// *before* awaiting anything — new requests for `name` 404 immediately
    /// — then drains its supervisor task with no lock held (see this
    /// module's own "Concurrency" and "What an in-flight viewer sees" docs).
    pub(crate) async fn remove_route(&self, name: &str) -> crate::Result<()> {
        let _guard = self.mutation_lock.lock().await;
        let removed = {
            let mut guard = self.inner.write();
            guard.remove(name)
        };
        let Some(runtime) = removed else {
            return Err(crate::MultimuxError::RouteNotFound {
                name: name.to_string(),
            });
        };
        self.rebuild_router();
        drain_route(runtime).await;
        // After the drain: the supervisor's own last gauge update cannot race
        // this one back to `1` (audit r07-O5).
        super::supervisor::record_route_removed(name);
        Ok(())
    }

    /// `GET /admin/routes`.
    pub(crate) fn list_routes(&self) -> Vec<RouteStatus> {
        self.inner
            .read()
            .values()
            .map(RouteStatus::from_runtime)
            .collect()
    }

    /// `GET /admin/routes/{name}`.
    pub(crate) fn get_route(&self, name: &str) -> Option<RouteStatus> {
        self.inner.read().get(name).map(RouteStatus::from_runtime)
    }

    /// `POST /admin/reload`: re-reads [`Self::config_path`] and converges the
    /// live route set to it — added/removed/changed routes are applied,
    /// but **a route whose [`Route`] is byte-for-byte unchanged is never
    /// restarted** (compared via [`Route`]'s `PartialEq`, which walks the
    /// whole `input`/`outputs` tree) — this is the property that
    /// distinguishes reload from "restart with extra steps"; see
    /// [`RouteStatus::created_at_unix_nanos`], which stays identical for an
    /// unchanged route across a reload and changes for anything add/restart
    /// touches.
    ///
    /// Every route this reload would add or restart is validated **and**
    /// built (its `Output`s constructed, its supervisor spawned) before any
    /// of them are inserted into the live registry, or any currently-live
    /// route is removed — a malformed or unbuildable route anywhere in the
    /// file aborts the whole reload with no mutation at all. If a *later*
    /// route in the same batch fails to build after an *earlier* one in the
    /// same batch already spawned its supervisor task (that spawn cannot
    /// itself be undone — `tokio::spawn` doesn't roll back), every runtime
    /// built so far in this batch is torn down immediately (never inserted,
    /// so no request could ever have reached it) rather than left running
    /// detached.
    ///
    /// A reload is validated as a WHOLE by `Config::validate` (which rejects
    /// two route names differing only in case), so the `Cam1`/`cam1` pair a
    /// live registry could briefly hold mid-reload cannot arise from a
    /// reloaded file: either the file is rejected outright (leaving the
    /// existing registry untouched) or it contains no such pair. The live
    /// registry itself also rejects a case-insensitive duplicate at
    /// `add_route`.
    pub(crate) async fn reload(&self) -> crate::Result<ReloadSummary> {
        let Some(path) = self.config_path.clone() else {
            return Err(crate::MultimuxError::ConfigInvalid {
                field: "admin.reload",
                reason: "no config file path known for this process (config was not loaded via \
                         a `serve_config_file*` entry point)"
                    .into(),
            });
        };
        let new_config = Config::from_json_file(&path)?;
        let _guard = self.mutation_lock.lock().await;

        let current: HashMap<String, Route> = {
            self.inner
                .read()
                .iter()
                .map(|(name, rt)| (name.clone(), rt.route.clone()))
                .collect()
        };

        let mut added_routes: Vec<Route> = Vec::new();
        let mut changed_routes: Vec<Route> = Vec::new();
        let mut removed_names: Vec<String> = Vec::new();
        let mut unchanged_names: Vec<String> = Vec::new();

        let new_by_name: HashMap<&str, &Route> = new_config
            .routes
            .iter()
            .map(|r| (r.name.as_str(), r))
            .collect();
        for name in current.keys() {
            if !new_by_name.contains_key(name.as_str()) {
                removed_names.push(name.clone());
            }
        }
        for route in &new_config.routes {
            match current.get(&route.name) {
                None => added_routes.push(route.clone()),
                Some(existing) if existing == route => unchanged_names.push(route.name.clone()),
                Some(_) => changed_routes.push(route.clone()),
            }
        }

        // Validate every to-add/to-restart route BEFORE building any of
        // them (side-effect-free, so a failure here needs no rollback at
        // all).
        for route in added_routes.iter().chain(changed_routes.iter()) {
            route.validate_standalone(&self.ctx.base_config.playlist_name)?;
        }

        // Build (and spawn) every to-add/to-restart route, rolling back
        // (abort, no graceful drain needed -- nothing already spawned in
        // this batch was ever reachable by a request) anything already
        // built in this batch if a later one fails.
        // Each built runtime is held in a `PendingRuntime` guard, so an `Err`
        // from a later route in this batch (or an unwinding panic) drops the
        // earlier ones and, through their `Drop`, cancels their push/WHEP
        // tasks and aborts their supervisors — never leaving a dialer dialling
        // or a port bound for a route that was never installed (W6/A2).
        let mut prepared: Vec<(Route, PendingRuntime)> = Vec::new();
        for route in added_routes.iter().chain(changed_routes.iter()) {
            let archive_floor = archive_floor_of(route).await;
            let runtime = self.spawn_route(route, archive_floor)?;
            prepared.push((route.clone(), PendingRuntime::new(runtime)));
        }

        // Build the prospective router (current set minus removals/restarts,
        // plus the newly prepared routes) BEFORE mutating `inner`, so a
        // router-build panic leaves the live registry untouched (W5/A1).
        let prospective = {
            let mut streams = self.streams_snapshot();
            for name in removed_names
                .iter()
                .chain(changed_routes.iter().map(|r| &r.name))
            {
                streams.remove(name);
            }
            for (route, pending) in &prepared {
                let runtime = pending.runtime.as_ref().expect("armed");
                streams.insert(
                    route.name.clone(),
                    (Arc::clone(&runtime.store), runtime.outputs.clone()),
                );
            }
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.build_router(streams)))
        };
        let new_router = match prospective {
            Ok(router) => router,
            Err(_) => {
                tracing::error!(
                    "building the router for a reloaded configuration panicked; no changes applied"
                );
                return Err(crate::MultimuxError::ConfigInvalid {
                    field: "routes",
                    reason: "the reloaded routes could not be mounted (a conflicting router path)"
                        .into(),
                });
            }
        };

        let removed_runtimes: Vec<RouteRuntime> = {
            let mut guard = self.inner.write();
            let mut removed_runtimes = Vec::new();
            for name in removed_names
                .iter()
                .chain(changed_routes.iter().map(|r| &r.name))
            {
                if let Some(rt) = guard.remove(name) {
                    removed_runtimes.push(rt);
                }
            }
            for (route, pending) in prepared {
                guard.insert(route.name.clone(), pending.into_installed());
            }
            removed_runtimes
        };
        self.router_slot.store(std::sync::Arc::new(new_router));

        futures_util::future::join_all(removed_runtimes.into_iter().map(drain_route)).await;
        // Routes that are gone for good (a *changed* route was replaced by a
        // new runtime that reports its own state) must not stay `up`.
        for name in &removed_names {
            super::supervisor::record_route_removed(name);
        }

        Ok(ReloadSummary {
            added: added_routes.into_iter().map(|r| r.name).collect(),
            removed: removed_names,
            changed: changed_routes.into_iter().map(|r| r.name).collect(),
            unchanged: unchanged_names,
        })
    }

    /// Tears down every currently-registered route's supervisor task, in
    /// orderly fashion (mirrors `serve_with_registry`'s own final loop) —
    /// called once, from [`serve_with_admin`]'s own shutdown path.
    async fn shutdown_all(&self) {
        let _guard = self.mutation_lock.lock().await;
        let all: Vec<RouteRuntime> = {
            let mut guard = self.inner.write();
            guard.drain().map(|(_, rt)| rt).collect()
        };
        futures_util::future::join_all(all.into_iter().map(drain_route)).await;
    }
}

/// Signals `runtime`'s supervisor task to stop and waits (bounded by
/// [`super::SUPERVISOR_SHUTDOWN_GRACE`]) for it to return on its own,
/// aborting it if it doesn't — the same bounded-drain shape
/// `serve_with_registry`'s final loop applies to every route at process
/// shutdown, applied here to exactly one route at a time so
/// [`RouteRegistry::remove_route`]/[`RouteRegistry::reload`] never hold a
/// lock across it (see this module's own "Concurrency" docs).
async fn drain_route(runtime: RouteRuntime) {
    runtime.cancel.cancel();
    let abort_handle = runtime.handle.abort_handle();
    if tokio::time::timeout(SUPERVISOR_SHUTDOWN_GRACE, runtime.handle)
        .await
        .is_err()
    {
        tracing::warn!(
            "admin: route supervisor task did not exit within the shutdown grace period; \
             aborting"
        );
        abort_handle.abort();
    }
    for h in runtime.push_handles {
        h.abort();
    }
}

/// One route's admin-visible status — `GET /admin/routes`/`GET
/// /admin/routes/{name}`'s response body. Reuses [`OutputKind`] (already
/// `Serialize`) verbatim rather than inventing a second output-kind
/// encoding; never includes the raw [`Route`] (which may embed
/// `InputSpec`/`AuthSpec` credentials — `Route`/`InputSpec`/`AuthSpec` are
/// deliberately `Deserialize`-only, never `Serialize`, so this can't
/// accidentally leak one).
#[derive(Debug, serde::Serialize)]
pub struct RouteStatus {
    /// Served stream name.
    pub name: String,
    /// A stable, lowercase token naming [`Route::input`]'s kind (`"rtsp"`,
    /// `"rtp"`, `"ts_udp"`, `"ts_http"`, `"srt"`, `"hls_pull"`,
    /// `"dash_pull"`, `"smooth_pull"`, `"rtmp"`, or `"custom"`) — never the
    /// input's own fields (which may carry a credential).
    pub input_kind: &'static str,
    /// This route's configured outputs.
    pub outputs: Vec<OutputKind>,
    /// This route's current [`crate::route::HealthState`], as its
    /// [`crate::route::RouteHandle::health`] label
    /// ([`crate::route::HealthState::name`]).
    pub health: String,
    /// Unix timestamp (**nanoseconds**) this route's [`RouteHandle`] was
    /// constructed — [`crate::route::RouteHandle::created_at`]. The proof
    /// that `RouteRegistry::reload` left an *unchanged* route's serving
    /// state completely alone: it is the same `RouteHandle`, so this value
    /// is identical before and after the reload, while an added or
    /// restarted route gets a new, later one. Nanosecond (not
    /// whole-second) resolution deliberately: two `RouteHandle`s built
    /// microseconds apart (as an add-then-reload-immediately test would)
    /// must compare unequal, not coincidentally round to the same second.
    pub created_at_unix_nanos: u128,
}

impl RouteStatus {
    fn from_runtime(rt: &RouteRuntime) -> Self {
        RouteStatus {
            name: rt.route.name.clone(),
            input_kind: input_kind_name(&rt.route.input),
            outputs: rt.route.outputs.clone(),
            health: rt.store.health().name().to_string(),
            created_at_unix_nanos: rt
                .store
                .created_at()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        }
    }
}

fn input_kind_name(input: &crate::config::InputSpec) -> &'static str {
    use crate::config::InputSpec::*;
    match input {
        Rtsp { .. } => "rtsp",
        Rtp { .. } => "rtp",
        TsUdp { .. } => "ts_udp",
        TsHttp { .. } => "ts_http",
        HlsPull { .. } => "hls_pull",
        DashPull { .. } => "dash_pull",
        SmoothPull { .. } => "smooth_pull",
        Rtmp { .. } => "rtmp",
        #[cfg(feature = "whip")]
        Whip { .. } => "whip",
        Srt { .. } => "srt",
        Custom { .. } => "custom",
        File { .. } => "file",
    }
}

/// `POST /admin/reload`'s response body — which route names were added,
/// removed, changed (same name, different config — restarted), or left
/// completely alone.
#[derive(Debug, serde::Serialize)]
pub struct ReloadSummary {
    /// Route names present in the reloaded file but not the running set.
    pub added: Vec<String>,
    /// Route names present in the running set but not the reloaded file.
    pub removed: Vec<String>,
    /// Route names present in both, with a different [`Route`] config —
    /// restarted (old instance drained, new one spawned).
    pub changed: Vec<String>,
    /// Route names present in both with byte-for-byte identical [`Route`]
    /// config — **left running, never restarted**.
    pub unchanged: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct ErrorBody {
    error: String,
}

fn error_response(status: StatusCode, message: String) -> Response {
    (status, Json(ErrorBody { error: message })).into_response()
}

fn status_for_error(e: &crate::MultimuxError) -> StatusCode {
    match e {
        crate::MultimuxError::RouteExists { .. } => StatusCode::CONFLICT,
        crate::MultimuxError::RouteNotFound { .. } => StatusCode::NOT_FOUND,
        crate::MultimuxError::ConfigInvalid { .. } | crate::MultimuxError::UnknownScheme { .. } => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Clone)]
struct AdminState {
    registry: Arc<RouteRegistry>,
}

async fn list_routes_handler(State(state): State<AdminState>) -> Response {
    Json(state.registry.list_routes()).into_response()
}

async fn get_route_handler(
    State(state): State<AdminState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    match state.registry.get_route(&name) {
        Some(status) => Json(status).into_response(),
        None => error_response(StatusCode::NOT_FOUND, format!("no such route {name:?}")),
    }
}

async fn add_route_handler(
    State(state): State<AdminState>,
    body: Result<Json<Route>, axum::extract::rejection::JsonRejection>,
) -> Response {
    // Force every `Json<Route>` rejection to `400` ourselves, rather than
    // axum's default `JsonRejection::into_response` (which varies by *why*
    // the body didn't parse: `422` for a data/shape mismatch, `415` for a
    // missing/wrong `Content-Type`, `400` only for outright invalid JSON
    // syntax) — issue #749's contract is one status for "the body is not a
    // usable `Route`": `400`, regardless of which of those three it was.
    let route = match body {
        Ok(Json(route)) => route,
        Err(rejection) => return error_response(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    match state.registry.add_route(route).await {
        Ok(status) => (StatusCode::CREATED, Json(status)).into_response(),
        Err(e) => error_response(status_for_error(&e), e.to_string()),
    }
}

async fn delete_route_handler(
    State(state): State<AdminState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    match state.registry.remove_route(&name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(status_for_error(&e), e.to_string()),
    }
}

async fn reload_handler(State(state): State<AdminState>) -> Response {
    match state.registry.reload().await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => error_response(status_for_error(&e), e.to_string()),
    }
}

/// Mandatory admin-auth gate — unlike [`super::output_auth_gate`] (which
/// no-ops when [`crate::config::Config::output_auth`] is `None`), this is
/// unconditional: reaching this middleware at all already means
/// [`crate::config::AdminSpec::auth`] resolved to a real [`Verifier`] (see
/// [`crate::config::AdminSpec`]'s own docs on why that field can't be
/// missing), so every admin request is checked, no exceptions.
async fn admin_auth_gate(
    State(verifier): State<Arc<Verifier>>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().as_str().to_string();
    let uri = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let headers: Vec<(&str, &str)> = req
        .headers()
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str(), v)))
        .collect();
    let peer_addr = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    let mut ctx = broadcast_auth::RequestContext::new(&method, &uri).with_headers(&headers);
    if let Some(peer_addr) = peer_addr {
        ctx = ctx.with_peer_addr(peer_addr);
    }
    match verifier.verify(&ctx) {
        AuthResult::Ok => next.run(req).await,
        _ => {
            let mut resp = StatusCode::UNAUTHORIZED.into_response();
            // `challenge_for` (not `challenge`): an expired nonce must be
            // answered with its own fresh nonce carrying `stale=true` (RFC
            // 7616 §3.3) — see `super::output_auth_gate`'s identical call.
            if let Ok(value) = HeaderValue::from_str(&verifier.challenge_for(&ctx)) {
                resp.headers_mut().insert(header::WWW_AUTHENTICATE, value);
            }
            resp
        }
    }
}

/// Builds the admin API's own (static — its shape never changes at runtime,
/// unlike the media router) [`Router`]: the five endpoints in this module's
/// own docs, gated whole behind `verifier`.
pub(crate) fn admin_router(registry: Arc<RouteRegistry>, verifier: Arc<Verifier>) -> Router {
    Router::new()
        .route(
            "/admin/routes",
            get(list_routes_handler).post(add_route_handler),
        )
        .route(
            "/admin/routes/{name}",
            get(get_route_handler).delete(delete_route_handler),
        )
        .route("/admin/reload", post(reload_handler))
        .with_state(AdminState { registry })
        .layer(middleware::from_fn_with_state(verifier, admin_auth_gate))
}

/// Per-connection media service: reads the *current* router snapshot
/// ([`RouteRegistry::current_router`]) on every request rather than once at
/// startup, so a route added/removed after this connection was accepted is
/// still reflected — see this module's own "Concurrency" docs. Holds the
/// address so `ConnectInfo<SocketAddr>` can still be inserted into every
/// request.
#[derive(Clone)]
struct DynamicMediaService {
    registry: Arc<RouteRegistry>,
    remote_addr: SocketAddr,
}

impl<B> Service<Request<B>> for DynamicMediaService
where
    B: axum::body::HttpBody<Data = bytes::Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, std::convert::Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        // Mirrors what `Router::into_make_service_with_connect_info` does
        // for the static path (`super::serve_with_registry`) — inserted
        // directly (rather than via `axum::Extension`'s `tower::Layer`,
        // whose produced service type isn't nameable outside axum itself)
        // since `ConnectInfo<T>`'s own `FromRequestParts` impl reads it from
        // the request extensions either way.
        req.extensions_mut().insert(ConnectInfo(self.remote_addr));
        let mut router = self.registry.current_router();
        Box::pin(async move { router.call(req).await })
    }
}

/// Run the multimux origin with the runtime admin API enabled — reached only
/// from [`super::serve_with_registry_impl`] when
/// [`crate::config::Config::admin`] is `Some`. Binds **two** listeners:
/// [`crate::config::Config::bind`] (media, served over a router rebuilt on
/// every admin mutation — see [`DynamicMediaService`]) and
/// [`crate::config::AdminSpec::bind`] (admin, a plain static [`Router`] —
/// see [`admin_router`]), and installs the same graceful-shutdown handling
/// [`super::serve_with_registry`] does for both, plus tearing down every
/// still-registered route's supervisor task via [`RouteRegistry::shutdown_all`].
pub(crate) async fn serve_with_admin(
    config: Config,
    scheme_registry: SchemeRegistry,
    config_path: Option<PathBuf>,
    prebound: Option<super::PreboundListeners>,
) -> crate::Result<()> {
    let (prebound_media, prebound_admin_listener) = match prebound {
        Some(p) => (Some(p.media), p.admin),
        None => (None, None),
    };
    let admin_spec = config
        .admin
        .clone()
        .expect("serve_with_admin is only ever called when config.admin.is_some()");

    tracing::info!(
        bind = %config.bind,
        admin_bind = %admin_spec.bind,
        routes = config.routes.len(),
        "multimux origin starting (runtime admin API enabled)"
    );

    // Resolve both verifiers before binding anything -- an unresolvable
    // `Custom` admin/output-auth scheme must fail startup, not open either
    // listener half-configured.
    let admin_verifier = Arc::new(super::resolve_verifier(
        &admin_spec.auth,
        ADMIN_AUTH_REALM,
        &scheme_registry,
    )?);
    let output_auth = match &config.output_auth {
        Some(spec) => Some(Arc::new(super::resolve_verifier(
            spec,
            super::OUTPUT_AUTH_REALM,
            &scheme_registry,
        )?)),
        None => None,
    };

    let http_limits = HttpLimits::from(&config);
    let base_config = Config {
        routes: Vec::new(),
        admin: None,
        ..config.clone()
    };
    let ctx = RegistryContext {
        base_config,
        scheme_registry: scheme_registry.clone(),
        http_limits,
        output_auth,
    };

    let registry = RouteRegistry::new(ctx, config_path);
    // Build every startup route exactly like a live `POST /admin/routes`
    // would (`config.validate()`, called by our one caller before
    // dispatching here, already rejected duplicate names, so `RouteExists`
    // can never trigger in this loop). A failure here returns `Err` and the
    // process exits before serving anything -- unlike `RouteRegistry::reload`,
    // no rollback of earlier-in-this-loop routes is needed: the whole
    // process is about to exit, not keep running with an orphaned task.
    for route in &config.routes {
        registry.add_route(route.clone()).await?;
    }

    let media_listener = match prebound_media {
        Some(listener) => listener,
        None => tokio::net::TcpListener::bind(config.bind.as_str()).await?,
    };
    let admin_listener = match prebound_admin_listener {
        Some(listener) => listener,
        None => tokio::net::TcpListener::bind(admin_spec.bind.as_str()).await?,
    };

    let (external_shutdown_tx, external_shutdown_rx) = watch::channel(false);
    let shutdown_task = tokio::spawn(async move {
        super::shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        let _ = external_shutdown_tx.send(true);
    });

    // One shutdown token, cancelled when the watcher flips the channel — the
    // same event both listeners used to race with `with_graceful_shutdown`.
    let shutdown_token = tokio_util::sync::CancellationToken::new();
    let admin_token = shutdown_token.clone();
    let media_token = shutdown_token.clone();
    let mut shutdown_rx = external_shutdown_rx.clone();
    let shutdown_watch = tokio::spawn(async move {
        let _ = shutdown_rx.changed().await;
        shutdown_token.cancel();
    });

    let admin_built = admin_router(Arc::clone(&registry), admin_verifier);
    let admin_task: tokio::task::JoinHandle<std::io::Result<()>> = tokio::spawn(async move {
        super::serve_hyper_util(admin_listener, admin_built, admin_token).await
    });

    let media_registry = Arc::clone(&registry);
    let media_result = super::serve_hyper_util_service(
        media_listener,
        move |addr| {
            let registry = Arc::clone(&media_registry);
            async move {
                DynamicMediaService {
                    registry,
                    remote_addr: addr,
                }
            }
        },
        media_token,
    )
    .await;

    // The media server has stopped (shutdown fired, or a fatal accept-loop
    // error) -- make sure the Ctrl-C/SIGTERM watcher tasks don't outlive us.
    shutdown_task.abort();
    shutdown_watch.abort();

    // Join the admin server the same bounded way every route's supervisor
    // is joined below — grab the abort handle before consuming `admin_task`
    // in the timeout so a wedged admin server can still be aborted rather
    // than left running detached.
    let admin_abort_handle = admin_task.abort_handle();
    match tokio::time::timeout(SUPERVISOR_SHUTDOWN_GRACE, admin_task).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => tracing::warn!(error = %e, "admin HTTP server exited with an error"),
        Ok(Err(e)) => tracing::warn!(error = %e, "admin HTTP server task panicked"),
        Err(_) => {
            tracing::warn!(
                "admin HTTP server did not exit within the shutdown grace period; aborting"
            );
            admin_abort_handle.abort();
        }
    }

    registry.shutdown_all().await;

    media_result?;
    Ok(())
}

/// The highest sequence number `route`'s DVR archive already holds, or `0`
/// when DVR is off (see `crate::dvr::archive_seq_floor`).
pub(super) async fn archive_floor_of(route: &Route) -> u32 {
    if route.dvr.enabled {
        crate::dvr::archive_seq_floor(&route.dvr, &route.name).await
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InputSpec;
    use crate::dvr::DvrConfig;

    fn ctx_for_tests() -> RegistryContext {
        RegistryContext {
            base_config: Config::default(),
            scheme_registry: SchemeRegistry::new(),
            http_limits: HttpLimits::default(),
            output_auth: None,
        }
    }

    /// A route carrying a `whep` output must not panic `spawn_route`.
    ///
    /// `build_output` has an `unreachable!()` for `OutputKind::Whep` (WHEP is
    /// driven by its own listen socket, not an `Arc<dyn Output>`), and
    /// `add_route` calls `spawn_route` while holding the registry WRITE lock.
    /// So the unwind poisoned the lock and every subsequent admin operation
    /// and router rebuild panicked on `.expect("...poisoned")` — one
    /// authenticated request, permanent admin lockout for the process
    /// lifetime. The startup path filtered `is_whep()`; this one did not.
    #[cfg(feature = "whep")]
    #[tokio::test]
    async fn adding_a_route_with_a_whep_output_does_not_poison_the_registry() {
        let registry = RouteRegistry::new(ctx_for_tests(), None);
        let mut route = rtsp_route("cam", "rtsp://127.0.0.1:554/s");
        route.outputs.push(OutputKind::Whep {
            listen: "127.0.0.1:0".to_string(),
        });

        // Must not panic. Whether the spawn itself succeeds is not the point.
        let _ = registry.add_route(route).await;

        // The lock must still be usable — this is what the poisoning broke.
        // The lock must still be usable — poisoning is what bricked this.
        let _routes = registry.list_routes();
    }

    fn rtsp_route(name: &str, url: &str) -> Route {
        Route {
            name: name.to_string(),
            input: InputSpec::Rtsp {
                url: url.to_string(),
                auth: None,
            },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
        }
    }

    /// MUTATION VERIFIED: changing `add_route` to skip the
    /// `guard.contains_key` check (always insert, overwriting any existing
    /// entry) makes this test fail: the second `add_route` call for `"cam1"`
    /// returns `Ok(..)` instead of `Err(RouteExists { .. })`, and
    /// `list_routes().len()` is `1` instead of the still-registered original
    /// -- both assertions below fail. Recompiled and re-run to confirm, then
    /// reverted.
    #[tokio::test]
    async fn add_duplicate_name_is_conflict_and_original_is_untouched() {
        let registry = RouteRegistry::new(ctx_for_tests(), None);
        let first = registry
            .add_route(rtsp_route("cam1", "rtsp://host/a"))
            .await
            .expect("first add succeeds");

        let err = registry
            .add_route(rtsp_route("cam1", "rtsp://host/b"))
            .await
            .expect_err("duplicate name must be rejected");
        assert!(matches!(err, crate::MultimuxError::RouteExists { name } if name == "cam1"));

        let still_there = registry.get_route("cam1").expect("route still registered");
        assert_eq!(
            still_there.created_at_unix_nanos, first.created_at_unix_nanos,
            "the original route's RouteHandle must not have been rebuilt"
        );
    }

    /// MUTATION VERIFIED: changing `remove_route` to return `Ok(())` for an
    /// unknown name instead of `RouteNotFound` makes this test's
    /// `expect_err` panic (`Ok(())` where an `Err` was expected). Recompiled
    /// and re-run to confirm, then reverted.
    #[tokio::test]
    async fn remove_unknown_route_is_not_found() {
        let registry = RouteRegistry::new(ctx_for_tests(), None);
        let err = registry
            .remove_route("nope")
            .await
            .expect_err("unknown route must 404-map");
        assert!(matches!(err, crate::MultimuxError::RouteNotFound { name } if name == "nope"));
    }

    /// MUTATION VERIFIED: changing `add_route`'s validation call from
    /// `route.validate_standalone()?` to always `Ok(())` (skip validation
    /// entirely) makes this test fail: `add_route` returns `Ok(..)` for an
    /// empty-outputs route instead of `Err(ConfigInvalid { .. })`.
    /// Recompiled and re-run to confirm, then reverted.
    #[tokio::test]
    async fn add_route_rejects_invalid_route_before_mutating() {
        let registry = RouteRegistry::new(ctx_for_tests(), None);
        let bad = Route {
            name: "cam1".to_string(),
            input: InputSpec::Rtsp {
                url: "rtsp://host/a".to_string(),
                auth: None,
            },
            outputs: Vec::new(),
            dvr: DvrConfig::default(),
        };
        let err = registry
            .add_route(bad)
            .await
            .expect_err("empty outputs must be rejected");
        assert!(matches!(err, crate::MultimuxError::ConfigInvalid { .. }));
        assert!(
            registry.list_routes().is_empty(),
            "a rejected add must leave the registry empty"
        );
    }
}
