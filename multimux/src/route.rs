//! One route's shared state: a [`ProgramId`]-keyed registry of
//! `ProgramServing` bundles — each one the `media_plane::Trunk` a program's
//! samples land in, the sans-IO LL-HLS origin over it
//! ([`hls_runtime::server::HlsOrigin`], plan step 4), and the small
//! `Trunk`-drained window DASH/LL-DASH need beyond what any `Trunk` ring
//! holds (`DashState`, below) — plus this route's [`HealthState`], which is
//! route-wide, not per-program.
//!
//! Replaces the deleted `hls_runtime::server::{MediaStore, HealthState}`
//! (step 5b): `MediaStore`'s playlist-rendering half moved into
//! [`HlsOrigin`] (step 4, reusing it rather than reimplementing it here —
//! see this crate's `http` module docs); its DASH-only fields
//! (`track_specs`/`created_at`/the closed-segment window) have no `Trunk`
//! ring of their own (a `Trunk` carries samples/segments/parts/events, never
//! codec metadata or a route's start time) and are recreated here, in
//! exactly the same "one small synced window, fed by draining one
//! `SegmentCursor`, never a second cache of the `Trunk`'s own data" shape
//! [`HlsOrigin`]'s own `Window` already established — see that type's
//! module doc, "The one thing that genuinely cannot come from the `Trunk`
//! alone". [`HealthState`] (route up/down) is also recreated here: it is
//! *not* [`media_plane::ingress::HealthState`], which is generic over one
//! ingest session's own error type and cannot give a homogeneous "is this
//! route live" answer across routes fed by different connectors.
//!
//! # No pre-first-program placeholder (issue #805 task 6)
//!
//! Earlier revisions of this type owned a single `Trunk`/`HlsOrigin`/
//! `DashState` triple, built eagerly in [`RouteHandle::new`] before any
//! program was known, with a `publish_owned_trunk` method to register that
//! placeholder into the (separately added) program registry once a caller had
//! written test data into it. That shape is gone. [`RouteHandle::new`] now
//! builds *no* serving state at all — a `ProgramServing` bundle is created
//! the instant (and only the instant) [`RouteHandle::publish_program`] is
//! called for that [`ProgramId`], exactly mirroring
//! `media_plane::IngestDriver` minting a `Trunk` the instant it observes
//! `media_plane::SessionEvent::NewProgram`.
//!
//! This closes the **publish-or-hang footgun** for good: previously, a
//! producer that wrote directly into the placeholder `Trunk` but forgot to
//! publish it left every request blocking on
//! `ProgramResolution::NotYetAnnounced` forever — a hang, not an error
//! (issue #805 task 2 caught exactly this bug twice, in both of that era's
//! production writers). That is now structurally impossible: there is no
//! `Trunk`/`HlsOrigin`/`DashState` to write into, resolvably or not, until
//! [`RouteHandle::publish_program`] creates one. A test or plugin that wants
//! to drive a program's serving state directly (rather than through a real
//! `media_plane::IngestDriver`) calls [`RouteHandle::publish_new_program`],
//! which mints a `Trunk` sized like this route's own configured ring
//! capacities and publishes it in one step — there is no way to get a
//! `Trunk` handle back from this type without it already being registered.
//!
//! # MPTS addressing (issue #805 task 6 — decided: documented, not implemented)
//!
//! With per-program serving state in place, one `RouteHandle`/one HTTP route
//! can genuinely serve several programs (several call sites already
//! exercise this — see `two_programs_resolve_independently_to_different_trunks`/
//! `two_programs_on_one_route_segment_independently`/
//! `two_programs_serve_distinct_media`). What is **not** wired up
//! is a way for an HTTP request to *select* a non-default program: every
//! egress call site in this crate (`crate::http::resolve_route_program`,
//! `crate::output::llhls`/`dash`/`ll_dash`, `crate::origin::resource`)
//! resolves the fixed `SPTS_PROGRAM_ID` — a request with no program
//! selector at all resolves to it exactly as before, and there is currently
//! no *other* selector a request can carry. Three ways to add one, in
//! ascending order of both flexibility and implementation cost:
//!
//! 1. **A URL path segment** (e.g. `/{stream}/program/{n}/media.m3u8`) — the
//!    most RESTful option and the easiest to cache/proxy correctly (the
//!    program identity is part of the resource's own address), but it changes
//!    every existing URL shape and needs a routing change in every output
//!    module plus the shared resource route.
//! 2. **A query parameter** (e.g. `?program=2`) — additive: every existing
//!    URL keeps working unchanged (no query parameter == `SPTS_PROGRAM_ID`),
//!    and it slots into the existing `Query<..>` extractors
//!    (`crate::output::llhls::BlockingReloadQuery` already does this for
//!    blocking-reload parameters). The natural "MVP" choice.
//! 3. **A config-declared per-program route** — `crate::config::Route` grows
//!    a second name/output set bound to a specific non-default `ProgramId` on
//!    the same input, so an MPTS route's second program gets its own
//!    first-class `{stream-name}` entry (and therefore its own URL namespace)
//!    rather than being a variant of the first program's. Most explicit, but
//!    it means the operator must already know an MPTS input's program
//!    numbers at config time (from an EPG/PMT dump, e.g.), which a query
//!    parameter or path segment does not require.
//!
//! **Recommendation:** option 2 (query parameter) first — it is additive, it
//! reuses the extractor pattern this crate already has, and it does not
//! require an operator to know a stream's program numbers before the route
//! exists (unlike option 3), nor a routing rewrite across every output module
//! (unlike option 1). Option 1 or 3 can still be layered in later without
//! contradicting it (a query parameter and a path segment are not mutually
//! exclusive; a config-declared route could simply pre-select one via the
//! same query mechanism internally).
//!
//! # Known gap: `NotYetAnnounced` is derived from "the registry is empty"
//!
//! `RouteHandle::resolve_program`'s `ProgramResolution::NotYetAnnounced`
//! vs. `ProgramResolution::NotFound` split (see that enum's own doc) is
//! computed from whether *any* program has been published yet, because that
//! is all this registry knows — it has no visibility into what the transport
//! itself has promised. On an MPTS route where program 1 has already been
//! minted and published but program 2 has not yet appeared (its PAT entry
//! exists but its PMT hasn't arrived, e.g.), resolving program 2 today returns
//! `ProgramResolution::NotFound` — a permanent 404 — when it arguably should
//! be `ProgramResolution::NotYetAnnounced` (a program the transport has
//! already promised, just not fully announced yet). Fixing this needs the
//! registry (or its caller) to know the transport's own promised program set
//! (e.g. the PAT's program list), not just which `Trunk`s have actually been
//! minted — out of scope here, recorded so a future MPTS-addressing task
//! does not have to rediscover it.

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{Container, HlsOrigin};
use media_plane::trunk::{
    PartEntry, SegmentCursor, SegmentCursorItem, SegmentEntry, SegmentWriter, TrunkConfig,
};
use media_plane::{ProgramId, Trunk};
use transmux::TrackSpec;

