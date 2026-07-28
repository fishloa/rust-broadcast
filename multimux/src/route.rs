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

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use broadcast_common::Timestamp;
use bytes::Bytes;
use ll_hls_runtime::server::LlHlsOrigin;
use media_plane::Trunk;
use media_plane::trunk::{
    PartEntry, SegmentCursor, SegmentCursorItem, SegmentEntry, SegmentWriter, TrunkConfig,
};
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
        }
    }

    /// The shared `Trunk` — used by the axum adapter (`crate::http`) to
    /// register a bounded wake-up ([`Trunk::listen`]) while a request is
    /// genuinely blocked.
    pub(crate) fn trunk(&self) -> &Arc<Trunk> {
        &self.trunk
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
    /// running total of every prior segment's duration — see
    /// [`Self::next_timeline_ns`].
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
