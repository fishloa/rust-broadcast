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
use axum::body::Body;
use axum::extract::{ConnectInfo, Path as AxumPath, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
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
    /// Assigned from [`RouteRegistry::next_generation`] when the runtime is spawned, so a
    /// reload can tell that the runtime registered under a name is a different one from the
    /// one it planned against.
    generation: u64,
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
/// (audit run 7, W6/A2). [`Self::into_installed`] disarms the rollback and
/// hands the runtime over, so an installed runtime is *not* torn down on
/// drop; the registry owns it from then on and tears it down via
/// [`drain_route`].
///
/// The runtime is held unconditionally (never behind an `Option`) and the
/// rollback lives in a separate [`RollbackGuard`] that owns only cancel/abort
/// handles, so `PendingRuntime` itself has no `Drop` and
/// [`Self::into_installed`] can consume it by value: "installed twice" and
/// "accessed after install" are unrepresentable (audit P2).
struct PendingRuntime {
    runtime: RouteRuntime,
    rollback: RollbackGuard,
}

/// The teardown half of a [`PendingRuntime`]: cancels the route's token and
/// aborts every task it spawned unless [`Self::disarm`]ed first.
struct RollbackGuard {
    cancel: tokio_util::sync::CancellationToken,
    aborts: Vec<tokio::task::AbortHandle>,
    armed: bool,
}

impl RollbackGuard {
    /// Consume the guard without tearing anything down.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for RollbackGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.cancel.cancel();
        for h in &self.aborts {
            h.abort();
        }
    }
}

impl PendingRuntime {
    fn new(runtime: RouteRuntime) -> Self {
        let mut aborts = vec![runtime.handle.abort_handle()];
        aborts.extend(runtime.push_handles.iter().map(|h| h.abort_handle()));
        PendingRuntime {
            rollback: RollbackGuard {
                cancel: runtime.cancel.clone(),
                aborts,
                armed: true,
            },
            runtime,
        }
    }

    /// The guarded runtime's store — for spawning push/WHEP outputs against
    /// it *before* the runtime is fully assembled (item 7).
    fn store(&self) -> Arc<RouteHandle> {
        Arc::clone(&self.runtime.store)
    }

    /// Attach the push/WHEP handles spawned against this guard's store.
    fn with_push_handles(mut self, handles: Vec<tokio::task::JoinHandle<()>>) -> Self {
        self.rollback
            .aborts
            .extend(handles.iter().map(|h| h.abort_handle()));
        self.runtime.push_handles.extend(handles);
        self
    }

    /// Take the assembled runtime out (disarming the rollback) — used by
    /// [`RouteRegistry::spawn_route`], whose result the caller re-wraps.
    fn into_runtime(self) -> RouteRuntime {
        self.into_installed()
    }