use crate::dvr::DvrRecorder;

/// Ring capacities [`ProgramServing::new`] gives each program's `Trunk`,
/// beyond [`crate::config::Config::window_segments`] (which becomes the
/// segment log's own capacity — the same "advertised window == retained
/// window" depth the deleted `MediaStore` gave a route). None of these three
/// rings (timed/sparse samples, events) are used by the `LlHlsSegmenter`-fed
/// pipeline a driver-backed route serves (it publishes finished
/// segments/parts directly, via `crate::source::segment`, never raw samples
/// through this crate) — sized generously rather than at `1` so a future
/// sample-level consumer of the same `Trunk` (a `PushEgress` tap, a DVR
/// `SegmentEgress`) has real headroom without a config change.
const DEFAULT_TIMED_CAPACITY: usize = 64;
const DEFAULT_SPARSE_CAPACITY: usize = 16;
const DEFAULT_EVENT_CAPACITY: usize = 64;
/// Live-part log / concurrent-waiter capacity (`TrunkConfig::part_capacity`
/// shares both jobs — see that field's own doc). Generous relative to how
/// many parts one open segment realistically holds.
const DEFAULT_PART_CAPACITY: usize = 64;

/// [`RouteHandle::name`]'s value until [`RouteHandle::with_name`] sets a real
/// one — matches `crate::origin::mod`'s own `"unknown"` fallback token for an
/// unrecognized route in its HTTP-path metrics labelling, so a route driven
/// directly (skipping `crate::origin::serve_with_registry`, e.g. a bare unit
/// test) still labels its own metrics with a clearly-synthetic, never-blank
/// value rather than an empty string.
const DEFAULT_ROUTE_NAME: &str = "unknown";

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("route.rs capacity constants are all non-zero")
}

/// The [`ProgramId`] every single-program-route (SPTS) publishes its `Trunk`
/// under in [`RouteHandle`]'s registry (issue #805 task 2) — every
/// driver-backed route (all nine input kinds, via
/// [`crate::source::report_driver_progress`]) publishes here, so egress
/// resolves every route uniformly through [`RouteHandle::resolve_program`].
///
/// A single named constant rather than a bare `ProgramId(0)` scattered across
/// call sites: this is the SPTS default. Resolving a request to a
/// *non-default* program (MPTS: N programs on one route) is the addressing
/// question this module's own doc records the options for (issue #805 task
/// 6) — when a selector is added, this constant becomes just the fallback a
/// request with no explicit program selector resolves to, not something
/// every call site independently hardcodes.
pub(crate) const SPTS_PROGRAM_ID: ProgramId = ProgramId(0);

/// This route's ingest health — distinct from
/// [`media_plane::ingress::HealthState`] (generic over one session's own
/// connector error type); this is the homogeneous "is *this route*
/// currently serving live media" status `crate::origin::readyz` and
/// `crate::prometheus::ROUTE_UP` need, regardless of which connector feeds
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HealthState {
    /// Not yet connected (or reconnecting for the first time this process).
    Connecting,
    /// Connected and actively producing media.
    Live,
    /// Lost its connection/ingest and is retrying with backoff — never
    /// permanent; see `crate::origin::supervisor::supervise_driver`'s own doc
    /// for why a route always retries rather than giving up.
    Reconnecting,
    /// Reserved for an unrecoverable error class a future caller may want to
    /// distinguish from an ordinary, retried [`HealthState::Reconnecting`] —
    /// `supervise_driver`'s loop does not currently produce it.
    Failed,
}

impl HealthState {
    /// The spec/field-enum label (workspace #204 convention).
    pub fn name(&self) -> &'static str {
        match self {
            HealthState::Connecting => "connecting",
            HealthState::Live => "live",
            HealthState::Reconnecting => "reconnecting",
            HealthState::Failed => "failed",
        }
    }
}

broadcast_common::impl_spec_display!(HealthState);

/// The outcome of resolving one [`ProgramId`] against a route's
/// [`RouteHandle::resolve_program`] registry.
///
/// Deliberately **not** an `Option<Arc<ProgramServing>>`: a bare `Option` can
/// only say "here it is" or "no" — it cannot distinguish two `None`-shaped
/// situations that need opposite egress treatment. "No program has been
/// announced on this route at all yet" (ingest may still be dialing/
/// connecting/negotiating — a **transient** condition) and "this route has at
/// least one live program, but not the one that was asked for" (a
/// **permanent** condition — the requested [`ProgramId`] will not spring into
/// existence later) look identical as `None`, but the first should make a
/// caller wait or answer not-ready (exactly
/// [`media_plane::EgressResponse::Await`]'s own case), while the second is a
/// genuine 404. Collapsing them would make an ordinary SPTS route mid-connect
/// indistinguishable from a client misaddressing an MPTS program that will
/// never exist on this route. (See this module's own doc for the one known
/// gap in this split: it cannot yet distinguish "MPTS program not minted yet"
/// from "this program will never exist".)
///
/// Not given a `name()`/`Display` under the workspace's #204 spec/field-enum
/// label convention: like [`media_plane::EgressResponse`] (whose module doc
/// gives the same rationale, and whose crate's `tests/label_coverage.rs`
/// skip-list names it for exactly this reason), this is a **data-carrying**
/// control-flow ADT — [`ProgramResolution::Found`] carries the resolved
/// [`ProgramServing`] bundle itself — not a static token decoded from a cited
/// spec; a caller already matches the typed variant instead of formatting a
/// label. `multimux` has no `tests/label_coverage.rs` drift-guard of its own
/// yet (unlike every sibling crate) to enforce or record that exemption
/// mechanically — see this task's own report.
///
/// Read by every migrated egress call site (`crate::output::llhls`/`dash`/
/// `ll_dash`, `crate::origin::resource`, via
/// `crate::http::resolve_route_program`): `Found` resolves and serves;
/// `NotYetAnnounced` is a wait/`503`-style "not ready"; `NotFound` is a
/// genuine `404` — see each call site for how it maps these.
#[non_exhaustive]
pub(crate) enum ProgramResolution {
    /// `program` is registered; here is its serving bundle.
    Found(Arc<ProgramServing>),
    /// No program has been announced on this route at all yet. Not a 404 —
    /// see this type's own doc for why a caller should treat this as
    /// "not ready", not "will never exist".
    NotYetAnnounced,
    /// At least one program is registered on this route, but not `program`.
    /// Unlike [`ProgramResolution::NotYetAnnounced`], this is a genuine,
    /// permanent 404: the route is live and simply does not carry the
    /// requested [`ProgramId`].
    NotFound,
}

