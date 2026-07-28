//! One route's shared state: the `media_plane::Trunk` every output reads
//! from, the sans-IO LL-HLS origin over it
//! ([`ll_hls_runtime::server::LlHlsOrigin`], plan step 4), the small
//! `Trunk`-drained window DASH/LL-DASH need beyond what any `Trunk` ring
//! holds (`DashState`, below), and this route's [`HealthState`].
//!
//! Replaces the deleted `ll_hls_runtime::server::{MediaStore, HealthState}`
//! (step 5b): `MediaStore`'s playlist-rendering half moved into
//! [`LlHlsOrigin`] (step 4, reusing it rather than reimplementing it here —
//! see this crate's `http` module docs); its DASH-only fields
//! (`track_specs`/`created_at`/the closed-segment window) have no `Trunk`
//! ring of their own (a `Trunk` carries samples/segments/parts/events, never
//! codec metadata or a route's start time) and are recreated here, in
//! exactly the same "one small synced window, fed by draining one
//! `SegmentCursor`, never a second cache of the `Trunk`'s own data" shape
//! [`LlHlsOrigin`]'s own `Window` already established — see that type's
//! module doc, "The one thing that genuinely cannot come from the `Trunk`
//! alone". [`HealthState`] (route up/down) is also recreated here: it is
//! *not* [`media_plane::ingress::HealthState`], which is generic over one
//! ingest session's own error type and cannot give a homogeneous "is this
//! route live" answer across routes fed by different connectors.

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use broadcast_common::Timestamp;
use bytes::Bytes;
use ll_hls_runtime::server::LlHlsOrigin;
use media_plane::trunk::{
    PartEntry, SegmentCursor, SegmentCursorItem, SegmentEntry, SegmentWriter, TrunkConfig,
};
use media_plane::{ProgramId, Trunk};
use transmux::TrackSpec;