    /// Take the runtime out for installation, disarming the rollback.
    fn into_installed(self) -> RouteRuntime {
        let PendingRuntime { runtime, rollback } = self;
        rollback.disarm();
        runtime
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

/// A media [`Router`] and, for each route it serves, the token that fires when that route is
/// replaced or removed (see [`crate::route::RouteHandle::retire`]).
struct LiveRouter {
    router: Router,
    retire: HashMap<String, tokio_util::sync::CancellationToken>,
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
    /// Source of [`RouteRuntime::generation`].
    next_generation: std::sync::atomic::AtomicU64,
    /// One lock per route name (lowercased, the same equivalence `add_route`
    /// uses for collisions), held for a route's whole add / remove / reload
    /// step **including** the drain of a displaced runtime and the archive
    /// scan. That ordering is what makes a route's archive sequence floor safe
    /// to compute outside [`Self::mutation_lock`]: an add of `X` scans the
    /// archive only after any earlier remove of `X` has fully drained (so the
    /// old runtime can write no more segments), and a slow scan of `X` holds
    /// `X`'s lock alone, never the registry-wide one. Always taken in sorted
    /// name order and before `mutation_lock`.
    name_locks: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    #[cfg(test)]
    hooks: TestHooks,
    inner: parking_lot::RwLock<HashMap<String, RouteRuntime>>,
    /// The currently-active media [`Router`], rebuilt whole on every
    /// mutation (see this module's own docs) and read by
    /// [`DynamicMediaService`] once per accepted request. `ArcSwap` makes
    /// that read lock-free: a rebuild never blocks a request already in
    /// flight, and a request never takes a lock the rebuild path wants.
    router_slot: arc_swap::ArcSwap<LiveRouter>,
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
            next_generation: std::sync::atomic::AtomicU64::new(0),
            name_locks: parking_lot::Mutex::new(HashMap::new()),
            #[cfg(test)]
            hooks: TestHooks::default(),
            inner: parking_lot::RwLock::new(HashMap::new()),
            router_slot: arc_swap::ArcSwap::from_pointee(LiveRouter {
                router: Router::new(),
                retire: HashMap::new(),
            }),
            config_path,
        });
        registry.rebuild_router();
        registry
    }

    /// Take the per-name lock of every name in `names`, in sorted order.
    async fn lock_names<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut keys: Vec<String> = names.into_iter().map(str::to_lowercase).collect();
        keys.sort_unstable();
        keys.dedup();
        let locks: Vec<Arc<tokio::sync::Mutex<()>>> = {
            let mut table = self.name_locks.lock();
            table.retain(|_, lock| Arc::strong_count(lock) > 1);
            keys.iter()
                .map(|key| Arc::clone(table.entry(key.clone()).or_default()))
                .collect()
        };
        let mut held = Vec::with_capacity(locks.len());
        for lock in locks {
            held.push(lock.lock_owned().await);
        }
        held
    }

    /// The highest sequence number `route`'s DVR archive already holds. Callers
    /// hold the route's name lock, not [`Self::mutation_lock`].
    async fn archive_floor(&self, route: &Route) -> u32 {
        #[cfg(test)]
        self.hooks.wait_floor_gate(&route.name).await;
        let floor = archive_floor_of(route).await;
        #[cfg(test)]
        self.hooks.log(format!("floor:{}", route.name));
        floor
    }

    fn check_name_free(&self, route: &Route) -> crate::Result<()> {
        // Exact-name collision, plus a case-insensitive one (audit run 7,
        // A4): two names differing only in case share one DVR archive
        // directory on a case-insensitive filesystem, and the router nests
        // them under distinct URL segments, which is confusing and wrong.
        let key = route.name.to_lowercase();
        let inner = self.inner.read();
        if inner.contains_key(&route.name) || inner.keys().any(|k| k.to_lowercase() == key) {
            return Err(crate::MultimuxError::RouteExists {
                name: route.name.clone(),
            });
        }
        Ok(())
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
    fn build_router(&self, streams: HashMap<String, StreamRoute>) -> LiveRouter {
        let retire = streams
            .iter()
            .map(|(name, (store, _))| (name.clone(), store.retired_token()))
            .collect();
        let mut app_state = AppState::new(streams).with_limits(self.ctx.http_limits);
        if let Some(verifier) = &self.ctx.output_auth {
            app_state = app_state.with_output_auth(Arc::clone(verifier));
        }
        // The concurrency bound (and the ops/reload budgets) is applied
        // inside `router()` itself, so it is present here too.
        LiveRouter {
            router: router(Arc::new(app_state)),
            retire,
        }
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

    /// The active router together with the retire tokens of the routes it serves, read as one
    /// snapshot (so a request never pairs a router with the token of a different route set).
    fn live_router(&self) -> Arc<LiveRouter> {
        self.router_slot.load_full()
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
            .with_ts_cut_policy(route.ts_hls_cut.into())
            .with_ts_idr_fallback(route.ts_hls_idr_fallback)
            // Issue #900 — see the identical fix (and its own doc) in
            // `super::serve_with_registry_impl`'s per-route loop; this is
            // the admin-API (`POST /admin/routes`/reload) equivalent.
            .with_dvr(route.dvr.clone())
            .with_program_date_time(self.ctx.base_config.program_date_time)
            .with_hold_cold_playlist(self.ctx.base_config.hold_cold_playlist)
            .with_declared_bandwidth_bps(route.declared_bandwidth_bps)
            .with_audio_renditions(route.audio_renditions.clone())
            .with_closed_caption_overrides(route.closed_caption_overrides.clone())
            .with_audio_rendition_sources(route.audio_rendition_sources.clone())
            .with_audio_profiles(route.audio_profiles.clone())
            .with_subtitles(route.subtitles),
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
            generation: self
                .next_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
        let _name = self.lock_names([route.name.as_str()]).await;
        self.check_name_free(&route)?;
        let archive_floor = self.archive_floor(&route).await;
        let _guard = self.mutation_lock.lock().await;
        self.check_name_free(&route)?;
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
                    Arc::clone(&pending.runtime.store),
                    pending.runtime.outputs.clone(),
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
        let _name = self.lock_names([name]).await;
        let runtime = {
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
            runtime.store.retire();
            runtime
        };
        // The registry-wide lock is released; the name lock is held until the
        // drain finishes, so an add of this name cannot read the archive floor
        // while the old runtime can still write to the archive.
        #[cfg(test)]
        self.hooks.wait_drain_gate(name).await;
        drain_route(runtime).await;
        // After the drain: the supervisor's own last gauge update cannot race
        // this one back to `1` (audit r07-O5).
        super::supervisor::record_route_removed(name);
        #[cfg(test)]
        self.hooks.log(format!("drained:{name}"));
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
        let (_names, global, plan, added_floors) = loop {
            let plan = self.plan_reload(&new_config);
            let names = self
                .lock_names(plan.touched().iter().map(String::as_str))
                .await;
            let mut added_floors = Vec::with_capacity(plan.added.len());
            for route in &plan.added {
                added_floors.push(self.archive_floor(route).await);
            }
            let global = self.mutation_lock.lock().await;
            // The registry may have changed while the locks were being taken: a route this
            // reload did not lock was added or removed, or a route it did plan was added,
            // replaced or removed (each runtime carries a generation, so a name whose
            // runtime is a different one now is noticed even when the set of names is the
            // same). Plan again against what is there now.
            if self.plan_reload(&new_config).state == plan.state {
                break (names, global, plan, added_floors);
            }
        };
        let ReloadPlan {
            added: added_routes,
            changed: changed_routes,
            removed: removed_names,
            unchanged: unchanged_names,
            ..
        } = plan;

        // Validate every to-add/to-restart route BEFORE building any of
        // them (side-effect-free, so a failure here needs no rollback at
        // all).
        for route in added_routes.iter().chain(changed_routes.iter()) {
            route.validate_standalone(&self.ctx.base_config.playlist_name)?;
        }

        // A changed route's old runtime is drained BEFORE its archive floor is read and its
        // replacement is spawned: until the old runtime has stopped it can still write
        // segments, and a floor read (or a replacement started) earlier could number below
        // them. The retired routes are kept so that a failure further down puts them back.
        let mut retired: Vec<(Route, RouteRuntime)> = Vec::new();
        {
            let mut guard = self.inner.write();
            for route in &changed_routes {
                if let Some(runtime) = guard.remove(&route.name) {
                    retired.push((runtime.route.clone(), runtime));
                }
            }
        }
        let (retired_routes, retired_runtimes): (Vec<Route>, Vec<RouteRuntime>) =
            retired.into_iter().unzip();
        let retired_stores: Vec<Arc<RouteHandle>> = retired_runtimes
            .iter()
            .map(|runtime| Arc::clone(&runtime.store))
            .collect();
        futures_util::future::join_all(retired_runtimes.into_iter().map(drain_route)).await;
        let mut floors = added_floors;
        for (route, old_store) in changed_routes.iter().zip(&retired_stores) {
            // Above what the old runtime showed as well as above the archive, so a client's
            // `_HLS_msn` for this route keeps its meaning on the replacement.
            floors.push(
                self.archive_floor(route)
                    .await
                    .max(old_store.media_sequence_floor()),
            );
        }

        // Build (and spawn) every to-add/to-restart route. Each built runtime is held in a
        // `PendingRuntime` guard, so an `Err` from a later route in this batch (or an
        // unwinding panic) drops the earlier ones and, through their `Drop`, cancels their
        // push/WHEP tasks and aborts their supervisors. The changed routes' old runtimes are
        // already drained, so on a failure they are started again from their old
        // configuration (`restore_retired`): a failed reload leaves the same routes running.
        let mut prepared: Vec<(Route, PendingRuntime)> = Vec::new();
        for (route, archive_floor) in added_routes.iter().chain(changed_routes.iter()).zip(floors) {
            match self.spawn_route(route, archive_floor) {
                Ok(runtime) => prepared.push((route.clone(), PendingRuntime::new(runtime))),
                Err(e) => {
                    drop(prepared);
                    self.restore_retired(&retired_routes, &retired_stores).await;
                    return Err(e);
                }
            }
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
                let runtime = &pending.runtime;
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
                drop(prepared);
                self.restore_retired(&retired_routes, &retired_stores).await;
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
        // Every request parked on a store this reload replaced or removed is woken and served
        // again by the router that is live now (the replacement, or a 404 when the name is gone).
        for store in retired_stores
            .iter()
            .chain(removed_runtimes.iter().map(|runtime| &runtime.store))
        {
            store.retire();
        }
        drop(global);

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

    /// What reloading `new_config` would add, restart, remove and leave alone,
    /// against the registry as it is now.
    fn plan_reload(&self, new_config: &Config) -> ReloadPlan {
        let (current, generations): (
            std::collections::BTreeMap<String, Route>,
            HashMap<String, u64>,
        ) = {
            let inner = self.inner.read();
            (
                inner
                    .iter()
                    .map(|(name, rt)| (name.clone(), rt.route.clone()))
                    .collect(),
                inner
                    .iter()
                    .map(|(name, rt)| (name.clone(), rt.generation))
                    .collect(),
            )
        };
        let new_by_name: HashMap<&str, &Route> = new_config
            .routes
            .iter()
            .map(|r| (r.name.as_str(), r))
            .collect();
        let mut plan = ReloadPlan::default();
        for name in current.keys() {
            if !new_by_name.contains_key(name.as_str()) {
                plan.removed.push(name.clone());
            }
        }
        for route in &new_config.routes {
            match current.get(&route.name) {
                None => plan.added.push(route.clone()),
                Some(existing) if existing == route => plan.unchanged.push(route.name.clone()),
                Some(_) => plan.changed.push(route.clone()),
            }
        }
        plan.state = plan
            .touched()
            .into_iter()
            .map(|name| {
                let generation = generations.get(&name).copied();
                (name, generation)
            })
            .collect();
        plan
    }

    /// Starts again, from their old configuration, the routes a failed reload had already
    /// stopped, and rebuilds the router around them. A route that cannot be started again is
    /// logged and left out of the registry.
    async fn restore_retired(&self, routes: &[Route], stores: &[Arc<RouteHandle>]) {
        for route in routes {
            let floor = archive_floor_of(route).await;
            match self.spawn_route(route, floor) {
                Ok(runtime) => {
                    self.inner.write().insert(
                        route.name.clone(),
                        PendingRuntime::new(runtime).into_installed(),
                    );
                }
                Err(e) => tracing::error!(
                    route = %route.name,
                    error = %e,
                    "admin: a failed reload could not start a stopped route again"
                ),
            }
        }
        self.rebuild_router();
        for store in stores {
            store.retire();
        }
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

/// The difference between the live registry and a reloaded configuration.
#[derive(Default)]
struct ReloadPlan {
    added: Vec<Route>,
    changed: Vec<Route>,
    removed: Vec<String>,
    unchanged: Vec<String>,
    /// Each touched name with the generation of the runtime registered under it when the
    /// plan was made (`None` when none was): two plans over the same names are the same plan
    /// only if every one of them is the same runtime.
    state: Vec<(String, Option<u64>)>,
}

impl ReloadPlan {
    /// The names a reload of this plan changes, sorted: the ones whose locks it
    /// needs.
    fn touched(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .added
            .iter()
            .chain(self.changed.iter())
            .map(|route| route.name.clone())
            .chain(self.removed.iter().cloned())
            .collect();
        names.sort_unstable();
        names
    }
}

/// Test-only gates and an event log, so a test can hold a route's archive scan
/// or drain open and observe the order the registry did things in.
#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    floor_gates: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    drain_gates: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    log: parking_lot::Mutex<Vec<String>>,
}

#[cfg(test)]
impl TestHooks {
    fn gate(
        map: &parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
        name: &str,
    ) -> Arc<tokio::sync::Notify> {
        Arc::clone(map.lock().entry(name.to_string()).or_default())
    }

    async fn wait_floor_gate(&self, name: &str) {
        if self.floor_gates.lock().contains_key(name) {
            Self::gate(&self.floor_gates, name).notified().await;
        }
    }

    async fn wait_drain_gate(&self, name: &str) {
        if self.drain_gates.lock().contains_key(name) {
            Self::gate(&self.drain_gates, name).notified().await;
        }
    }

    fn log(&self, event: String) {
        self.log.lock().push(event);
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
    /// The typed error that failed this route's output
    /// ([`crate::route::RouteHandle::output_error`]), e.g. HEVC delivered to
    /// a `ts_hls` output; `null` while the output is healthy.
    pub output_error: Option<String>,
    /// `"fell_back"` or `"awaiting"` once a `ts_hls` output passed its IDR-wait
    /// bound without an IDR picture
    /// ([`crate::route::RouteHandle::ts_idr_status`]); `null` while the output
    /// is IDR-aligned.
    pub ts_hls_idr: Option<&'static str>,
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
            output_error: rt.store.output_error(),
            ts_hls_idr: rt.store.ts_idr_status().map(|status| status.name()),
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
        DashifIngest { .. } => "dashif_ingest",
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
        .map(|(name, value)| (name.as_str(), value.to_str().unwrap_or_default()))
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
        AuthResult::BadRequest => StatusCode::BAD_REQUEST.into_response(),
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

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let registry = Arc::clone(&self.registry);
        let remote_addr = self.remote_addr;
        Box::pin(async move {
            let (mut parts, body) = req.into_parts();
            // Mirrors what `Router::into_make_service_with_connect_info` does
            // for the static path (`super::serve_with_registry`) — inserted
            // directly (rather than via `axum::Extension`'s `tower::Layer`,
            // whose produced service type isn't nameable outside axum itself)
            // since `ConnectInfo<T>`'s own `FromRequestParts` impl reads it from
            // the request extensions either way.
            parts.extensions.insert(ConnectInfo(remote_addr));
            let name = parts
                .uri
                .path()
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or_default()
                .to_string();
            // A GET or HEAD has no body, so it can be served again from its parts. Such a
            // request is bound to the route that served it: when that route is replaced or
            // removed (its retire token fires) the request is dropped wherever it waits,
            // including a parked LL-HLS blocking reload, and served again against the router
            // that is live then, so it continues to wait on the replacement (same msn and
            // part) or answers 404 when no route of that name is left.
            let replay = matches!(parts.method, Method::GET | Method::HEAD).then(|| {
                (
                    parts.method.clone(),
                    parts.uri.clone(),
                    parts.version,
                    parts.headers.clone(),
                )
            });
            let mut request = Some(Request::from_parts(parts, Body::new(body)));
            let mut attempts = 0usize;
            loop {
                let req = match request.take() {
                    Some(req) => req,
                    None => match replay.as_ref() {
                        Some((method, uri, version, headers)) => {
                            let mut req = Request::new(Body::empty());
                            *req.method_mut() = method.clone();
                            *req.uri_mut() = uri.clone();
                            *req.version_mut() = *version;
                            *req.headers_mut() = headers.clone();
                            req.extensions_mut().insert(ConnectInfo(remote_addr));
                            req
                        }
                        None => return Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                    },
                };
                let live = registry.live_router();
                let mut router = live.router.clone();
                attempts += 1;
                let token = match (&replay, attempts <= MAX_REDISPATCHES) {
                    (Some(_), true) => live.retire.get(&name).cloned(),
                    _ => None,
                };
                let Some(token) = token else {
                    return router.call(req).await;
                };
                tokio::select! {
                    response = router.call(req) => return response,
                    () = token.cancelled() => {}
                }
            }
        })
    }
}

/// How many times one request is served again because its route was replaced under it; a route
/// replaced faster than that is served by the router of the last attempt.
const MAX_REDISPATCHES: usize = 8;

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
        crate::dvr::archive_seq_floor(&route.dvr, &route.name)
            .await
            .value()
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

    /// Lets every spawned task run until it is blocked. On the current-thread test runtime this
    /// is exact (no timers are involved), so a "has not finished yet" check after it cannot
    /// pass merely because a loaded machine had not scheduled the task.
    async fn settle() {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
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
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
        }
    }

    /// A slow archive scan for route `a` holds only `a`'s name lock: an admin
    /// change to route `b` is not delayed by it.
    #[tokio::test]
    async fn a_slow_archive_scan_for_one_route_does_not_block_another() {
        let registry = RouteRegistry::new(ctx_for_tests(), None);
        let gate = TestHooks::gate(&registry.hooks.floor_gates, "a");
        let slow = tokio::spawn({
            let registry = Arc::clone(&registry);
            async move { registry.add_route(rtsp_route("a", "rtsp://host/a")).await }
        });
        settle().await;
        assert!(!slow.is_finished(), "route a's scan is held open");

        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            registry.add_route(rtsp_route("b", "rtsp://host/b")),
        )
        .await
        .expect("route b is not delayed by route a's scan")
        .expect("add b");
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            registry.remove_route("b"),
        )
        .await
        .expect("removing b is not delayed either")
        .expect("remove b");

        gate.notify_one();
        slow.await.expect("task").expect("add a");
        assert!(registry.get_route("a").is_some());
    }

    /// Removing a route and adding the same name again: the add reads the
    /// archive floor only after the removed route has drained, so the new
    /// runtime cannot start numbering below a segment the old one still wrote.
    #[tokio::test]
    async fn an_add_after_a_remove_reads_the_floor_only_after_the_drain() {
        let registry = RouteRegistry::new(ctx_for_tests(), None);
        registry
            .add_route(rtsp_route("x", "rtsp://host/x"))
            .await
            .expect("first add");
        registry.hooks.log.lock().clear();
        let drain_gate = TestHooks::gate(&registry.hooks.drain_gates, "x");

        let remove = tokio::spawn({
            let registry = Arc::clone(&registry);
            async move { registry.remove_route("x").await }
        });
        while registry.get_route("x").is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let add = tokio::spawn({
            let registry = Arc::clone(&registry);
            async move { registry.add_route(rtsp_route("x", "rtsp://host/x")).await }
        });
        settle().await;
        assert!(
            !add.is_finished() && registry.hooks.log.lock().is_empty(),
            "the add must wait for the drain, not scan the archive under it"
        );

        drain_gate.notify_one();
        remove.await.expect("task").expect("remove");
        add.await.expect("task").expect("add");
        assert_eq!(
            *registry.hooks.log.lock(),
            ["drained:x".to_string(), "floor:x".to_string()]
        );
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
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
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

    /// a reload plans `z` as "added", an admin add of `z` (different config) finishes
    /// while the reload waits for z's name lock; the re-plan sees `z` as "changed" but compares
    /// only the touched NAMES, so the stale plan installs z over the live runtime without
    /// draining it.
    #[tokio::test]
    async fn a_reload_planned_before_a_concurrent_add_does_not_leak_the_added_runtime() {
        let dir = std::env::temp_dir().join(format!("audit14-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"routes":[{"name":"z","input":{"type":"rtsp","url":"rtsp://127.0.0.1:554/z2"},"outputs":["llhls"]}]}"#,
        )
        .unwrap();
        let registry = RouteRegistry::new(ctx_for_tests(), Some(path));
        let gate = TestHooks::gate(&registry.hooks.floor_gates, "z");

        let add = tokio::spawn({
            let registry = Arc::clone(&registry);
            async move {
                registry
                    .add_route(rtsp_route("z", "rtsp://127.0.0.1:554/z1"))
                    .await
            }
        });
        settle().await;
        let reload = tokio::spawn({
            let registry = Arc::clone(&registry);
            async move { registry.reload().await }
        });
        settle().await;

        gate.notify_one();
        add.await.expect("task").expect("add z");
        let first_runtime_cancel = registry
            .inner
            .read()
            .get("z")
            .expect("z installed")
            .cancel
            .clone();
        // The reload reads z's floor again on its second plan: keep opening the gate.
        let opener = tokio::spawn({
            let gate = Arc::clone(&gate);
            async move {
                loop {
                    gate.notify_one();
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        });
        let summary = tokio::time::timeout(std::time::Duration::from_secs(20), reload)
            .await
            .expect("reload finishes")
            .expect("task")
            .expect("reload");
        opener.abort();
        assert!(
            first_runtime_cancel.is_cancelled(),
            "the runtime the add installed was replaced without being cancelled/drained \
             (summary: {summary:?})"
        );
    }

    /// a reload that restarts a CHANGED route reads its archive floor while the old
    /// runtime is still live and writing; the old runtime is drained only after the new one is
    /// installed, so the new runtime can number below a segment the old one still writes.
    #[tokio::test]
    async fn a_changed_route_reads_its_archive_floor_only_after_the_old_runtime_drained() {
        let dir = std::env::temp_dir().join(format!("audit14-changed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"routes":[{"name":"z","input":{"type":"rtsp","url":"rtsp://127.0.0.1:554/z2"},"outputs":["llhls"]}]}"#,
        )
        .unwrap();
        let registry = RouteRegistry::new(ctx_for_tests(), Some(path));
        registry
            .add_route(rtsp_route("z", "rtsp://127.0.0.1:554/z1"))
            .await
            .expect("add z");
        let old_cancel = registry.inner.read().get("z").expect("z").cancel.clone();
        let gate = TestHooks::gate(&registry.hooks.floor_gates, "z");
        let reload = tokio::spawn({
            let registry = Arc::clone(&registry);
            async move { registry.reload().await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let drained_when_floor_is_read = old_cancel.is_cancelled();
        gate.notify_one();
        let _ = reload.await;
        assert!(
            drained_when_floor_is_read,
            "the archive floor of the changed route z was read while the old runtime was live"
        );
    }
}