/// One closed segment's identity, as DASH/LL-DASH's `Representation`
/// rendering needs it (`crate::output::dash`/`crate::output::ll_dash`) —
/// bytes are never held here (a DASH manifest never embeds segment bytes;
/// it only names them), so this is not a duplicate of what
/// [`HlsOrigin`]'s own `Window` (or the `Trunk`'s segment log itself)
/// holds.
#[derive(Debug, Clone, Copy)]
pub struct DashWindowSegment {
    /// Matches [`media_plane::trunk::SegmentEntry::sequence_number`].
    pub segment_seq: u32,
    /// Matches [`media_plane::trunk::SegmentEntry::duration`], as seconds.
    pub duration_secs: f64,
}

/// The small `Trunk`-drained state DASH/LL-DASH rendering needs beyond any
/// `Trunk` ring — see this module's own doc. Shared by
/// [`crate::output::dash::DashOutput`] and
/// [`crate::output::ll_dash::LlDashOutput`] (the same single `MediaStore`
/// both read from before this port), so a route configuring both outputs
/// for the same program still has exactly one drained window, not two
/// independently-lagging copies.
pub(crate) struct DashState {
    cursor: Mutex<SegmentCursor>,
    window: Mutex<VecDeque<DashWindowSegment>>,
    capacity: usize,
    track_specs: Mutex<Vec<TrackSpec>>,
}

impl DashState {
    fn new(trunk: &Arc<Trunk>, window_segments: NonZeroUsize) -> Self {
        DashState {
            cursor: Mutex::new(trunk.subscribe_segments()),
            window: Mutex::new(VecDeque::new()),
            capacity: window_segments.get(),
            track_specs: Mutex::new(Vec::new()),
        }
    }

    fn set_track_specs(&self, specs: Vec<TrackSpec>) {
        *self
            .track_specs
            .lock()
            .expect("DashState track_specs lock poisoned") = specs;
    }

    fn track_specs(&self) -> Vec<TrackSpec> {
        self.track_specs
            .lock()
            .expect("DashState track_specs lock poisoned")
            .clone()
    }

    /// Absorb every segment this route's `SegmentCursor` has produced since
    /// the last call — the same non-blocking, called-at-the-top-of-render
    /// shape as [`HlsOrigin`]'s own `drain` (see that type's module doc).
    fn drain(&self) {
        let mut cursor = self
            .cursor
            .lock()
            .expect("DashState segment cursor lock poisoned");
        let mut window = self.window.lock().expect("DashState window lock poisoned");
        while let Some(item) = cursor.poll() {
            if let SegmentCursorItem::Segment(entry) = item {
                if window.len() == self.capacity {
                    window.pop_front();
                }
                window.push_back(DashWindowSegment {
                    segment_seq: entry.sequence_number,
                    duration_secs: entry.duration.as_secs_f64(),
                });
            }
        }
    }

    fn window_segments(&self) -> Vec<DashWindowSegment> {
        self.drain();
        self.window
            .lock()
            .expect("DashState window lock poisoned")
            .iter()
            .copied()
            .collect()
    }
}

/// One program's complete serving state: its `Trunk`, the sans-IO
/// [`HlsOrigin`] built over it, and the [`DashState`] window drained from
/// it — grouped in one struct (rather than three parallel
/// `ProgramId`-keyed maps that could disagree with each other about which
/// `Trunk` a given program's `ll_hls`/`dash` are actually bound to) because
/// they are always created, read, and replaced together: see
/// [`RouteHandle::publish_program`], the only place one of these is built.
///
/// `pub(crate)`: read by `crate::http::resolve_route_program` (the one place
/// every egress call site resolves one of these from a `RouteHandle`) and by
/// this crate's own tests; never named outside this crate.
pub(crate) struct ProgramServing {
    trunk: Arc<Trunk>,
    ll_hls: Arc<HlsOrigin>,
    dash: Arc<DashState>,
    /// DVR durable segment archive (issue #746) — `None` when this route
    /// does not have DVR enabled. Owns a pinning `SegmentCursor`, drained
    /// by [`Self::poll_dvr`].
    dvr: Mutex<Option<DvrRecorder>>,
    /// Lazily-acquired write handle for [`RouteHandle::add_segment`]/
    /// [`RouteHandle::add_part`] (this crate's own direct-`Trunk`-write test
    /// helpers, standing in for a real `crate::source::segment::ProgramSegmenter`).
    /// `None` until the first such call — **not** acquired eagerly at
    /// publish time, because a driver-backed program's segment writer is
    /// taken by its own real `ProgramSegmenter` immediately after
    /// `crate::source::report_driver_progress` publishes it
    /// (`crate::source::segment::drive_program_segmenters`, the very next
    /// call in the same iteration); grabbing it here first would starve that
    /// real segmenter of the one segment/part writer a `Trunk` ever hands
    /// out (`media_plane::Trunk::segment_writer` returns `None` on every call
    /// after the first). A program driven by a real segmenter never calls
    /// [`RouteHandle::add_segment`]/[`add_part`](RouteHandle::add_part), so
    /// the two writers never actually contend in practice.
    segment_writer: Mutex<Option<SegmentWriter>>,
    /// Cumulative nanoseconds of segment duration published so far via
    /// [`RouteHandle::add_segment`] — the only honest source for
    /// [`media_plane::trunk::SegmentEntry::timeline_position`] a test writing
    /// straight through it has (it never itself computes an
    /// absolute-timeline position). Independent per program, since each
    /// program's own timeline starts at zero.
    next_timeline_ns: AtomicU64,
}

impl ProgramServing {
    /// Build a fresh bundle over `trunk`: the [`HlsOrigin`] and
    /// [`DashState`] every egress call site reads, both bound to `trunk` from
    /// construction, plus an optional DVR recorder if `dvr_config` is
    /// provided. The one and only place either is created — see
    /// [`RouteHandle::publish_program`].
    fn new(
        trunk: Arc<Trunk>,
        target_duration_secs: f64,
        part_target_ms: u32,
        window_segments: NonZeroUsize,
        container: Container,
        dvr_config: Option<crate::dvr::DvrConfig>,
        route_name: &str,
    ) -> Arc<Self> {
        let mut builder = HlsOrigin::builder(Arc::clone(&trunk))
            .target_duration_secs(target_duration_secs)
            .window_segments(window_segments)
            .container(container);
        if container == Container::Fmp4 {
            builder = builder.low_latency(part_target_ms);
        }
        let ll_hls = Arc::new(
            builder
                .build()
                .expect("target_duration_secs and window_segments are always set above"),
        );
        let dash = Arc::new(DashState::new(&trunk, window_segments));

        let ext = match container {
            Container::Fmp4 => ".m4s",
            Container::MpegTs => ".ts",
            _ => ".m4s",
        };
        let dvr = dvr_config.filter(|c| c.enabled).and_then(|cfg| {
            match DvrRecorder::new(route_name.to_string(), cfg, ext, &trunk) {
                Ok(recorder) => {
                    tracing::info!(
                        route = %route_name,
                        "DVR recording started"
                    );
                    Some(recorder)
                }
                Err(e) => {
                    tracing::error!(
                        route = %route_name,
                        error = %e,
                        "DVR recorder creation failed; recording disabled for this program"
                    );
                    None
                }
            }
        });

        Arc::new(ProgramServing {
            trunk,
            ll_hls,
            dash,
            dvr: Mutex::new(dvr),
            segment_writer: Mutex::new(None),
            next_timeline_ns: AtomicU64::new(0),
        })
    }