/// Ring capacities [`RouteHandle::new`] gives the `Trunk` it builds, beyond
/// [`crate::config::Config::window_segments`] (which becomes the segment
/// log's own capacity — the same "advertised window == retained window"
/// depth the deleted `MediaStore` gave a route). None of these three rings
/// (timed/sparse samples, events) are used by the `LlHlsSegmenter`-fed
/// pipeline this handle serves today (it publishes finished
/// segments/parts directly, never raw samples through
/// [`media_plane::TrunkWriter`]) — sized generously rather than at `1` so a
/// future sample-level consumer of the same `Trunk` (a `PushEgress` tap, a
/// DVR `SegmentEgress`) has real headroom without a config change.
const DEFAULT_TIMED_CAPACITY: usize = 64;
const DEFAULT_SPARSE_CAPACITY: usize = 16;
const DEFAULT_EVENT_CAPACITY: usize = 64;
/// Live-part log / concurrent-waiter capacity (`TrunkConfig::part_capacity`
/// shares both jobs — see that field's own doc). Generous relative to how
/// many parts one open segment realistically holds.
const DEFAULT_PART_CAPACITY: usize = 64;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("route.rs capacity constants are all non-zero")
}

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
    /// Lost its connection/pipeline and is retrying with backoff — never
    /// permanent; see `crate::origin::supervisor::supervise`'s own doc for
    /// why a route always retries rather than giving up.
    Reconnecting,
    /// Reserved for an unrecoverable error class a future caller may want to
    /// distinguish from an ordinary, retried [`HealthState::Reconnecting`] —
    /// `supervise`'s loop does not currently produce it.
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
/// Deliberately **not** an `Option<Arc<Trunk>>`: a bare `Option` can only say
/// "here it is" or "no" — it cannot distinguish two `None`-shaped situations
/// that need opposite egress treatment. "No program has been announced on
/// this route at all yet" (ingest may still be dialing/connecting/
/// negotiating — a **transient** condition) and "this route has at least one
/// live program, but not the one that was asked for" (a **permanent**
/// condition — the requested [`ProgramId`] will not spring into existence
/// later) look identical as `None`, but the first should make a caller wait
/// or answer not-ready (exactly [`media_plane::EgressResponse::Await`]'s own
/// case), while the second is a genuine 404. Collapsing them would make an
/// ordinary SPTS route mid-connect indistinguishable from a client
/// misaddressing an MPTS program that will never exist on this route.
///
/// Not given a `name()`/`Display` under the workspace's #204 spec/field-enum
/// label convention: like [`media_plane::EgressResponse`] (whose module doc
/// gives the same rationale, and whose crate's `tests/label_coverage.rs`
/// skip-list names it for exactly this reason), this is a **data-carrying**
/// control-flow ADT — [`ProgramResolution::Found`] carries the resolved
/// `Arc<Trunk>` itself — not a static token decoded from a cited spec; a
/// caller already matches the typed variant instead of formatting a label.
/// `multimux` has no `tests/label_coverage.rs` drift-guard of its own yet
/// (unlike every sibling crate) to enforce or record that exemption
/// mechanically — see this task's own report.
#[non_exhaustive]
#[allow(dead_code)] // `Found`'s payload is read once an egress caller lands (issue #805)
pub(crate) enum ProgramResolution {
    /// `program` is registered; here is its `Trunk`.
    Found(Arc<Trunk>),
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
/// [`LlHlsOrigin`]'s own `Window` (or the `Trunk`'s segment log itself)
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
/// still has exactly one drained window, not two independently-lagging
/// copies.
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
        *self.track_specs.lock().unwrap() = specs;
    }

    fn track_specs(&self) -> Vec<TrackSpec> {
        self.track_specs.lock().unwrap().clone()
    }

    /// Absorb every segment this route's `SegmentCursor` has produced since
    /// the last call — the same non-blocking, called-at-the-top-of-render
    /// shape as [`LlHlsOrigin`]'s own `drain` (see that type's module doc).
    fn drain(&self) {
        let mut cursor = self.cursor.lock().unwrap();
        let mut window = self.window.lock().unwrap();
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
        self.window.lock().unwrap().iter().copied().collect()
    }
}

/// One route's shared state — replaces the deleted `MediaStore`. Built once
/// per configured route ([`crate::origin::serve_with_registry`]), then
/// shared (via `Arc`) between whatever ingest publishes into it
/// ([`crate::pipeline::run_pipeline`]) and every [`crate::output::Output`]/
/// the shared resource route that reads from it.
pub struct RouteHandle {
    trunk: Arc<Trunk>,
    segment_writer: SegmentWriter,
    ll_hls: Arc<LlHlsOrigin>,
    dash: Arc<DashState>,
    health: Mutex<HealthState>,
    /// Cumulative nanoseconds of segment duration published so far — the
    /// only honest source for [`media_plane::trunk::SegmentEntry::timeline_position`]
    /// this `LlHlsSegmenter`-fed pipeline has (it never itself computes an
    /// absolute-timeline position; see `crate::pipeline::run_pipeline`).
    next_timeline_ns: AtomicU64,
    target_duration_secs: f64,
    part_target_ms: u32,
    created_at: SystemTime,
    /// Egress-resolvable `ProgramId -> Arc<Trunk>` registry — the additive
    /// step (issue #805 task 1) toward this crate's converged architecture:
    /// `media_plane::IngestDriver` mints one `Trunk` per program the instant
    /// it observes `media_plane::SessionEvent::NewProgram`, while this
    /// `RouteHandle` is built *before* any program is known
    /// (`crate::origin::serve_with_registry` constructs one per configured
    /// route, ahead of dialing/listening). This map is how those two
    /// lifetimes reconcile without forcing either one to change when the
    /// other becomes ready: [`RouteHandle::publish_program`] is the ingest
    /// side's write, [`RouteHandle::resolve_program`] is the egress side's
    /// read. Additive only, for now — the still-owned `trunk` field above is
    /// untouched, and nothing yet calls either new method (a later task
    /// wires ingest publish + migrates the five `trunk()` call sites).
    ///
    /// A `HashMap`, not a single slot: MPTS carries an unbounded (bounded
    /// only by `media_plane::IngestDriver::max_programs`) number of programs
    /// per one route/`RouteHandle`, so the registry's shape must already be
    /// "N programs" even though every route wired up so far is SPTS (exactly
    /// one program). A later, MPTS-aware egress path resolves a request to
    /// whichever entry its request identifies (e.g. a program number parsed
    /// from the request path or a query parameter) by calling
    /// `resolve_program` with that `ProgramId` — this map does not need to
    /// change shape to support that; only the request-to-`ProgramId` mapping
    /// on the egress side is new work for that later task.
    ///
    /// # Why `RwLock<HashMap<..>>`, not `Mutex` (unlike this struct's
    /// `Mutex<HealthState>` field, above)
    ///
    /// [`RouteHandle::resolve_program`] is the hottest read path this handle
    /// will expose once wired to egress: unlike `health`
    /// (touched once per connection-state transition) or `next_timeline_ns`
    /// (touched once per produced segment), a route's program registry would
    /// be read on *every* served HTTP request, across however many
    /// concurrent viewers that route has. A `Mutex` would serialize all of
    /// those mutually non-conflicting reads against one another for no
    /// reason; `RwLock` lets an unbounded number of concurrent readers
    /// proceed together and blocks only around the rare write — at most once
    /// per announced program, bounded by `max_programs`, never a
    /// steady-state cost.
    programs: RwLock<HashMap<ProgramId, Arc<Trunk>>>,
}