    /// This program's `Trunk` — the same `Arc` `crate::http::resolve_route_program`
    /// hands to [`crate::http::resolve_blocking`].
    pub(crate) fn trunk(&self) -> Arc<Trunk> {
        Arc::clone(&self.trunk)
    }

    /// This program's sans-IO LL-HLS origin.
    pub(crate) fn ll_hls(&self) -> Arc<HlsOrigin> {
        Arc::clone(&self.ll_hls)
    }

    fn set_init(&self, bytes: impl Into<Bytes>) {
        self.ll_hls.set_init(bytes);
    }

    fn init_bytes(&self) -> Option<Bytes> {
        self.ll_hls.init_bytes()
    }

    fn set_track_specs(&self, specs: Vec<TrackSpec>) {
        self.dash.set_track_specs(specs);
    }

    fn track_specs(&self) -> Vec<TrackSpec> {
        self.dash.track_specs()
    }

    fn window_segments(&self) -> Vec<DashWindowSegment> {
        self.dash.window_segments()
    }

    /// Runs `f` against this program's `Trunk` segment writer, acquiring it
    /// (from the `Trunk` itself) on first use. `None` if the writer is
    /// unavailable — already taken by a real `crate::source::segment::ProgramSegmenter`
    /// on this same `Trunk` (see this struct's own `segment_writer` field
    /// doc) — in which case the caller logs rather than panics: a test
    /// mis-mixing both write paths on one `Trunk` is a test-construction bug,
    /// not something that should take the whole process down.
    fn with_segment_writer<R>(&self, f: impl FnOnce(&SegmentWriter) -> R) -> Option<R> {
        let mut guard = self
            .segment_writer
            .lock()
            .expect("ProgramServing segment_writer lock poisoned");
        if guard.is_none() {
            *guard = self.trunk.segment_writer();
        }
        guard.as_ref().map(f)
    }

    fn add_part(&self, info: transmux::ll_hls::PartInfo) {
        let published = self.with_segment_writer(|writer| {
            writer.publish_part(PartEntry::new(
                info.bytes,
                info.segment_seq,
                info.part_index,
                Duration::from_secs_f64(info.duration),
                info.independent,
            ));
        });
        if published.is_none() {
            tracing::warn!(
                "RouteHandle::add_part: this program's Trunk segment writer is unavailable \
                 (already taken by a real ProgramSegmenter?)"
            );
        }
    }

    fn add_segment(&self, info: transmux::ll_hls::SegmentInfo) {
        let duration = Duration::from_secs_f64(info.duration);
        let start_ns = self.next_timeline_ns.fetch_add(
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        let published = self.with_segment_writer(|writer| {
            writer.publish_segment(SegmentEntry::new(
                info.bytes,
                info.segment_seq,
                duration,
                Timestamp::from_nanos(start_ns),
                transmux::SegmentMeta {
                    discontinuous: false,
                },
            ));
        });
        if published.is_none() {
            tracing::warn!(
                "RouteHandle::add_segment: this program's Trunk segment writer is unavailable \
                 (already taken by a real ProgramSegmenter?)"
            );
        }
    }

    /// `(in-progress-or-last-closed segment sequence number, its currently
    /// resident live-part count)` — mirrors
    /// `hls_runtime::server::HlsOrigin::live_edge`'s own derivation,
    /// directly against the `Trunk` (no cache): the abuse-bound check
    /// `crate::origin::resource`'s chunked-transfer whole-segment route
    /// (issue #721) needs.
    pub(crate) fn latest_progress(&self) -> (u32, usize) {
        let last_closed = self.trunk.last_closed_segment().unwrap_or(0);
        let candidate = last_closed + 1;
        let parts = self.trunk.parts_in_segment(candidate);
        if parts.is_empty() {
            (last_closed, 0)
        } else {
            (candidate, parts.len())
        }
    }

    /// Drain this program's DVR pinning cursor (if DVR is enabled) and
    /// persist any new finished segments to disk. Called by the route's
    /// supervise loop after `drive_program_segmenters`.
    pub(crate) fn poll_dvr(&self) {
        let mut dvr_guard = self.dvr.lock().expect("ProgramServing dvr lock poisoned");
        if let Some(ref mut recorder) = *dvr_guard {
            let init_bytes = self.init_bytes();
            let init_slice = init_bytes.as_deref();
            if let Err(e) = recorder.poll_and_persist(init_slice) {
                tracing::error!(
                    "DVR poll_and_persist failed: {e}; \
                     recording may be incomplete"
                );
            }
        }
    }
}

/// One route's shared state — replaces the deleted `MediaStore`. Built once
/// per configured route ([`crate::origin::serve_with_registry`]), then shared
/// (via `Arc`) between whatever ingest publishes into it (every
/// driver-backed `crate::source::*::run_*`, via `crate::source::report_driver_progress`/
/// `crate::source::segment::drive_program_segmenters` — or the single
/// [`crate::source::advance_route`] facade over both) and every
/// [`crate::output::Output`]/the shared resource route that reads from it.
///
/// Owns **no** `Trunk`/`HlsOrigin`/`DashState` of its own — see this
/// module's own "No pre-first-program placeholder" doc. Every program's
/// serving state lives in `Self::programs` (the `programs` field, below),
/// created only by [`Self::publish_program`].
pub struct RouteHandle {
    health: Mutex<HealthState>,
    target_duration_secs: f64,
    part_target_ms: u32,
    /// Stored so [`Self::publish_program`]/[`Self::publish_new_program`] can
    /// build every program's [`ProgramServing`]/`Trunk` with the same
    /// advertised-window depth [`Self::new`] was given.
    window_segments_cap: NonZeroUsize,
    /// Which container every program on this route is served as — set via
    /// [`Self::with_container`], defaulting to [`Container::Fmp4`] (every
    /// pre-#887 route's behaviour, unchanged). A route-wide property, not a
    /// per-program one (see issue #887 / `crate::config::Route::outputs`'s
    /// exclusivity rule): a `Trunk` has one segment ring per program, so a
    /// program's samples are segmented into fMP4 *or* TS, never both.
    container: Container,
    created_at: SystemTime,
    /// Egress-resolvable `ProgramId -> Arc<ProgramServing>` registry:
    /// `media_plane::IngestDriver` mints one `Trunk` per program the instant
    /// it observes `media_plane::SessionEvent::NewProgram`, while this
    /// `RouteHandle` is built *before* any program is known
    /// (`crate::origin::serve_with_registry` constructs one per configured
    /// route, ahead of dialing/listening). This map is how those two
    /// lifetimes reconcile without forcing either one to change when the
    /// other becomes ready: [`RouteHandle::publish_program`] is the ingest
    /// side's write (every driver-backed `run_*` calls it via
    /// `crate::source::report_driver_progress`), [`RouteHandle::resolve_program`]
    /// is the egress side's read (every migrated egress call site resolves
    /// through it, via `crate::http::resolve_route_program`).
    ///
    /// A `HashMap`, not a single slot: MPTS carries an unbounded (bounded
    /// only by `media_plane::IngestDriver::max_programs`) number of programs
    /// per one route/`RouteHandle`, so the registry's shape must already be
    /// "N programs" even though most routes wired up so far are SPTS (exactly
    /// one program). See this module's own doc for the MPTS *addressing*
    /// question this leaves open (selecting a non-default program from an
    /// HTTP request) — the registry itself already supports N programs today.
    ///
    /// # Why `RwLock<HashMap<..>>`, not `Mutex` (unlike this struct's
    /// `Mutex<HealthState>` field, above)
    ///
    /// [`RouteHandle::resolve_program`] is the hottest read path this handle
    /// exposes: unlike `health` (touched once per connection-state
    /// transition), a route's program registry is read on *every* served
    /// HTTP request, across however many concurrent viewers that route has.
    /// A `Mutex` would serialize all of those mutually non-conflicting reads
    /// against one another for no reason; `RwLock` lets an unbounded number
    /// of concurrent readers proceed together and blocks only around the
    /// rare write — at most once per announced program, bounded by
    /// `max_programs`, never a steady-state cost.
    programs: RwLock<HashMap<ProgramId, Arc<ProgramServing>>>,
    /// This route's name, for labelling metrics that are per-route but
    /// recorded from code with no other way to know it (issue #809:
    /// `crate::source::segment::drive_program_segmenters`'s parts/segments
    /// counters). Defaults to [`DEFAULT_ROUTE_NAME`] until [`Self::with_name`]
    /// sets a real one; `crate::origin::serve_with_registry` always does, for
    /// every production route.
    name: String,
    /// Per-route DVR config — `None` when not configured, passed to every
    /// [`ProgramServing::new`] built by [`Self::publish_program`].
    dvr_config: Option<crate::dvr::DvrConfig>,
    /// Notifies waiters when a new program is published via
    /// [`Self::publish_program`] — used by push output tasks that need to
    /// discover a `Trunk` to subscribe to (issue #744).
    program_notify: tokio::sync::Notify,
}

impl RouteHandle {
    /// Build a fresh route: no program known yet, an empty registry, and this
    /// route's target duration/part target/window depth recorded so every
    /// later [`Self::publish_program`] builds each program's `Trunk`/
    /// `HlsOrigin`/`DashState` consistently.
    pub fn new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self {
        let window_segments = NonZeroUsize::new(window_segments).unwrap_or(NonZeroUsize::MIN);
        RouteHandle {
            health: Mutex::new(HealthState::Connecting),
            target_duration_secs,
            part_target_ms,
            window_segments_cap: window_segments,
            container: Container::default(),
            created_at: SystemTime::now(),
            programs: RwLock::new(HashMap::new()),
            name: DEFAULT_ROUTE_NAME.to_string(),
            dvr_config: None,
            program_notify: tokio::sync::Notify::new(),
        }
    }

    /// Names this route (see [`Self::name`]'s own field doc) — a consuming
    /// builder so `crate::origin::serve_with_registry`'s one production call
    /// site can chain it straight onto [`Self::new`] before wrapping the
    /// result in an `Arc`.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// This route's name (see the `name` field's own doc) —
    /// `DEFAULT_ROUTE_NAME` (`"unknown"`) if [`Self::with_name`] was never
    /// called.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets which container every program on this route is served as
    /// (issue #887) — a consuming builder, chained onto [`Self::new`] the
    /// same way as [`Self::with_name`]. Every program [`Self::publish_program`]
    /// builds from this point on gets an [`hls_runtime::server::HlsOrigin`]
    /// configured with this container (see `ProgramServing::new`).
    /// Defaults to [`Container::Fmp4`] if never called.
    pub fn with_container(mut self, container: Container) -> Self {
        self.container = container;
        self
    }

    /// Attach DVR config to this route (issue #746) — a consuming builder,
    /// chained onto [`Self::new`] the same way as [`Self::with_name`].
    /// Every program [`Self::publish_program`] builds from this point on
    /// gets a DVR recorder created alongside its `ProgramServing`.
    pub fn with_dvr(mut self, dvr_config: crate::dvr::DvrConfig) -> Self {
        self.dvr_config = Some(dvr_config);
        self
    }

    /// This route's configured container (see [`Self::with_container`]) —
    /// needed by [`crate::source::segment::drive_program_segmenters`] to pick
    /// the fMP4 (`LlHlsSegmenter`) or classic-TS
    /// (`transmux::ts_hls::StreamingTsHlsSegmenter`) segmenter path for every
    /// program on this route.
    pub(crate) fn container(&self) -> Container {
        self.container
    }

    /// This route's configured advertised-window depth (see
    /// [`Self::window_segments_cap`]'s own field doc) — needed by
    /// [`crate::source::segment::drive_program_segmenters`] to size a
    /// `StreamingTsHlsSegmenter`'s own internal rolling-playlist window
    /// consistently with every other per-route capacity.
    pub(crate) fn window_segments_cap(&self) -> NonZeroUsize {
        self.window_segments_cap
    }