impl RouteHandle {
    /// Build a fresh route: a new `Trunk` sized by `window_segments` (its
    /// segment-log capacity, matching the deleted `MediaStore`'s "advertised
    /// window == retained window" depth), the `LlHlsOrigin` over it, and this
    /// route's `DashState`.
    pub fn new(target_duration_secs: f64, part_target_ms: u32, window_segments: usize) -> Self {
        let window_segments = NonZeroUsize::new(window_segments).unwrap_or(NonZeroUsize::MIN);
        let trunk_config = TrunkConfig::new(
            nz(DEFAULT_TIMED_CAPACITY),
            nz(DEFAULT_SPARSE_CAPACITY),
            window_segments,
            nz(DEFAULT_EVENT_CAPACITY),
            nz(DEFAULT_PART_CAPACITY),
        );
        let trunk = Trunk::new(trunk_config);
        let segment_writer = trunk
            .segment_writer()
            .expect("RouteHandle takes the Trunk's one segment writer at construction");
        let ll_hls = Arc::new(LlHlsOrigin::new(
            Arc::clone(&trunk),
            target_duration_secs,
            part_target_ms,
            window_segments,
        ));
        let dash = Arc::new(DashState::new(&trunk, window_segments));
        RouteHandle {
            trunk,
            segment_writer,
            ll_hls,
            dash,
            health: Mutex::new(HealthState::Connecting),
            next_timeline_ns: AtomicU64::new(0),
            target_duration_secs,
            part_target_ms,
            created_at: SystemTime::now(),
            programs: RwLock::new(HashMap::new()),
        }
    }

    /// The shared `Trunk` — used by the axum adapter (`crate::http`) to
    /// register a bounded wake-up ([`Trunk::listen`]) while a request is
    /// genuinely blocked.
    pub(crate) fn trunk(&self) -> &Arc<Trunk> {
        &self.trunk
    }