    /// Publish `program`'s `Trunk` into this route's registry, building its
    /// `ProgramServing` bundle (`HlsOrigin`+`DashState`) the first time —
    /// the ingest side's write. Two kinds of caller: every driver-backed
    /// `run_*`, via `crate::source::report_driver_progress`, once
    /// `media_plane::IngestDriver` reports `media_plane::SessionEvent::NewProgram`
    /// and mints a `Trunk` for it; and [`Self::publish_new_program`], which
    /// mints its own `Trunk` and calls this — this crate's own tests' way of
    /// getting a program registered without a real driver.
    ///
    /// A repeat call for a `program` already bound to the exact same `Arc`
    /// is a no-op (does not rebuild the bundle, which would silently discard
    /// whatever was already written through it) — `media_plane::IngestDriver`
    /// itself keeps one stable `Trunk` per `ProgramId` for the life of a
    /// session (a repeat `NewProgram` for an already-known program does not
    /// mint a second `Trunk`), so in practice this is called once per program
    /// with a stable `Arc`, then repeatedly as a harmless idempotent check.  A
    /// call with a **different** `Arc` for an already-registered `ProgramId`
    /// rebuilds the bundle over the new `Trunk` (unusual — no caller in this
    /// crate does it today — but a rebind, not silently ignored).
    pub fn publish_program(&self, program: ProgramId, trunk: Arc<Trunk>) {
        let mut programs = self
            .programs
            .write()
            .expect("RouteHandle::programs lock poisoned");
        let already_bound = programs
            .get(&program)
            .is_some_and(|existing| Arc::ptr_eq(&existing.trunk, &trunk));
        if already_bound {
            return;
        }
        let serving = ProgramServing::new(
            trunk,
            self.target_duration_secs,
            self.part_target_ms,
            self.window_segments_cap,
            self.container,
            self.dvr_config.clone(),
            &self.name,
        );
        programs.insert(program, serving);
        self.program_notify.notify_waiters();
    }

    /// Wait until at least one program is published on this route, then return
    /// the first program's `Trunk`. Used by push output tasks (issue #744).
    pub async fn await_first_trunk(&self) -> Arc<Trunk> {
        loop {
            {
                let programs = self
                    .programs
                    .read()
                    .expect("RouteHandle::programs lock poisoned");
                if let Some(serving) = programs.values().next() {
                    return serving.trunk();
                }
            }
            self.program_notify.notified().await;
        }
    }

    /// Test/plugin convenience: mint a fresh `Trunk` sized like this route's
    /// own configured ring capacities, publish it under `program` in one
    /// step, and return the `Arc<Trunk>` so a caller can also grab its own
    /// `media_plane::trunk::SegmentCursor`/`SegmentWriter` directly if it
    /// wants to feed or read raw data outside [`Self::add_segment`]/
    /// [`Self::add_part`]/[`Self::set_init`].
    ///
    /// There is no way to get an unpublished `Trunk` handle back from this
    /// type — the mint and the publish happen in the same call, so a caller
    /// can never forget the second half (the "publish-or-hang" footgun this
    /// module's own doc describes is structurally impossible now, not just
    /// avoided by convention).
    pub fn publish_new_program(&self, program: ProgramId) -> Arc<Trunk> {
        let trunk_config = TrunkConfig::new(
            nz(DEFAULT_TIMED_CAPACITY),
            nz(DEFAULT_SPARSE_CAPACITY),
            self.window_segments_cap,
            nz(DEFAULT_EVENT_CAPACITY),
            nz(DEFAULT_PART_CAPACITY),
        );
        let trunk = Trunk::new(trunk_config);
        self.publish_program(program, Arc::clone(&trunk));
        trunk
    }

    /// Read-only lookup of `program`'s [`ProgramServing`] bundle, if
    /// registered. The shared implementation behind [`Self::resolve_program`]
    /// and every per-program convenience accessor below.
    pub(crate) fn serving(&self, program: ProgramId) -> Option<Arc<ProgramServing>> {
        self.programs
            .read()
            .expect("RouteHandle::programs lock poisoned")
            .get(&program)
            .cloned()
    }

    /// Resolve `program` against this route's registry — the egress side's
    /// read (see [`ProgramResolution`] for why this returns a three-way enum
    /// rather than `Option<Arc<ProgramServing>>`, and [`Self::programs`]'s
    /// own doc for why the lock is a `RwLock`). Every migrated egress call
    /// site resolves through `crate::http::resolve_route_program`, which
    /// wraps this.
    pub(crate) fn resolve_program(&self, program: ProgramId) -> ProgramResolution {
        let programs = self
            .programs
            .read()
            .expect("RouteHandle::programs lock poisoned");
        match programs.get(&program) {
            Some(serving) => ProgramResolution::Found(Arc::clone(serving)),
            None if programs.is_empty() => ProgramResolution::NotYetAnnounced,
            None => ProgramResolution::NotFound,
        }
    }

    /// `program`'s sans-IO LL-HLS origin, if registered — `#[cfg(test)]` only:
    /// used directly by this crate's own tests (e.g. `crate::source::segment`'s
    /// `render_playlist` helper); every HTTP-facing egress call site instead
    /// resolves the whole [`ProgramServing`] bundle via
    /// `crate::http::resolve_route_program` (trunk + `ll_hls` together,
    /// atomically, from one registry read), so this has no production caller.
    #[cfg(test)]
    pub(crate) fn ll_hls(&self, program: ProgramId) -> Option<Arc<HlsOrigin>> {
        self.serving(program).map(|s| s.ll_hls())
    }

    /// Store `program`'s fMP4 init segment bytes — forwards to
    /// [`HlsOrigin::set_init`]. A no-op (logged) if `program` has not been
    /// published yet — there is nowhere for the bytes to land until
    /// [`Self::publish_program`]/[`Self::publish_new_program`] creates the
    /// bundle.
    pub fn set_init(&self, program: ProgramId, bytes: impl Into<Bytes>) {
        match self.serving(program) {
            Some(serving) => serving.set_init(bytes),
            None => tracing::warn!(?program, "RouteHandle::set_init: program not published yet"),
        }
    }

    /// `program`'s fMP4 init segment bytes, if set and if `program` is
    /// registered.
    pub fn init_bytes(&self, program: ProgramId) -> Option<Bytes> {
        self.serving(program)?.init_bytes()
    }

    /// Record `program`'s track specs (issue #663 P4: DASH's `codecs` string
    /// derivation) — the one piece of codec metadata no `Trunk` ring holds. A
    /// no-op (logged) if `program` has not been published yet.
    pub fn set_track_specs(&self, program: ProgramId, specs: Vec<TrackSpec>) {
        match self.serving(program) {
            Some(serving) => serving.set_track_specs(specs),
            None => {
                tracing::warn!(
                    ?program,
                    "RouteHandle::set_track_specs: program not published yet"
                )
            }
        }
    }

    /// `program`'s recorded track specs — empty if `program` is not
    /// registered or none have been recorded yet.
    pub fn track_specs(&self, program: ProgramId) -> Vec<TrackSpec> {
        self.serving(program)
            .map(|s| s.track_specs())
            .unwrap_or_default()
    }

    /// Publish one live part (`transmux::ll_hls::LlHlsSegmenter::take_ready_parts`)
    /// into `program`'s `Trunk` live-part log. A no-op (logged) if `program`
    /// has not been published yet, or if its `Trunk`'s segment writer is
    /// already held by a real `crate::source::segment::ProgramSegmenter` (see
    /// `ProgramServing`'s own doc).
    pub fn add_part(&self, program: ProgramId, info: transmux::ll_hls::PartInfo) {
        match self.serving(program) {
            Some(serving) => serving.add_part(info),
            None => tracing::warn!(?program, "RouteHandle::add_part: program not published yet"),
        }
    }

    /// Publish one finished segment (`transmux::ll_hls::LlHlsSegmenter::take_ready_segments`)
    /// into `program`'s `Trunk` segment log. A no-op (logged) if `program`
    /// has not been published yet, or if its `Trunk`'s segment writer is
    /// already held by a real `crate::source::segment::ProgramSegmenter`.
    pub fn add_segment(&self, program: ProgramId, info: transmux::ll_hls::SegmentInfo) {
        match self.serving(program) {
            Some(serving) => serving.add_segment(info),
            None => tracing::warn!(
                ?program,
                "RouteHandle::add_segment: program not published yet"
            ),
        }
    }

    /// `program`'s currently-resident closed-segment window, oldest first —
    /// the `Trunk`-drained replacement for `MediaStore::window_segments`.
    /// Empty if `program` is not registered.
    pub fn window_segments(&self, program: ProgramId) -> Vec<DashWindowSegment> {
        self.serving(program)
            .map(|s| s.window_segments())
            .unwrap_or_default()
    }

    /// Configured target full-segment duration, seconds — route-wide (every
    /// program on a route shares one segmenter configuration), so this needs
    /// no [`ProgramId`].
    pub fn target_duration_secs(&self) -> f64 {
        self.target_duration_secs
    }

    /// Configured LL-HLS part target, milliseconds — route-wide, see
    /// [`Self::target_duration_secs`].
    pub fn part_target_ms(&self) -> u32 {
        self.part_target_ms
    }

    /// When this route was constructed — `MPD@availabilityStartTime`'s
    /// source (`crate::output::dash`/`crate::output::ll_dash`); route-wide,
    /// not something any per-program `Trunk` ring tracks.
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// This route's current [`HealthState`] — route-wide ingest health, not
    /// per-program.
    pub fn health(&self) -> HealthState {
        *self.health.lock().unwrap()
    }

    /// Transition this route's [`HealthState`].
    pub fn set_health(&self, state: HealthState) {
        *self.health.lock().unwrap() = state;
    }

    /// Drain every published program's DVR pinning cursor (if DVR is
    /// enabled on this route) — called by
    /// [`crate::source::advance_route`] once per iteration, after
    /// `drive_program_segmenters` has published any new segments.
    pub(crate) fn drain_dvr(&self) {
        let programs = self
            .programs
            .read()
            .expect("RouteHandle::programs lock poisoned");
        for serving in programs.values() {
            serving.poll_dvr();
        }
    }
}

#[cfg(test)]
mod program_registry_tests {
    //! Coverage for `RouteHandle::publish_program`/`resolve_program` (issue
    //! #805 task 1) and the per-program `ProgramServing` bundle (issue #805
    //! task 6). Each `MUTATION VERIFIED` doc comment below records a real
    //! edit made to `route.rs`'s production code, a re-run of that specific
    //! test to confirm the named assertion failed with the stated
    //! actual-vs-expected values, and the subsequent revert.

    use super::*;
    use transmux::CodecConfig;