    /// Publish `program`'s `Trunk` into this route's registry — the ingest
    /// side's write, called once `media_plane::IngestDriver` (or
    /// `ListenDriver`) reports `media_plane::SessionEvent::NewProgram` and
    /// mints a `Trunk` for it (a later task wires an actual caller; see
    /// `crate::origin::serve_with_registry`'s currently-stubbed match arm).
    ///
    /// Overwrites any prior entry for the same `ProgramId` outright — the
    /// same "last write wins" semantics `HashMap::insert` always has.
    /// `IngestDriver` itself keeps one stable `Trunk` per `ProgramId` for the
    /// life of a session (a repeat `NewProgram` for an already-known program
    /// does not mint a second `Trunk`), so in practice this is called once
    /// per program, not repeatedly with different `Trunk`s for the same key.
    #[allow(dead_code)] // wired to an ingest caller in a later task (issue #805)
    pub(crate) fn publish_program(&self, program: ProgramId, trunk: Arc<Trunk>) {
        self.programs
            .write()
            .expect("RouteHandle::programs lock poisoned")
            .insert(program, trunk);
    }

    /// Resolve `program` against this route's registry — the egress side's
    /// read (see [`ProgramResolution`] for why this returns a three-way enum
    /// rather than `Option<Arc<Trunk>>`, and this struct's `programs` field
    /// doc for why the lock is a `RwLock`). Not yet called by any egress
    /// path — a later task migrates `crate::output`/`crate::origin::resource`
    /// off the still-owned `trunk()` accessor onto this method.
    #[allow(dead_code)] // wired to an egress caller in a later task (issue #805)
    pub(crate) fn resolve_program(&self, program: ProgramId) -> ProgramResolution {
        let programs = self
            .programs
            .read()
            .expect("RouteHandle::programs lock poisoned");
        match programs.get(&program) {
            Some(trunk) => ProgramResolution::Found(Arc::clone(trunk)),
            None if programs.is_empty() => ProgramResolution::NotYetAnnounced,
            None => ProgramResolution::NotFound,
        }
    }

    /// The sans-IO LL-HLS origin every output's init/segment/part bytes (and
    /// LL-HLS's own playlist) resolve through.
    pub(crate) fn ll_hls(&self) -> &Arc<LlHlsOrigin> {
        &self.ll_hls
    }

    /// Store the fMP4 init segment bytes — forwards to [`LlHlsOrigin::set_init`].
    pub fn set_init(&self, bytes: impl Into<Bytes>) {
        self.ll_hls.set_init(bytes);
    }

    /// The fMP4 init segment bytes, if set.
    pub fn init_bytes(&self) -> Option<Bytes> {
        self.ll_hls.init_bytes()
    }

    /// Record this route's track specs (issue #663 P4: DASH's `codecs`
    /// string derivation) — the one piece of codec metadata no `Trunk` ring
    /// holds.
    pub fn set_track_specs(&self, specs: Vec<TrackSpec>) {
        self.dash.set_track_specs(specs);
    }