    /// A minimal standalone `Trunk`, distinct in identity from any other
    /// call's — every ring capacity is `1` since these tests never publish
    /// samples/segments/parts/events, only check `Arc` identity through the
    /// registry.
    fn test_trunk() -> Arc<Trunk> {
        Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1)))
    }

    /// MUTATION VERIFIED: changing `publish_program` to insert a
    /// freshly-constructed `Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1),
    /// nz(1), nz(1)))`-backed bundle instead of the `trunk` argument it was
    /// actually given makes this test fail:
    /// `assert_eq!(Arc::as_ptr(&resolved.trunk()), Arc::as_ptr(&trunk))` fails
    /// (via `ProgramServing::trunk()`), comparing two genuinely different heap
    /// addresses (e.g. `0x1055f3ab0 != 0x1055f4120` — exact addresses vary per
    /// run) because the registry silently bound a different `Trunk` to the
    /// published program. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn publish_then_resolve_returns_same_trunk() {
        let route = RouteHandle::new(4.0, 500, 4);
        let trunk = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk));

        match route.resolve_program(ProgramId(1)) {
            ProgramResolution::Found(resolved) => {
                assert_eq!(Arc::as_ptr(&resolved.trunk()), Arc::as_ptr(&trunk));
            }
            _ => panic!(
                "expected ProgramResolution::Found for a just-published program, got a variant \
                 that is not Found"
            ),
        }
    }

    /// MUTATION VERIFIED: swapping `resolve_program`'s two `None` arms (so a
    /// miss against an *empty* registry returns `ProgramResolution::NotFound`
    /// and a miss against a *non-empty* registry returns
    /// `ProgramResolution::NotYetAnnounced`) makes this test fail:
    /// `assert!(matches!(result, ProgramResolution::NotYetAnnounced))` fails
    /// because `result` is actually `ProgramResolution::NotFound` — resolving
    /// against a route with zero announced programs must never look like a
    /// permanent 404. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn resolve_before_any_program_is_not_yet_announced() {
        let route = RouteHandle::new(4.0, 500, 4);
        let result = route.resolve_program(ProgramId(1));
        assert!(
            matches!(result, ProgramResolution::NotYetAnnounced),
            "expected NotYetAnnounced with an empty registry"
        );
    }

    /// MUTATION VERIFIED: changing `resolve_program`'s `None` arm to always
    /// return `ProgramResolution::NotYetAnnounced` (dropping the
    /// `programs.is_empty()` check entirely, i.e. `None =>
    /// ProgramResolution::NotYetAnnounced`) makes this test fail:
    /// `assert!(matches!(result, ProgramResolution::NotFound))` fails because
    /// `result` is actually `ProgramResolution::NotYetAnnounced` — a genuine
    /// 404 (an announced route with a different program than the one asked
    /// for) must not be reported as merely "not ready yet". Recompiled and
    /// re-run to confirm the failure, then reverted.
    #[test]
    fn resolve_unknown_program_among_others_is_not_found() {
        let route = RouteHandle::new(4.0, 500, 4);
        route.publish_program(ProgramId(1), test_trunk());

        let result = route.resolve_program(ProgramId(2));
        assert!(
            matches!(result, ProgramResolution::NotFound),
            "expected NotFound: program 2 was never published, but program 1 was"
        );
    }

    /// MUTATION VERIFIED: replacing `resolve_program`'s `programs.get(&program)`
    /// lookup with a single-program shortcut — `programs.values().next()`
    /// (return whichever entry happens to be first, ignoring `program`
    /// entirely) — makes this test fail:
    /// `assert_eq!(Arc::as_ptr(&second.trunk()), Arc::as_ptr(&trunk_b))` fails,
    /// comparing `trunk_a`'s address against `trunk_b`'s (e.g.
    /// `0x1055f3ab0 != 0x1055f4120`) because the shortcut answers program 2's
    /// request with program 1's `Trunk`. This is exactly the MPTS-readiness
    /// property a single-program-only implementation cannot satisfy.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn two_programs_resolve_independently_to_different_trunks() {
        let route = RouteHandle::new(4.0, 500, 4);
        let trunk_a = test_trunk();
        let trunk_b = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk_a));
        route.publish_program(ProgramId(2), Arc::clone(&trunk_b));

        let first = match route.resolve_program(ProgramId(1)) {
            ProgramResolution::Found(s) => s,
            _ => panic!("expected ProgramResolution::Found for program 1"),
        };
        let second = match route.resolve_program(ProgramId(2)) {
            ProgramResolution::Found(s) => s,
            _ => panic!("expected ProgramResolution::Found for program 2"),
        };

        assert_eq!(Arc::as_ptr(&first.trunk()), Arc::as_ptr(&trunk_a));
        assert_eq!(Arc::as_ptr(&second.trunk()), Arc::as_ptr(&trunk_b));
        assert_ne!(Arc::as_ptr(&first.trunk()), Arc::as_ptr(&second.trunk()));
    }

    fn video_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Vp8 {
                width: 1280,
                height: 720,
            },
        )
    }

    fn seg(seq: u32, duration: f64) -> transmux::ll_hls::SegmentInfo {
        transmux::ll_hls::SegmentInfo {
            bytes: vec![seq as u8; 8],
            duration,
            segment_seq: seq,
            part_count: 1,
        }
    }

    /// **The headline per-program-serving-state test (issue #805 task 6).**
    /// Two programs on one route each get their own init bytes and their own
    /// closed segment — proving the per-program `ProgramServing` bundle is
    /// real, per-`ProgramId` state, not one shared slot that the second
    /// `publish_program` call would silently overwrite (the exact bug the old
    /// single owned-`Trunk`-plus-`ll_hls`/`dash` design could not avoid once
    /// a second program appeared on the same route).
    ///
    /// MUTATION VERIFIED: changing `RouteHandle::set_init` to always resolve
    /// `SPTS_PROGRAM_ID` regardless of the `program` argument passed in (i.e.
    /// `self.serving(SPTS_PROGRAM_ID)` instead of `self.serving(program)`)
    /// makes this test fail — but at the **first** assertion, not the second:
    /// this test uses `ProgramId(1)`/`ProgramId(2)` (never `ProgramId(0)` ==
    /// `SPTS_PROGRAM_ID`), so the mutated `set_init` resolves a program that
    /// was never published at all, and *both* `set_init` calls silently no-op
    /// (logged, not panicking — see this method's own doc). The first
    /// assertion, `assert_eq!(route.init_bytes(program_a),
    /// Some(Bytes::from_static(b"init-1")), "program 1 must serve its own
    /// init bytes")`, is therefore what actually fails: actual `None` vs.
    /// expected `Some(b"init-1")` (verbatim panic: `assertion `left == right`
    /// failed: program 1 must serve its own init bytes / left: None / right:
    /// Some(b"init-1")`) — the second assertion (program 2) never runs.
    /// Recompiled and re-run to confirm this exact failure and which
    /// assertion fires, then reverted.
    #[test]
    fn two_programs_serve_distinct_media() {
        let route = RouteHandle::new(1.0, 500, 8);
        let program_a = ProgramId(1);
        let program_b = ProgramId(2);

        route.publish_new_program(program_a);
        route.publish_new_program(program_b);

        route.set_init(program_a, &b"init-1"[..]);
        route.set_init(program_b, &b"init-2"[..]);
        route.set_track_specs(program_a, vec![video_spec(1)]);
        route.set_track_specs(program_b, vec![video_spec(2)]);
        route.add_segment(program_a, seg(10, 2.0));
        route.add_segment(program_b, seg(20, 2.0));
        route.add_segment(program_b, seg(21, 2.0));

        assert_eq!(
            route.init_bytes(program_a),
            Some(Bytes::from_static(b"init-1")),
            "program 1 must serve its own init bytes"
        );
        assert_eq!(
            route.init_bytes(program_b),
            Some(Bytes::from_static(b"init-2")),
            "program 2 must serve its own, DISTINCT init bytes"
        );

        let window_a = route.window_segments(program_a);
        let window_b = route.window_segments(program_b);
        assert_eq!(
            window_a.iter().map(|s| s.segment_seq).collect::<Vec<_>>(),
            vec![10],
            "program 1 must show only its own segment"
        );
        assert_eq!(
            window_b.iter().map(|s| s.segment_seq).collect::<Vec<_>>(),
            vec![20, 21],
            "program 2 must show only its own (different-count) segments"
        );

        assert_eq!(
            route
                .track_specs(program_a)
                .iter()
                .map(|s| s.track_id)
                .collect::<Vec<_>>(),
            vec![1],
            "program 1's own track spec"
        );
        assert_eq!(
            route
                .track_specs(program_b)
                .iter()
                .map(|s| s.track_id)
                .collect::<Vec<_>>(),
            vec![2],
            "program 2's own, DIFFERENT track spec"
        );
    }

    /// A request for a program that has never been announced on this route
    /// resolves `NotYetAnnounced` — the wait/503 case — never `NotFound`
    /// (404), matching `crate::http::resolve_route_program`'s mapping. See
    /// `crate::output::llhls`'s/`crate::origin::resource`'s own
    /// `*_not_yet_announced_is_503_not_404` tests for the same property
    /// proven through the real HTTP handlers.
    #[test]
    fn unannounced_program_is_not_yet_announced_not_not_found() {
        let route = RouteHandle::new(4.0, 500, 4);
        assert!(matches!(
            route.resolve_program(ProgramId(7)),
            ProgramResolution::NotYetAnnounced
        ));
    }

    /// `RouteHandle::name` defaults to the documented sentinel and reflects
    /// whatever `with_name` set.
    #[test]
    fn name_defaults_then_reflects_with_name() {
        let route = RouteHandle::new(4.0, 500, 4);
        assert_eq!(route.name(), DEFAULT_ROUTE_NAME);
        let route = route.with_name("cam1");
        assert_eq!(route.name(), "cam1");
    }
}