    /// This route's recorded track specs, if any.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.dash.track_specs()
    }

    /// Publish one live part (`transmux::ll_hls::LlHlsSegmenter::take_ready_parts`)
    /// into the `Trunk`'s live-part log.
    pub fn add_part(&self, info: transmux::ll_hls::PartInfo) {
        self.segment_writer.publish_part(PartEntry::new(
            info.bytes,
            info.segment_seq,
            info.part_index,
            Duration::from_secs_f64(info.duration),
            info.independent,
        ));
    }

    /// Publish one finished segment (`transmux::ll_hls::LlHlsSegmenter::take_ready_segments`)
    /// into the `Trunk`'s segment log. `timeline_position` is derived from a
    /// running total of every prior segment's duration — see this route's
    /// `next_timeline_ns` field, below.
    pub fn add_segment(&self, info: transmux::ll_hls::SegmentInfo) {
        let duration = Duration::from_secs_f64(info.duration);
        let start_ns = self.next_timeline_ns.fetch_add(
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        self.segment_writer.publish_segment(SegmentEntry::new(
            info.bytes,
            info.segment_seq,
            duration,
            Timestamp::from_nanos(start_ns),
            transmux::SegmentMeta {
                discontinuous: false,
            },
        ));
    }

    /// The currently-resident closed-segment window, oldest first — the
    /// `Trunk`-drained replacement for `MediaStore::window_segments`.
    pub fn window_segments(&self) -> Vec<DashWindowSegment> {
        self.dash.window_segments()
    }

    /// Configured target full-segment duration, seconds.
    pub fn target_duration_secs(&self) -> f64 {
        self.target_duration_secs
    }

    /// Configured LL-HLS part target, milliseconds.
    pub fn part_target_ms(&self) -> u32 {
        self.part_target_ms
    }

    /// When this route was constructed — `MPD@availabilityStartTime`'s
    /// source (`crate::output::dash`/`crate::output::ll_dash`); not
    /// something any `Trunk` ring tracks (a `Trunk` has no wall-clock
    /// concept of its own construction time).
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// This route's current [`HealthState`].
    pub fn health(&self) -> HealthState {
        *self.health.lock().unwrap()
    }

    /// Transition this route's [`HealthState`].
    pub fn set_health(&self, state: HealthState) {
        *self.health.lock().unwrap() = state;
    }

    /// `(in-progress-or-last-closed segment sequence number, its currently
    /// resident live-part count)` — mirrors
    /// `ll_hls_runtime::server::LlHlsOrigin::live_edge`'s own derivation,
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
}

#[cfg(test)]
mod program_registry_tests {
    //! Coverage for `RouteHandle::publish_program`/`resolve_program` (issue
    //! #805 task 1) — the additive `ProgramId -> Arc<Trunk>` registry. Each
    //! `MUTATION VERIFIED` doc comment below records a real edit made to
    //! `route.rs`'s production code, a re-run of that specific test to
    //! confirm the named assertion failed with the stated actual-vs-expected
    //! values, and the subsequent revert.

    use super::*;

    /// A minimal standalone `Trunk`, distinct in identity from any other
    /// call's — every ring capacity is `1` since these tests never publish
    /// samples/segments/parts/events, only check `Arc` identity through the
    /// registry.
    fn test_trunk() -> Arc<Trunk> {
        Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1)))
    }

    /// MUTATION VERIFIED: changing `publish_program` to insert a
    /// freshly-constructed `Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1),
    /// nz(1), nz(1)))` instead of the `trunk` argument it was actually
    /// given (i.e. `programs.write().unwrap().insert(program,
    /// Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1))))`)
    /// makes this test fail: `resolve_program`'s `Found` arm still returns
    /// `Some`, but `assert_eq!(Arc::as_ptr(&resolved), Arc::as_ptr(&trunk))`
    /// fails, comparing two genuinely different heap addresses (e.g.
    /// `0x1055f3ab0 != 0x1055f4120` — exact addresses vary per run) because
    /// the registry silently substituted a different `Trunk` for the one
    /// published. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn publish_then_resolve_returns_same_arc() {
        let route = RouteHandle::new(4.0, 500, 4);
        let trunk = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk));

        match route.resolve_program(ProgramId(1)) {
            ProgramResolution::Found(resolved) => {
                assert_eq!(Arc::as_ptr(&resolved), Arc::as_ptr(&trunk));
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
    /// `assert_eq!(Arc::as_ptr(&second), Arc::as_ptr(&trunk_b))` fails,
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
            ProgramResolution::Found(t) => t,
            _ => panic!("expected ProgramResolution::Found for program 1"),
        };
        let second = match route.resolve_program(ProgramId(2)) {
            ProgramResolution::Found(t) => t,
            _ => panic!("expected ProgramResolution::Found for program 2"),
        };

        assert_eq!(Arc::as_ptr(&first), Arc::as_ptr(&trunk_a));
        assert_eq!(Arc::as_ptr(&second), Arc::as_ptr(&trunk_b));
        assert_ne!(Arc::as_ptr(&first), Arc::as_ptr(&second));
    }
}
