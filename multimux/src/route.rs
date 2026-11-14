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
//! `crate::output::hls`/`dash`/`ll_dash`, `crate::origin::resource`)
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
//!    (`crate::output::hls::BlockingReloadQuery` already does this for
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

use parking_lot::{Mutex, RwLock};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
#[cfg(test)]
use transmux::TrackId;

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{HlsOrigin, HlsOriginConfig};
use media_plane::trunk::{
    PartEntry, SegmentCursor, SegmentCursorItem, SegmentEntry, SegmentWriter, TrunkConfig,
};
use media_plane::{ProgramId, Trunk};
use transmux::Container;
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

/// Default for [`RouteHandle::with_max_concurrent_warms`].
pub const DEFAULT_MAX_CONCURRENT_WARMS: usize = 8;

/// [`RouteConfig`]'s default advertised-window depth (segments), and so the
/// default segment capacity of every trunk a route builds.
pub const DEFAULT_WINDOW_SEGMENTS: usize = 8;

/// [`RouteHandle::name`]'s value until [`RouteHandle::with_name`] sets a real
/// one — matches `crate::origin::mod`'s own `"unknown"` fallback token for an
/// unrecognized route in its HTTP-path metrics labelling, so a route driven
/// directly (skipping `crate::Server`, e.g. a bare unit
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
    /// Lost its connection/ingest and is retrying with backoff. Transient by
    /// definition — a failure that cannot resolve itself becomes
    /// [`HealthState::Failed`] instead; see
    /// `crate::origin::supervisor::supervise_driver`.
    Reconnecting,
    /// Stopped retrying because the failure cannot resolve itself: repeated
    /// RTSP authentication rejections (a wrong password never becomes right)
    /// or a `404` on DESCRIBE (a wrong path never appears). Issue #957 — the
    /// supervisor previously retried these forever, so an operator typo
    /// produced an endless warn-level loop instead of a visible failure.
    ///
    /// Surfaced through the admin API's route status, so it is diagnosable
    /// without reading logs.
    Failed,
    /// The source finished cleanly and will not produce more media (an HLS
    /// playlist that reached `#EXT-X-ENDLIST`, a static DASH/Smooth manifest).
    /// The route keeps serving what it published and is not reconnected —
    /// re-pulling a finite source only repeats the same content.
    Ended,
}

impl HealthState {
    /// Whether the route has media to serve: still being produced
    /// ([`HealthState::Live`]) or finished and retained ([`HealthState::Ended`]).
    pub fn is_serving(&self) -> bool {
        matches!(self, HealthState::Live | HealthState::Ended)
    }

    /// The spec/field-enum label (workspace #204 convention).
    pub fn name(&self) -> &'static str {
        match self {
            HealthState::Connecting => "connecting",
            HealthState::Live => "live",
            HealthState::Reconnecting => "reconnecting",
            HealthState::Failed => "failed",
            HealthState::Ended => "ended",
        }
    }
}

broadcast_common::impl_spec_display!(HealthState);

/// Why a classic-TS output is not IDR-aligned: the stream carried no IDR
/// picture within the route's wait bound
/// ([`crate::config::TsHlsIdrFallback::bound`]). Open-GOP H.264 (recovery-point
/// SEI, no type-5 NAL) is the usual cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TsHlsIdrStatus {
    /// The output fell back to cutting on non-IDR random-access points;
    /// segments do not all start on an IDR (Apple HLS Authoring Spec 7.4).
    FellBack,
    /// Fallback is off (`"never"`): the output serves no segment until an IDR
    /// arrives.
    Awaiting,
}

impl TsHlsIdrStatus {
    /// The admin-status and metric-label token.
    pub fn name(&self) -> &'static str {
        match self {
            TsHlsIdrStatus::FellBack => "fell_back",
            TsHlsIdrStatus::Awaiting => "awaiting",
        }
    }
}

broadcast_common::impl_spec_display!(TsHlsIdrStatus);

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
/// gives the same rationale, and whose crate's `workspace-guards/tests/guards/label.rs`
/// skip-list names it for exactly this reason), this is a **data-carrying**
/// control-flow ADT — [`ProgramResolution::Found`] carries the resolved
/// [`ProgramServing`] bundle itself — not a static token decoded from a cited
/// spec; a caller already matches the typed variant instead of formatting a
/// label. `multimux` has no `workspace-guards/tests/guards/label.rs` drift-guard of its own
/// yet (unlike every sibling crate) to enforce or record that exemption
/// mechanically — see this task's own report.
///
/// Read by every migrated egress call site (`crate::output::hls`/`dash`/
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
    pub segment_seq: media_plane::ids::SegmentSeq,
    /// Matches [`media_plane::trunk::SegmentEntry::duration`], as seconds.
    pub duration_secs: f64,
    /// This segment's **absolute** start on this program's own monotonic
    /// timeline, in nanoseconds — [`media_plane::trunk::SegmentEntry::timeline_position`],
    /// which starts at `0` for the program's first segment and never resets as
    /// the window slides. Needed by the Smooth output, whose `c@t`/`tfxd` must
    /// be stable for a given segment as the window advances (issue #1083, W13
    /// follow-up): the window-relative cumulative sum changes on every slide,
    /// this does not.
    pub timeline_position_ns: u64,
    /// The number the shared resource route and the HLS playlist show this
    /// segment under: the `Trunk`'s own number plus the offset the
    /// route carries over from the program a reconnect replaced, so it keeps
    /// counting up across a source reconnect.
    pub public_seq: u64,
    /// The segment is a gap placeholder (no media bytes): the program had no
    /// muxable track while it was cut (`#EXT-X-GAP` on HLS, the end of a DASH
    /// Period).
    pub gap: bool,
    /// The init-segment generation this segment was cut against, for versioned
    /// init URLs (issue #2441).
    pub init_generation: media_plane::ids::Generation,
    /// The DASH Period this segment is listed in; `None` only for a gap placeholder cut before
    /// the program's first Period opened.
    pub(crate) period: Option<PeriodAnchor>,
    /// [`HlsOrigin::instance`] of the program that cut this segment: this program's own, or a
    /// predecessor's whose segments are still in the DASH window (see
    /// [`ProgramServing::dash_window_segments`]).
    pub(crate) instance: u64,
}

/// The identity of one DASH Period, fixed when the Period opens and never changed by an MPD
/// update: ISO/IEC 23009-1 §5.4.1 keeps `Period@id` across updates, and the Period's
/// `@start`, `SegmentTemplate@startNumber` and `@presentationTimeOffset` are derived from its
/// first segment, so neither may move as the window slides past that segment
/// (`private/specs-verified/dash/23009-1/05-3-2-period.md` §5.3.2.1/§5.3.2.2,
/// `private/specs-verified/dash/23009-1/05-4_to_5-7.md` §5.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PeriodAnchor {
    /// How many Periods the presentation opened before this one: `Period@id`.
    pub(crate) ordinal: u64,
    /// The public number of the Period's first segment: `SegmentTemplate@startNumber`.
    pub(crate) first_public_seq: u64,
    /// The media time of that segment's earliest sample, nanoseconds:
    /// `@presentationTimeOffset` once scaled to the Representation's timescale.
    pub(crate) media_start_ns: u64,
    /// The ordinal of the Period this one is period-connected to (ISO/IEC 23009-1 §5.3.2.4):
    /// the previous Period, when this one opened only because the numbering skipped the
    /// replaced program's unfinished segment and the init segment stayed the same.
    pub(crate) connected_to: Option<u64>,
    /// The ordinal of the first Period of the run of period-connected Periods this one belongs
    /// to, which names their shared asset (§5.8.4.10); `None` for a Period connected to none.
    pub(crate) asset: Option<u64>,
}

/// Where a program's DASH Periods stand: the Period now open and what the last segment was cut
/// under. A media segment opens a new Period after a gap placeholder, when its init generation
/// differs from the previous segment's (a Period has one fixed set of encoded versions,
/// ISO/IEC 23009-1 §5.3.2.4), or when it is a continued program's first segment and the
/// numbering skipped the replaced program's unfinished segment, which no origin serves; otherwise
/// it joins the open one.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PeriodCursor {
    anchor: Option<PeriodAnchor>,
    /// `(gap, init generation, public number)` of the last segment drained.
    last: Option<(bool, media_plane::ids::Generation, u64)>,
    /// The next segment does not follow `last`'s number: the open Period ends at `last`.
    skipped_numbers: bool,
}

impl PeriodCursor {
    /// The Period a media segment `segment` (`(public number, media start ns)`), cut under
    /// `generation`, would be listed in. `program_start` is the program's first segment: the
    /// presentation's first Period opens there even when the segments before `segment` were
    /// never drained (the `Trunk` evicted them before an MPD was rendered).
    ///
    /// A Period opened only because numbers were skipped keeps the init segment, so it is
    /// period-connected to the open one (§5.3.2.4: equivalent Initialization Segments, timestamps
    /// non-continuous across the skipped segment; DASH-IF IOP 4.3 4.11.4.2).
    fn anchor_for(
        &self,
        generation: media_plane::ids::Generation,
        segment: (u64, u64),
        program_start: (u64, u64),
    ) -> PeriodAnchor {
        let same_init = matches!(self.last, Some((false, last, _)) if last == generation);
        let (first_public_seq, media_start_ns) = if self.last.is_none() {
            program_start
        } else {
            segment
        };
        match self.anchor {
            Some(open) if same_init && !self.skipped_numbers => open,
            previous => {
                let connected = previous.filter(|_| same_init);
                PeriodAnchor {
                    ordinal: previous.map_or(0, |p| p.ordinal.saturating_add(1)),
                    first_public_seq,
                    media_start_ns,
                    connected_to: connected.map(|p| p.ordinal),
                    asset: connected.map(|p| p.asset.unwrap_or(p.ordinal)),
                }
            }
        }
    }

    fn advance(
        &mut self,
        gap: bool,
        generation: media_plane::ids::Generation,
        segment: (u64, u64),
        program_start: (u64, u64),
    ) -> Option<PeriodAnchor> {
        if !gap {
            self.anchor = Some(self.anchor_for(generation, segment, program_start));
        }
        self.last = Some((gap, generation, segment.0));
        self.skipped_numbers = false;
        self.anchor
    }

    /// This cursor as the successor program sees it: its origin numbers init generations from
    /// [`SEEDED_INIT_GENERATION`] again, with the predecessor's last init seeded as that
    /// generation (see [`ProgramServing::new`]), and its first segment is `first_public_seq`.
    fn continued(self, first_public_seq: u64) -> Self {
        PeriodCursor {
            anchor: self.anchor,
            last: self
                .last
                .map(|(gap, _, number)| (gap, SEEDED_INIT_GENERATION, number)),
            skipped_numbers: self
                .last
                .is_some_and(|(_, _, number)| number.saturating_add(1) != first_public_seq),
        }
    }
}

/// The init generation a fresh origin gives its first init segment: the predecessor's seeded
/// init on a continued program.
const SEEDED_INIT_GENERATION: media_plane::ids::Generation = media_plane::ids::Generation::new(1);

/// What a replacement program continues from its predecessor on the same
/// route: the public segment numbering ([`HlsOrigin`]'s media sequence
/// offset, the DVR archive's index, DASH `startNumber`, Smooth fragment
/// names) and the program timeline (`DashState`'s window and the DVR's
/// `start_pts_ns`), which a fresh `Trunk` would otherwise restart from
/// segment 1 and position 0. One value per published program, computed once
/// in [`RouteHandle::publish_program`] so no output keeps its own copy.
#[derive(Debug, Clone, Copy)]
pub struct ProgramEpoch {
    pub(crate) seq_offset: u64,
    /// Where this program's media timeline starts: the time the DASH
    /// manifest derives for its first number, `(first public number -
    /// startNumber)` nominal segments, so the segment bytes, the manifests
    /// and the DVR agree. `0` for the first program.
    pub media_start_ns: u64,
    /// The previous origin's Discontinuity Sequence Number, so
    /// `EXT-X-DISCONTINUITY-SEQUENCE` never decreases across a reconnect.
    pub(crate) discontinuity_sequence: u64,
    /// The first public number the route ever served: DASH `startNumber`.
    pub(crate) first_number: u64,
    /// The route's first presentation start: DASH `availabilityStartTime`,
    /// which an update of a dynamic MPD must not change
    /// (`private/specs-verified/dash/etsi-ts-103-285/ch04-dash-constraints-http-sync.md` 4.8.3).
    pub(crate) presentation_origin: SystemTime,
}

/// What a program's successor needs of it to continue its numbering and timeline: the next media
/// sequence number, the discontinuity sequence of its newest segment, its epoch, its init segment
/// and where its DASH Periods stand. Taken from
/// the live [`ProgramServing`] on a reconnect, or from the route this one replaces
/// ([`RouteHandle::continue_from`]).
#[derive(Clone)]
pub(crate) struct ProgramContinuation {
    next_media_sequence: u64,
    discontinuity_sequence_after: u64,
    epoch: ProgramEpoch,
    /// The predecessor's current init segment.
    init: Option<Bytes>,
    /// The predecessor's DASH Period position after its last segment.
    period: PeriodCursor,
    /// The predecessor's DASH window, its own segments and any it still lists of earlier ones.
    window: Vec<DashWindowSegment>,
    /// The programs that cut the segments of `window`, which keep serving them.
    servings: Vec<Arc<ProgramServing>>,
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
    track_specs: Mutex<Vec<(u32, Vec<TrackSpec>)>>,
    /// Bumped by every change to `track_specs`; part of an MPD cache stamp.
    track_epoch: AtomicU64,
    /// Wall-clock instant the route's presentation began: when its first
    /// `Trunk` was minted, carried unchanged across a reconnect. The source of
    /// `MPD@availabilityStartTime` (ISO/IEC 23009-1 §5.3.1.2 Table 3: "the
    /// anchor for the MPD in wall-clock time", the time from which every
    /// segment's availability is measured, §5.3.9.5.3) — NOT the route's
    /// construction time, which precedes ingest (#1187).
    presentation_origin: SystemTime,
    epoch: ProgramEpoch,
    period: Mutex<PeriodCursor>,
    /// [`HlsOrigin::instance`] of this program's origin.
    instance: u64,
}

impl DashState {
    /// `inherited` is the predecessor's DASH window: its newest segments stay listed (and age
    /// out as this program's segments arrive) so a Period whose segments are still inside the
    /// time-shift buffer is not withdrawn (ISO/IEC 23009-1 §5.3.9.5.3, §5.4.1).
    fn new(
        trunk: &Arc<Trunk>,
        window_segments: NonZeroUsize,
        epoch: ProgramEpoch,
        period: PeriodCursor,
        instance: u64,
        inherited: &[DashWindowSegment],
    ) -> Self {
        let capacity = window_segments.get();
        let kept = &inherited[inherited.len().saturating_sub(capacity)..];
        DashState {
            epoch,
            period: Mutex::new(period),
            instance,
            cursor: Mutex::new(trunk.subscribe_segments()),
            window: Mutex::new(kept.iter().copied().collect()),
            capacity,
            track_specs: Mutex::new(Vec::new()),
            track_epoch: AtomicU64::new(0),
            presentation_origin: epoch.presentation_origin,
        }
    }

    fn set_track_specs(&self, specs: Vec<TrackSpec>) {
        let mut log = self.track_specs.lock();
        match log.last_mut() {
            Some((_, latest)) => *latest = specs,
            None => log.push((0, specs)),
        }
        self.track_epoch.fetch_add(1, Ordering::Release);
    }

    fn begin_track_specs(
        &self,
        first_segment: media_plane::ids::SegmentSeq,
        specs: Vec<TrackSpec>,
    ) {
        let first = first_segment.value();
        let mut log = self.track_specs.lock();
        match log.last_mut() {
            Some((from, latest)) if *from >= first => *latest = specs,
            _ => log.push((first, specs)),
        }
        self.track_epoch.fetch_add(1, Ordering::Release);
    }

    fn track_specs(&self) -> Vec<TrackSpec> {
        self.track_specs
            .lock()
            .last()
            .map(|(_, specs)| specs.clone())
            .unwrap_or_default()
    }

    fn track_specs_at(&self, segment: media_plane::ids::SegmentSeq) -> Vec<TrackSpec> {
        let log = self.track_specs.lock();
        log.iter()
            .rev()
            .find(|(from, _)| *from <= segment.value())
            .or_else(|| log.first())
            .map(|(_, specs)| specs.clone())
            .unwrap_or_default()
    }

    fn retire_track_specs_before(&self, oldest: media_plane::ids::SegmentSeq) {
        let mut log = self.track_specs.lock();
        while log.len() > 1 && log[1].0 <= oldest.value() {
            log.remove(0);
            self.track_epoch.fetch_add(1, Ordering::Release);
        }
    }

    /// Absorb every segment this route's `SegmentCursor` has produced since
    /// the last call — the same non-blocking, called-at-the-top-of-render
    /// shape as [`HlsOrigin`]'s own `drain` (see that type's module doc).
    ///
    /// `generation_of` names the init generation each segment was cut under; every segment
    /// passes through here once, in order, so this is where its DASH Period is fixed.
    fn drain(
        &self,
        generation_of: impl Fn(media_plane::ids::SegmentSeq) -> media_plane::ids::Generation,
    ) {
        let mut cursor = self.cursor.lock();
        let mut window = self.window.lock();
        let mut period = self.period.lock();
        while let Some(item) = cursor.poll() {
            if let SegmentCursorItem::Segment(entry) = item {
                if window.len() == self.capacity {
                    window.pop_front();
                }
                let public_seq = self
                    .epoch
                    .seq_offset
                    .saturating_add(u64::from(entry.sequence_number.value()));
                let gap = entry.bytes.is_empty();
                let init_generation = generation_of(entry.sequence_number);
                let timeline_position_ns = entry.timeline_position.as_nanos();
                window.push_back(DashWindowSegment {
                    segment_seq: entry.sequence_number,
                    duration_secs: entry.duration.as_secs_f64(),
                    timeline_position_ns,
                    public_seq,
                    gap,
                    init_generation,
                    period: period.advance(
                        gap,
                        init_generation,
                        (public_seq, timeline_position_ns),
                        self.program_start(),
                    ),
                    instance: self.instance,
                });
            }
        }
        if let Some(oldest) = window.iter().find(|s| s.instance == self.instance) {
            self.retire_track_specs_before(oldest.segment_seq);
        }
    }

    fn window_segments(
        &self,
        generation_of: impl Fn(media_plane::ids::SegmentSeq) -> media_plane::ids::Generation,
    ) -> Vec<DashWindowSegment> {
        self.drain(generation_of);
        self.window.lock().iter().copied().collect()
    }

    /// The Period cursor after every segment published so far.
    fn period_cursor(
        &self,
        generation_of: impl Fn(media_plane::ids::SegmentSeq) -> media_plane::ids::Generation,
    ) -> PeriodCursor {
        self.drain(generation_of);
        *self.period.lock()
    }

    /// The Period this program's first segment will open or join, for an MPD rendered before
    /// any segment closed: a continued program's next number and media start are its epoch's.
    fn first_period(&self, generation: media_plane::ids::Generation) -> PeriodAnchor {
        self.period
            .lock()
            .anchor_for(generation, self.program_start(), self.program_start())
    }

    /// `(public number, media start ns)` of this program's first segment.
    fn program_start(&self) -> (u64, u64) {
        (
            self.epoch.seq_offset.saturating_add(1),
            self.epoch.media_start_ns,
        )
    }
}

/// Why [`RouteHandle::add_segment`] could not publish a segment.
#[derive(Debug, thiserror::Error, Clone)]
#[non_exhaustive]
pub enum AddSegmentError {
    /// No program with this id has been published on the route yet.
    #[error("program not published yet")]
    ProgramNotPublished,
    /// The program's Trunk segment writer is held by a running ProgramSegmenter.
    #[error("segment writer unavailable (held by a running ProgramSegmenter)")]
    WriterUnavailable,
    /// The Trunk refused the segment (would stall a DVR pin, or non-monotonic number).
    #[error(transparent)]
    Publish(#[from] media_plane::trunk::TryPublishSegmentError),
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
    /// Segments with SCTE-35 `emsg` boxes spliced in, shared by every request.
    pub(crate) spliced: crate::origin::resource::SplicedSegments,
    /// The Smooth origin over this program, with the plan and fragments it has
    /// cached; built by the first Smooth request.
    pub(crate) smooth: std::sync::OnceLock<Arc<crate::output::smooth::SmoothServing>>,
    /// Rendered MPDs, valid for one [`MpdStamp`].
    mpd_cache: Mutex<MpdCache>,
    /// The route's bound on concurrent [`ServedEgress::prepare`](media_plane::egress::ServedEgress::prepare)
    /// runs on the blocking pool, shared by every egress of the route.
    warms: Arc<tokio::sync::Semaphore>,
    /// Test probe: MPD renders actually performed (cache misses).
    #[cfg(test)]
    pub(crate) mpd_renders: std::sync::atomic::AtomicUsize,
    /// Whether the program has muxable media now. Cleared when the source
    /// drops every muxable track; the specs and segments already published
    /// are unaffected.
    tracks_active: std::sync::atomic::AtomicBool,
    /// This program's route name — labels the DVR-failure metric recorded when
    /// a panic during persist stops recording for this program (I1).
    route_name: String,
    /// Test-only (I1): when set, the next `poll_dvr_blocking` persist panics,
    /// so the fail-closed path can be exercised without a real DVR bug.
    #[cfg(test)]
    panic_on_persist: std::sync::atomic::AtomicBool,
    /// DVR durable segment archive (issue #746) — `None` when this route
    /// does not have DVR enabled. Owns a pinning `SegmentCursor`, drained
    /// by [`Self::poll_dvr`].
    dvr: Mutex<Option<DvrRecorder>>,
    /// Whether `dvr` still holds a recorder; cleared when a persist panic
    /// drops it, so [`Self::set_init`] stops logging init changes nobody
    /// will archive.
    dvr_recording: std::sync::atomic::AtomicBool,
    /// Every init change since the recorder last archived under it, with the
    /// segment it starts at, so the recorder archives each pending segment
    /// under the init it was closed under.
    init_log: Mutex<Vec<crate::dvr::InitGeneration>>,
    /// The program this one replaced on a reconnect, until the first persist
    /// of this program's recorder has flushed the segments it had closed but
    /// not yet archived (see [`Self::flush_predecessor`]).
    predecessor: Mutex<Option<Arc<ProgramServing>>>,
    /// The replaced programs whose segments this program's DASH window still lists: they keep
    /// serving those segments and their init segments until the segments age out of the window
    /// (see [`Self::dash_window_segments`]).
    retained: Mutex<Vec<Arc<ProgramServing>>>,
    /// Lazily-acquired write handle for [`RouteHandle::add_segment`]/
    /// [`RouteHandle::add_part`] (this crate's own direct-`Trunk`-write test
    /// helpers, standing in for a real `crate::source::segment::ProgramSegmenter`).
    /// `None` until the first such call — **not** acquired eagerly at
    /// publish time, because a driver-backed program's segment writer is
    /// taken by its own real `ProgramSegmenter` immediately after
    /// `crate::source::report_driver_progress` publishes it
    /// (`crate::source::segment::drive_program_segmenters`, the very next
    /// call in the same iteration); grabbing it here first would starve that
    /// real segmenter of the one live segment/part writer a `Trunk` hands
    /// out (`media_plane::Trunk::segment_writer` returns `None` while a
    /// previously-issued writer is still alive). A program driven by a real segmenter never calls
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
    /// Sibling audio renditions (issue #1428 slice 2b).
    pub(crate) audio_rendition_servings: Vec<AudioServing>,
    /// Sibling WebVTT subtitle renditions, declared once the stream shows its
    /// teletext subtitle pages (issue #1428).
    subtitle_servings: Mutex<Vec<SubtitleServing>>,
    /// What a sibling origin needs to match this program's primary.
    sibling_params: SiblingParams,
}

/// The primary origin's settings every sibling origin repeats.
#[derive(Clone, Copy)]
struct SiblingParams {
    target_duration_secs: f64,
    part_target: Duration,
    window_segments: NonZeroUsize,
    media_sequence_offset: u64,
    discontinuity_sequence: u64,
    program_date_time: crate::config::ProgramDateTimeSource,
    hold_cold_playlist: bool,
}

/// Slots a program can serve subtitle renditions in. The media playlist
/// routes are fixed per slot (`subs-{slot}.m3u8`), so the set of subtitle
/// renditions can be learned from the stream after the router is built.
pub(crate) const MAX_SUBTITLE_RENDITIONS: usize = 8;

/// First `track_id` embedded in the segment filenames of a subtitle sibling
/// (slot `n` uses `SUBTITLE_TRACK_BASE + n`), clear of the primary's and the
/// audio siblings' ids.
const SUBTITLE_TRACK_BASE: u32 = 1000;

/// Relative URI of the media playlist of subtitle `slot`.
pub(crate) fn subtitle_playlist_uri(slot: usize) -> String {
    format!("subs-{slot}.m3u8")
}

/// One WebVTT subtitle rendition served from its own sibling `Trunk` and
/// `HlsOrigin` (issue #1428).
#[derive(Clone)]
pub struct SubtitleServing {
    pub decl: hls_runtime::server::SubtitleRenditionDecl,
    pub identity: String,
    pub track_id: transmux::TrackId,
    pub trunk: std::sync::Arc<Trunk>,
    pub origin: std::sync::Arc<hls_runtime::server::HlsOrigin>,
}

/// One separate audio rendition served from its own sibling `Trunk`/`HlsOrigin`
/// (issue #1428 slice 2b). `track_id` is the value embedded in the
/// rendition's init/segment/part filenames, so the shared resource route can
/// pick the right origin.
#[derive(Clone)]
pub struct AudioServing {
    pub decl: hls_runtime::server::AudioRenditionDecl,
    pub track_id: transmux::TrackId,
    pub trunk: std::sync::Arc<Trunk>,
    pub origin: std::sync::Arc<hls_runtime::server::HlsOrigin>,
}

/// Wire rendition-report siblings (RFC 8216bis §4.4.5.4): every origin
/// reports every OTHER rendition's live edge. `others` are the siblings of the
/// primary, each with its media playlist URI.
fn wire_rendition_siblings(primary: &Arc<HlsOrigin>, others: &[(Arc<HlsOrigin>, String)]) {
    use hls_runtime::server::RenditionSibling;
    let mut entries: Vec<(Arc<HlsOrigin>, String)> = Vec::with_capacity(others.len() + 1);
    entries.push((Arc::clone(primary), "media.m3u8".to_string()));
    entries.extend(others.iter().cloned());
    for (origin, self_uri) in &entries {
        let siblings: Vec<RenditionSibling> = entries
            .iter()
            .filter(|(_, uri)| uri != self_uri)
            .map(|(o, uri)| RenditionSibling {
                origin: Arc::downgrade(o),
                playlist_uri: uri.clone(),
            })
            .collect();
        origin.set_rendition_siblings(siblings);
    }
}

/// What a rendered MPD was produced from: the trunk's publish counter, the
/// origin's setter epoch and the track-spec log's epoch, read **before**
/// rendering so a change that lands mid-render only ever leaves the cached MPD
/// older than its stamp claims.
#[derive(Clone, PartialEq, Eq)]
struct MpdStamp {
    trunk: u64,
    origin: u64,
    tracks: u64,
}

#[derive(Default)]
struct MpdCache {
    stamp: Option<MpdStamp>,
    entries: HashMap<crate::output::dash::DashFlavour, Arc<std::sync::OnceLock<Option<String>>>>,
}

impl ProgramServing {
    /// `render`'s MPD for `flavour`, computed once per [`MpdStamp`]: a
    /// concurrent request waits for the one render rather than repeating it,
    /// and the cache lock is never held across a render.
    pub(crate) fn cached_mpd(
        &self,
        flavour: crate::output::dash::DashFlavour,
        render: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let stamp = MpdStamp {
            trunk: self.trunk.generation(),
            origin: self.ll_hls.render_epoch(),
            tracks: self.dash.track_epoch.load(Ordering::Acquire),
        };
        let cell = {
            let mut cache = self.mpd_cache.lock();
            if cache.stamp.as_ref() != Some(&stamp) {
                cache.entries.clear();
                cache.stamp = Some(stamp);
            }
            Arc::clone(cache.entries.entry(flavour).or_default())
        };
        cell.get_or_init(|| {
            #[cfg(test)]
            self.mpd_renders
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            render()
        })
        .clone()
    }

    /// Build a fresh bundle over `trunk`: the [`HlsOrigin`] and
    /// [`DashState`] every egress call site reads, both bound to `trunk` from
    /// construction, plus an optional DVR recorder if `dvr_config` is
    /// provided. The one and only place either is created — see
    /// [`RouteHandle::publish_program`].
    ///
    /// `previous` is the program this one continues (a reconnect, or the route a reload
    /// replaced). Its last init segment seeds the origin's first init generation before any
    /// segment is cut, and its DASH Period carries on: a byte-identical init stays in that
    /// generation and Period, a different one opens the next of each at this program's first
    /// segment (see [`PeriodCursor`]).
    fn new(
        trunk: Arc<Trunk>,
        route: &RouteHandle,
        epoch: ProgramEpoch,
        previous: Option<&ProgramContinuation>,
    ) -> Result<Arc<Self>, hls_runtime::server::HlsOriginBuildError> {
        let target_duration_secs = route.target_duration_secs;
        let part_target = route.part_target;
        let window_segments = route.window_segments_cap;
        let container = route.container;
        let dvr_config = route.dvr_config.clone();
        let route_name = route.name.as_str();
        let program_date_time = route.program_date_time;
        let declared_bandwidth_bps = route.declared_bandwidth_bps;
        let audio_renditions = &route.audio_renditions;
        let closed_caption_overrides = &route.closed_caption_overrides;
        let split_audio = !route.audio_rendition_sources.is_empty();
        let hold_cold_playlist = route.hold_cold_playlist;
        let warms = Arc::clone(&route.warms);
        let media_sequence_offset = epoch.seq_offset;
        let mut builder =
            HlsOriginConfig::new(Arc::clone(&trunk), target_duration_secs, window_segments)
                .with_container(container)
                .with_media_sequence_offset(media_sequence_offset)
                .with_discontinuity_sequence(epoch.discontinuity_sequence)
                .with_hold_cold_playlist(hold_cold_playlist);
        if program_date_time == crate::config::ProgramDateTimeSource::Arrival {
            builder = builder.with_program_date_time_arrival(Arc::new(|| {
                jiff::Timestamp::now().as_millisecond()
            }));
        }
        if container == Container::Fmp4 {
            builder = builder.with_low_latency(part_target);
        }
        if let Some(declared) = declared_bandwidth_bps {
            builder = builder.with_declared_bandwidth_bps(declared);
        }
        if !audio_renditions.is_empty() {
            builder = builder.with_audio_renditions(audio_renditions.clone());
        }
        if !closed_caption_overrides.is_empty() {
            builder = builder.with_closed_caption_overrides(closed_caption_overrides.clone());
        }
        builder = builder.with_playlist_uri("media.m3u8");
        // Sibling trunks + origins for the split audio renditions (issue #1428
        // slice 2b). Each carries one audio track's fMP4 and is served by its
        // own `HlsOrigin`; the primary trunk carries the remaining (video)
        // tracks.
        let sibling_decls: &[hls_runtime::server::AudioRenditionDecl] =
            if split_audio { audio_renditions } else { &[] };
        let mut audio_servings = Vec::with_capacity(sibling_decls.len());
        let mut sibling_origins: Vec<Arc<hls_runtime::server::HlsOrigin>> =
            Vec::with_capacity(sibling_decls.len());
        for (idx, decl) in sibling_decls.iter().enumerate() {
            let sibling_trunk = Arc::new(Trunk::new(route.trunk_config));
            let track_id = transmux::TrackId::new(u32::try_from(idx + 2).unwrap_or(u32::MAX));
            let mut ab = HlsOriginConfig::new(
                Arc::clone(&sibling_trunk),
                target_duration_secs,
                window_segments,
            )
            .with_container(Container::Fmp4)
            .with_media_sequence_offset(media_sequence_offset)
            .with_discontinuity_sequence(epoch.discontinuity_sequence)
            .with_hold_cold_playlist(hold_cold_playlist);
            if program_date_time == crate::config::ProgramDateTimeSource::Arrival {
                ab = ab.with_program_date_time_arrival(Arc::new(|| {
                    jiff::Timestamp::now().as_millisecond()
                }));
            }
            ab = ab.with_low_latency(part_target);
            ab = ab.with_playlist_uri(decl.uri.clone());
            let origin = Arc::new(HlsOrigin::new(ab)?);
            sibling_origins.push(Arc::clone(&origin));
            audio_servings.push(AudioServing {
                decl: decl.clone(),
                track_id,
                trunk: Arc::clone(&sibling_trunk),
                origin,
            });
        }

        if !sibling_origins.is_empty() {
            builder = builder.with_audio_sibling_origins(sibling_origins);
        }
        // `build` rejects a non-finite/non-positive target duration, a zero
        // part target and an offset with no room for u32 — all reachable
        // from caller-supplied numbers, so surfaced, never `expect`ed.
        let ll_hls = Arc::new(HlsOrigin::new(builder)?);
        if let Some(init) = previous.and_then(|p| p.init.clone()) {
            ll_hls.set_init(init);
        }

        let audio_entries: Vec<(Arc<HlsOrigin>, String)> = audio_servings
            .iter()
            .map(|a| (Arc::clone(&a.origin), a.decl.uri.clone()))
            .collect();
        wire_rendition_siblings(&ll_hls, &audio_entries);

        let period = previous.map_or_else(PeriodCursor::default, |p| {
            p.period.continued(epoch.seq_offset.saturating_add(1))
        });
        let dash = Arc::new(DashState::new(
            &trunk,
            window_segments,
            epoch,
            period,
            ll_hls.instance(),
            previous.map_or(&[][..], |p| p.window.as_slice()),
        ));

        let ext = match container {
            Container::Fmp4 => ".m4s",
            Container::MpegTs => ".ts",
            _ => ".m4s",
        };
        let dvr = dvr_config.filter(|c| c.enabled).and_then(|cfg| {
            match DvrRecorder::new(route_name.to_string(), cfg, ext, &trunk) {
                Ok(recorder) => {
                    // Index under the same numbers the origin shows.
                    let recorder = recorder.with_seq_offset(media_sequence_offset);
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

        Ok(Arc::new(ProgramServing {
            mpd_cache: Mutex::new(MpdCache::default()),
            warms,
            #[cfg(test)]
            mpd_renders: std::sync::atomic::AtomicUsize::new(0),
            trunk,
            ll_hls,
            dash,
            spliced: Default::default(),
            smooth: std::sync::OnceLock::new(),
            tracks_active: std::sync::atomic::AtomicBool::new(true),
            route_name: route_name.to_string(),
            #[cfg(test)]
            panic_on_persist: std::sync::atomic::AtomicBool::new(false),
            dvr_recording: std::sync::atomic::AtomicBool::new(dvr.is_some()),
            dvr: Mutex::new(dvr),
            init_log: Mutex::new(Vec::new()),
            predecessor: Mutex::new(None),
            retained: Mutex::new(previous.map(|p| p.servings.clone()).unwrap_or_default()),
            segment_writer: Mutex::new(None),
            next_timeline_ns: AtomicU64::new(epoch.media_start_ns),
            audio_rendition_servings: audio_servings,
            subtitle_servings: Mutex::new(Vec::new()),
            sibling_params: SiblingParams {
                target_duration_secs,
                part_target,
                window_segments,
                media_sequence_offset,
                discontinuity_sequence: epoch.discontinuity_sequence,
                program_date_time,
                hold_cold_playlist,
            },
        }))
    }

    pub(crate) fn warms(&self) -> &tokio::sync::Semaphore {
        &self.warms
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
        let first_seq = self
            .trunk
            .last_closed_segment()
            .unwrap_or(media_plane::ids::SegmentSeq::new(0))
            .saturating_next();
        self.set_init_from(bytes, first_seq);
    }

    fn set_init_from(&self, bytes: impl Into<Bytes>, first_seq: media_plane::ids::SegmentSeq) {
        let bytes = bytes.into();
        if self.dvr_recording.load(Ordering::SeqCst)
            && self.ll_hls.init_bytes().as_ref() != Some(&bytes)
        {
            self.init_log.lock().push(crate::dvr::InitGeneration {
                first_seq,
                init: bytes.clone(),
            });
        }
        self.ll_hls.set_init_from(bytes, first_seq);
    }

    /// The init changes the recorder has yet to archive under, oldest first;
    /// with none logged, the current init applies to every pending segment.
    fn init_generations(&self) -> Vec<crate::dvr::InitGeneration> {
        let logged = self.init_log.lock().clone();
        if !logged.is_empty() {
            return logged;
        }
        self.init_bytes()
            .map(|init| crate::dvr::InitGeneration {
                first_seq: media_plane::ids::SegmentSeq::new(0),
                init,
            })
            .into_iter()
            .collect()
    }

    fn flush_predecessor(&self) {
        let predecessor = self.predecessor.lock().take();
        if let Some(predecessor) = predecessor {
            predecessor.poll_dvr_blocking();
        }
    }

    pub(crate) fn set_audio_init(&self, index: usize, bytes: impl Into<Bytes>) {
        if let Some(serving) = self.audio_rendition_servings.get(index) {
            serving.origin.set_init(bytes);
        }
    }

    pub(crate) fn audio_servings(&self) -> &[AudioServing] {
        &self.audio_rendition_servings
    }

    pub(crate) fn subtitle_servings(&self) -> Vec<SubtitleServing> {
        self.subtitle_servings.lock().clone()
    }

    pub(crate) fn subtitle_by_track(&self, track_id: transmux::TrackId) -> Option<SubtitleServing> {
        self.subtitle_servings
            .lock()
            .iter()
            .find(|s| s.track_id == track_id)
            .cloned()
    }

    /// Builds the sibling `Trunk`/`HlsOrigin` of each declaration (once: a
    /// later call returns the servings already built), points the primary's
    /// master playlist at them and wires rendition reports between every
    /// origin. `None` when a sibling cannot be built or the declarations are
    /// invalid.
    pub(crate) fn declare_subtitles(
        &self,
        decls: &[hls_runtime::server::SubtitleRenditionDecl],
        identities: &[String],
    ) -> Option<Vec<SubtitleServing>> {
        let mut servings = self.subtitle_servings.lock();
        if !servings.is_empty() {
            let declared: Vec<_> = servings.iter().map(|s| &s.identity).collect();
            if declared.iter().copied().ne(identities.iter()) {
                tracing::warn!(
                    route = %self.route_name,
                    "subtitle renditions already declared for this program; keeping them"
                );
            }
            return Some(servings.clone());
        }
        if decls.is_empty()
            || decls.len() > MAX_SUBTITLE_RENDITIONS
            || decls.len() != identities.len()
        {
            return None;
        }
        let params = self.sibling_params;
        let mut built = Vec::with_capacity(decls.len());
        for (slot, (decl, identity)) in decls.iter().zip(identities).enumerate() {
            let trunk = Trunk::new(
                TrunkConfig::default()
                    .with_timed_capacity(nz(DEFAULT_TIMED_CAPACITY))
                    .with_sparse_capacity(nz(DEFAULT_SPARSE_CAPACITY))
                    .with_segment_capacity(params.window_segments)
                    .with_event_capacity(nz(DEFAULT_EVENT_CAPACITY))
                    .with_part_capacity(nz(DEFAULT_PART_CAPACITY)),
            );
            let mut builder = HlsOriginConfig::new(
                Arc::clone(&trunk),
                params.target_duration_secs,
                params.window_segments,
            )
            .with_container(Container::WebVtt)
            .with_media_sequence_offset(params.media_sequence_offset)
            .with_discontinuity_sequence(params.discontinuity_sequence)
            .with_hold_cold_playlist(params.hold_cold_playlist)
            .with_low_latency(params.part_target)
            .with_playlist_uri(decl.uri.clone());
            if params.program_date_time == crate::config::ProgramDateTimeSource::Arrival {
                builder = builder.with_program_date_time_arrival(Arc::new(|| {
                    jiff::Timestamp::now().as_millisecond()
                }));
            }
            let origin = match HlsOrigin::new(builder) {
                Ok(origin) => Arc::new(origin),
                Err(error) => {
                    tracing::warn!(%error, "subtitle rendition origin build failed");
                    return None;
                }
            };
            let slot_id = u32::try_from(slot).unwrap_or(u32::MAX);
            built.push(SubtitleServing {
                decl: decl.clone(),
                identity: identity.clone(),
                track_id: transmux::TrackId::new(SUBTITLE_TRACK_BASE.saturating_add(slot_id)),
                trunk,
                origin,
            });
        }
        if let Err(error) = self.ll_hls.set_subtitle_renditions(decls.to_vec()) {
            tracing::warn!(%error, "subtitle renditions rejected by the master playlist");
            return None;
        }
        let mut others: Vec<(Arc<HlsOrigin>, String)> = self
            .audio_rendition_servings
            .iter()
            .map(|a| (Arc::clone(&a.origin), a.decl.uri.clone()))
            .collect();
        others.extend(
            built
                .iter()
                .map(|s| (Arc::clone(&s.origin), s.decl.uri.clone())),
        );
        wire_rendition_siblings(&self.ll_hls, &others);
        *servings = built.clone();
        Some(built)
    }

    pub(crate) fn audio_by_track(&self, track_id: transmux::TrackId) -> Option<&AudioServing> {
        self.audio_rendition_servings
            .iter()
            .find(|s| s.track_id == track_id)
    }

    pub(crate) fn init_bytes(&self) -> Option<Bytes> {
        self.ll_hls.init_bytes()
    }

    fn set_track_specs(
        &self,
        specs: Vec<TrackSpec>,
        mapped: &[(usize, TrackSpec)],
        first_segment: Option<media_plane::ids::SegmentSeq>,
    ) {
        // The HLS master playlist's `CODECS` comes from the same specs (the
        // only source for a TS-HLS origin, which has no init segment to read).
        // With separate audio renditions the primary playlist carries only the
        // unmapped tracks, so its `CODECS` must not name the mapped audio.
        let primary = if mapped.is_empty() {
            specs.clone()
        } else {
            media_plane::segmenter::audio::leader_tracks(&specs, mapped)
        };
        if let Err(error) = self.ll_hls.set_track_specs(&primary) {
            tracing::warn!(%error, "HLS master CODECS unavailable for these track specs");
        }
        for (rendition, source) in mapped {
            let Some(serving) = self.audio_rendition_servings.get(*rendition) else {
                continue;
            };
            let mut spec = source.clone();
            spec.track_id = serving.track_id;
            if let Err(error) = serving.origin.set_track_specs(std::slice::from_ref(&spec)) {
                tracing::warn!(%error, "audio rendition CODECS unavailable for these track specs");
            }
        }
        match first_segment {
            Some(first) => self.dash.begin_track_specs(first, specs),
            None => self.dash.set_track_specs(specs),
        }
    }

    pub(crate) fn track_specs(&self) -> Vec<TrackSpec> {
        self.dash.track_specs()
    }

    pub(crate) fn track_specs_at(&self, segment: media_plane::ids::SegmentSeq) -> Vec<TrackSpec> {
        self.dash.track_specs_at(segment)
    }

    fn set_tracks_active(&self, active: bool) {
        self.tracks_active.store(active, Ordering::SeqCst);
    }

    fn tracks_active(&self) -> bool {
        self.tracks_active.load(Ordering::SeqCst)
    }

    /// This program's own segments in the DASH window.
    pub(crate) fn window_segments(&self) -> Vec<DashWindowSegment> {
        let instance = self.ll_hls.instance();
        self.dash_window_segments()
            .into_iter()
            .filter(|segment| segment.instance == instance)
            .collect()
    }

    /// The DASH window: this program's segments, after those of replaced programs it still
    /// lists. A replaced program stops being retained once none of its segments is listed.
    pub(crate) fn dash_window_segments(&self) -> Vec<DashWindowSegment> {
        let window = self
            .dash
            .window_segments(|seq| self.ll_hls.init_generation_for(seq));
        self.retained.lock().retain(|serving| {
            let instance = serving.ll_hls.instance();
            window.iter().any(|segment| segment.instance == instance)
        });
        window
    }

    /// The retained replaced program that cut segments under `instance`.
    pub(crate) fn retained_serving(&self, instance: u64) -> Option<Arc<ProgramServing>> {
        self.retained
            .lock()
            .iter()
            .find(|serving| serving.ll_hls.instance() == instance)
            .cloned()
    }

    /// Every retained replaced program, newest first.
    pub(crate) fn retained_servings(&self) -> Vec<Arc<ProgramServing>> {
        self.retained.lock().iter().rev().cloned().collect()
    }

    /// The DASH Period an MPD rendered before this program's first segment lists: the one the
    /// first segment will open or join under the current init.
    pub(crate) fn first_period(&self) -> PeriodAnchor {
        self.dash
            .first_period(self.ll_hls.current_init_generation())
    }

    fn presentation_origin(&self) -> SystemTime {
        self.dash.presentation_origin
    }

    fn epoch(&self) -> ProgramEpoch {
        self.dash.epoch
    }

    fn continuation(self: &Arc<Self>) -> ProgramContinuation {
        let window = self.dash_window_segments();
        let mut servings = self.retained.lock().clone();
        servings.push(Arc::clone(self));
        ProgramContinuation {
            window,
            servings,
            // `next_media_sequence()`, not `- 1`: the number the old origin's *open* segment was
            // shown with is skipped too — its parts were already served and must not be
            // reissued for different media.
            next_media_sequence: self
                .ll_hls
                .next_media_sequence()
                // Keep room for the `u32` sequence range `HlsOrigin` validates (unreachable in
                // practice: 2^64 segments).
                .min(u64::MAX - u64::from(u32::MAX)),
            discontinuity_sequence_after: self.ll_hls.discontinuity_sequence_after(),
            epoch: self.epoch(),
            init: self.init_bytes(),
            period: self
                .dash
                .period_cursor(|seq| self.ll_hls.init_generation_for(seq)),
        }
    }

    /// Runs `f` against this program's `Trunk` segment writer, acquiring it
    /// (from the `Trunk` itself) on first use. `None` if the writer is
    /// unavailable — already taken by a real `crate::source::segment::ProgramSegmenter`
    /// on this same `Trunk` (see this struct's own `segment_writer` field
    /// doc) — in which case the caller logs rather than panics: a test
    /// mis-mixing both write paths on one `Trunk` is a test-construction bug,
    /// not something that should take the whole process down.
    fn with_segment_writer<R>(&self, f: impl FnOnce(&SegmentWriter) -> R) -> Option<R> {
        let mut guard = self.segment_writer.lock();
        if guard.is_none() {
            *guard = self.trunk.segment_writer();
        }
        guard.as_ref().map(f)
    }

    fn add_part(&self, info: transmux::ll_hls::PartInfo) {
        let published = self.with_segment_writer(|writer| {
            writer.publish_part(PartEntry::new(
                info.bytes,
                media_plane::ids::SegmentSeq::new(info.segment_seq),
                media_plane::ids::PartIndex::new(info.part_index),
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

    /// Non-blocking: uses [`SegmentWriter::try_publish_segment`], never the
    /// blocking [`SegmentWriter::publish_segment`] (issue #1082's T7
    /// finding — this can run on a shared tokio worker thread same as
    /// `crate::source::segment::ProgramSegmenter`, which must never park
    /// one on a stalled `ArchiveOverrun::StallIngest` DVR pin). Unlike that
    /// type, this path has no retry queue of its own: a segment that
    /// cannot go through right now (or whose `sequence_number` turns out
    /// non-monotonic) is returned as an error, not queued — this is the
    /// test/fallback direct-write path for a `Trunk` with no real
    /// `ProgramSegmenter` running on it (see this struct's own doc), not
    /// the production ingest pipeline, so losing one segment here is far
    /// preferable to adding queueing machinery a real caller never
    /// exercises.
    fn add_segment(&self, info: transmux::ll_hls::SegmentInfo) -> Result<(), AddSegmentError> {
        let duration = Duration::from_secs_f64(info.duration);
        let start_ns = self.next_timeline_ns.fetch_add(
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        let published = self.with_segment_writer(|writer| {
            writer.try_publish_segment(SegmentEntry::new(
                info.bytes,
                media_plane::ids::SegmentSeq::new(info.segment_seq),
                duration,
                Timestamp::from_nanos(start_ns),
                transmux::SegmentMeta {
                    discontinuous: false,
                },
            ))
        });
        match published {
            None => Err(AddSegmentError::WriterUnavailable),
            Some(Err(e)) => Err(AddSegmentError::Publish(e)),
            Some(Ok(())) => Ok(()),
        }
    }

    /// `(in-progress-or-last-closed segment sequence number, its currently
    /// resident live-part count)` — mirrors
    /// `hls_runtime::server::HlsOrigin::live_edge`'s own derivation,
    /// directly against the `Trunk` (no cache): the abuse-bound check
    /// `crate::origin::resource`'s chunked-transfer whole-segment route
    /// (issue #721) needs.
    pub(crate) fn latest_progress(&self) -> (media_plane::ids::SegmentSeq, usize) {
        let last_closed = self
            .trunk
            .last_closed_segment()
            .unwrap_or(media_plane::ids::SegmentSeq::new(0));
        let candidate = last_closed.saturating_next();
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
    ///
    /// Async and `spawn_blocking`-dispatched (issue #1083, W3): persisting a
    /// segment is `File::write_all` + `fs::rename`, real synchronous disk
    /// I/O. Run on the caller (a shared tokio worker, in production) it could
    /// stall every other task on that worker for the duration of a slow
    /// write. Taking the DVR mutex inside the blocking closure also keeps
    /// the ingest task from holding it across the I/O.
    pub(crate) async fn poll_dvr(self: &Arc<Self>) {
        // Cheap pre-checks so a route with no DVR, or with nothing new to
        // persist, never pays for a `spawn_blocking` round-trip (issue #1083,
        // F: the UDP ingest path calls `advance_route` once per datagram).
        let latest = self.trunk.last_closed_segment();
        let latest_init = self.init_generations().pop().map(|g| g.init);
        {
            // A `try_lock` (not a blocking lock): this runs on the async
            // ingest task, and the lock may be held by `poll_dvr_blocking`
            // across a disk write. If it is, there is nothing useful to do
            // here anyway — the in-flight poll covers this segment.
            // `parking_lot` has no poisoning (SP6.6), so `try_lock` is a
            // plain `Option`: `None` means the lock is held (the in-flight
            // poll covers this segment), and there is no poison arm to fail
            // closed on any more.
            let Some(guard) = self.dvr.try_lock() else {
                return;
            };
            match guard.as_ref() {
                None => return,
                Some(recorder) if !recorder.needs_poll_for_init(latest, latest_init.as_deref()) => {
                    return;
                }
                Some(_) => {}
            }
        }
        let this = Arc::clone(self);
        if let Err(e) = tokio::task::spawn_blocking(move || this.poll_dvr_blocking()).await {
            tracing::error!("DVR persist task failed: {e}; recording may be incomplete");
        }
    }

    /// The synchronous body of [`Self::poll_dvr`], run on the blocking pool.
    ///
    /// **Fail closed on a panic (I1).** `DvrRecorder` holds multi-step state
    /// (`write_offset`/`index`/`periods`/`total_bytes`); a panic inside
    /// `poll_and_persist` (mid-`append_segment`, between the file write and the
    /// index push, say) can leave them disagreeing, so continuing to persist
    /// would write an archive whose index does not match its data (audit run 7,
    /// D3 / #1083). `parking_lot` guards release without poisoning, so without
    /// this wrap the next poll would keep writing to the half-updated recorder.
    /// The persist step is therefore caught with `catch_unwind`; on a panic the
    /// recorder is dropped (recording stops for this program for good), the
    /// `multimux_dvr_failed_total` counter is incremented and an error is
    /// logged — the same safety property the deleted poison-recovery wrapper
    /// provided, without relying on poisoning.
    fn poll_dvr_blocking(&self) {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        self.flush_predecessor();
        let mut dvr_guard = self.dvr.lock();
        let Some(recorder) = dvr_guard.as_mut() else {
            return;
        };
        let inits = self.init_generations();
        let logged = self.init_log.lock().len();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            if self
                .panic_on_persist
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                panic!("injected DVR persist panic (test)");
            }
            recorder.poll_and_persist_inits(&inits)
        }));
        match outcome {
            Ok(Ok(())) => {
                let mut log = self.init_log.lock();
                if logged > 1 {
                    log.drain(..logged - 1);
                }
            }
            Ok(Err(e)) => {
                tracing::error!(
                    "DVR poll_and_persist failed: {e}; \
                     recording may be incomplete"
                );
            }
            Err(payload) => {
                // A panic mid-persist: the recorder's multi-step state is
                // suspect, so stop recording rather than persist a corrupt
                // archive (fail closed).
                let _ = payload;
                *dvr_guard = None;
                self.dvr_recording.store(false, Ordering::SeqCst);
                self.init_log.lock().clear();
                tracing::error!(
                    route = %self.route_name,
                    "DVR persist panicked; recording stopped for this program \
                     rather than risk a corrupt archive"
                );
                metrics::counter!(
                    crate::prometheus::DVR_FAILED_TOTAL,
                    "route" => self.route_name.clone(),
                )
                .increment(1);
            }
        }
    }

    /// Feed raw TS bytes to this program's DVR recorder for EIT p/f
    /// tracking (issue #903) — a no-op when DVR is disabled, or when
    /// `DvrConfig::dvb_service_id` was never set. See
    /// [`crate::dvr::DvrRecorder::feed_si`].
    pub(crate) fn feed_si(&self, ts_bytes: &[u8]) {
        // Called from an async ingest loop (the TS-carrying sources), so this
        // must NOT block a runtime worker waiting for the DVR mutex while
        // `poll_dvr_blocking` holds it across a disk write (audit run 7, I).
        // EIT feeding is best-effort: if the lock is held, skip this chunk —
        // the next one (a few packets later) will feed it.
        // `parking_lot`: `try_lock` is a plain `Option` (`None` = contended,
        // skip this chunk — best-effort) and there is no poison arm.
        let Some(mut dvr_guard) = self.dvr.try_lock() else {
            return;
        };
        if let Some(ref mut recorder) = *dvr_guard
            && let Err(e) = recorder.feed_si(ts_bytes)
        {
            tracing::error!("DVR feed_si failed: {e}; EIT-aligned rolling may be incomplete");
        }
    }
}

/// One route's shared state — replaces the deleted `MediaStore`. Built once
/// per configured route ([`crate::Server`]), then shared
/// (via `Arc`) between whatever ingest publishes into it (every
/// driver-backed `crate::source::*::run_*`, via `crate::source::report_driver_progress`/
/// `crate::source::segment::drive_program_segmenters` — or the single
/// [`crate::source::advance_route`] facade over both) and every
/// [`crate::output::Output`]/the shared resource route that reads from it.
///
/// The route-wide settings [`RouteHandle::new`] records: the segmenter's
/// target duration, part target and advertised-window depth, plus every
/// serving/output option a configured route can set. A plain struct with a
/// [`Default`] and `with_*` builders, so the one constructor takes a value
/// rather than a dozen positional arguments, and a test builds only the
/// fields it cares about.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RouteConfig {
    pub target_duration_secs: f64,
    pub part_target: Duration,
    /// Advertised-window depth: the segment log's capacity, the playlist
    /// window's article, and the segment capacity of every program/trunk
    /// this route builds — one number, one source of truth. Clamped up to
    /// `1`.
    pub window_segments: usize,
    pub container: Container,
    pub ts_cut_policy: transmux::SegmentCutPolicy,
    pub ts_idr_fallback: crate::config::TsHlsIdrFallback,
    pub program_date_time: crate::config::ProgramDateTimeSource,
    pub hold_cold_playlist: bool,
    pub declared_bandwidth_bps: Option<u64>,
    pub audio_renditions: Vec<hls_runtime::server::AudioRenditionDecl>,
    pub audio_rendition_sources: Vec<transmux::audio_profile::AudioTrackSelector>,
    pub closed_caption_overrides: Vec<hls_runtime::server::ClosedCaptionDecl>,
    pub audio_profiles: Vec<transmux::audio_profile::AudioProfileOverride>,
    pub subtitles: bool,
    /// Concurrency bound for cold-egress warm-ups; see
    /// [`RouteHandle::with_max_concurrent_warms`].
    pub warms: Arc<tokio::sync::Semaphore>,
}

impl Default for RouteConfig {
    fn default() -> Self {
        RouteConfig {
            target_duration_secs: 1.0,
            part_target: Duration::from_millis(250),
            window_segments: DEFAULT_WINDOW_SEGMENTS,
            container: Container::default(),
            ts_cut_policy: transmux::SegmentCutPolicy::default(),
            ts_idr_fallback: crate::config::TsHlsIdrFallback::default(),
            program_date_time: crate::config::ProgramDateTimeSource::Off,
            hold_cold_playlist: hls_runtime::server::HOLD_COLD_PLAYLIST_DEFAULT,
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
            warms: Arc::new(tokio::sync::Semaphore::new(DEFAULT_MAX_CONCURRENT_WARMS)),
        }
    }
}

impl RouteConfig {
    #[must_use]
    pub fn with_target_duration_secs(mut self, v: f64) -> Self {
        self.target_duration_secs = v;
        self
    }

    #[must_use]
    pub fn with_part_target(mut self, v: Duration) -> Self {
        self.part_target = v;
        self
    }

    /// Sets the advertised-window depth (and so every trunk's segment
    /// capacity; a zero is clamped up to `1`).
    #[must_use]
    pub fn with_window_segments(mut self, segments: usize) -> Self {
        self.window_segments = segments;
        self
    }

    #[must_use]
    pub fn with_container(mut self, v: Container) -> Self {
        self.container = v;
        self
    }

    #[must_use]
    pub fn with_ts_cut_policy(mut self, v: transmux::SegmentCutPolicy) -> Self {
        self.ts_cut_policy = v;
        self
    }

    #[must_use]
    pub fn with_ts_idr_fallback(mut self, v: crate::config::TsHlsIdrFallback) -> Self {
        self.ts_idr_fallback = v;
        self
    }

    #[must_use]
    pub fn with_program_date_time(mut self, v: crate::config::ProgramDateTimeSource) -> Self {
        self.program_date_time = v;
        self
    }

    #[must_use]
    pub fn with_hold_cold_playlist(mut self, v: bool) -> Self {
        self.hold_cold_playlist = v;
        self
    }

    #[must_use]
    pub fn with_declared_bandwidth_bps(mut self, v: Option<u64>) -> Self {
        self.declared_bandwidth_bps = v;
        self
    }

    #[must_use]
    pub fn with_audio_renditions(
        mut self,
        v: Vec<hls_runtime::server::AudioRenditionDecl>,
    ) -> Self {
        self.audio_renditions = v;
        self
    }

    #[must_use]
    pub fn with_audio_rendition_sources(
        mut self,
        v: Vec<transmux::audio_profile::AudioTrackSelector>,
    ) -> Self {
        self.audio_rendition_sources = v;
        self
    }

    #[must_use]
    pub fn with_closed_caption_overrides(
        mut self,
        v: Vec<hls_runtime::server::ClosedCaptionDecl>,
    ) -> Self {
        self.closed_caption_overrides = v;
        self
    }

    #[must_use]
    pub fn with_audio_profiles(
        mut self,
        v: Vec<transmux::audio_profile::AudioProfileOverride>,
    ) -> Self {
        self.audio_profiles = v;
        self
    }

    #[must_use]
    pub fn with_subtitles(mut self, v: bool) -> Self {
        self.subtitles = v;
        self
    }

    /// Sets the cold-egress warm-up concurrency bound; a zero is clamped up
    /// to `1`.
    #[must_use]
    pub fn with_max_concurrent_warms(mut self, permits: usize) -> Self {
        self.warms = Arc::new(tokio::sync::Semaphore::new(permits.max(1)));
        self
    }
}

/// Owns **no** `Trunk`/`HlsOrigin`/`DashState` of its own — see this
/// module's own "No pre-first-program placeholder" doc. Every program's
/// serving state lives in `Self::programs` (the `programs` field, below),
/// created only by [`Self::publish_program`].
pub struct RouteHandle {
    /// This route's catch-up scan permits; built by the first catch-up request.
    catchup_scans: std::sync::OnceLock<Arc<tokio::sync::Semaphore>>,
    /// See [`Self::with_max_concurrent_warms`].
    warms: Arc<tokio::sync::Semaphore>,
    /// Fires when this route is replaced or removed; see [`Self::retire`].
    retired: tokio_util::sync::CancellationToken,
    health: Mutex<HealthState>,
    target_duration_secs: f64,
    part_target: Duration,
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
    /// Segment cut rule of this route's classic-TS segmenter, set via
    /// [`Self::with_ts_cut_policy`]; unused by the fMP4 container.
    ts_cut_policy: transmux::SegmentCutPolicy,
    /// What an `idr_only` classic-TS output does when no IDR arrives
    /// ([`crate::config::TsHlsIdrFallback`]).
    ts_idr_fallback: crate::config::TsHlsIdrFallback,
    /// Set once an `idr_only` classic-TS output has passed its IDR-wait bound
    /// without an IDR ([`TsHlsIdrStatus`]); `None` while IDR-aligned.
    ts_idr_status: Mutex<Option<TsHlsIdrStatus>>,
    /// Display text of the typed error that failed this route's output (see
    /// [`Self::fail_output`]); `None` while the output is healthy.
    output_error: Mutex<Option<String>>,
    created_at: SystemTime,
    /// Egress-resolvable `ProgramId -> Arc<ProgramServing>` registry:
    /// `media_plane::IngestDriver` mints one `Trunk` per program the instant
    /// it observes `media_plane::SessionEvent::NewProgram`, while this
    /// `RouteHandle` is built *before* any program is known
    /// (`crate::Server` constructs one per configured
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
    programs: RwLock<BTreeMap<ProgramId, Arc<ProgramServing>>>,
    /// This route's name, for labelling metrics that are per-route but
    /// recorded from code with no other way to know it (issue #809:
    /// `crate::source::segment::drive_program_segmenters`'s parts/segments
    /// counters). Defaults to [`DEFAULT_ROUTE_NAME`] until [`Self::with_name`]
    /// sets a real one; `crate::Server` always does, for
    /// every production route.
    name: String,
    /// Per-route DVR config — `None` when not configured, passed to every
    /// [`ProgramServing::new`] built by [`Self::publish_program`].
    dvr_config: Option<crate::dvr::DvrConfig>,
    /// The highest sequence number the DVR archive already holds (see
    /// [`Self::with_archive_floor`]); a new program's Media Sequence Numbers
    /// start above it, so a process restart does not reuse numbers the
    /// archive and any CDN already know for other media.
    archive_floor: u32,
    /// Where this route's live HLS playlists get their
    /// `#EXT-X-PROGRAM-DATE-TIME` values from (see
    /// [`Self::with_program_date_time`]). Defaults to
    /// [`crate::config::ProgramDateTimeSource::Off`] for a bare
    /// [`Self::new`] (tests), so existing test behaviour is unchanged;
    /// production routes set it from [`crate::config::Config::program_date_time`].
    program_date_time: crate::config::ProgramDateTimeSource,
    /// See [`Self::with_hold_cold_playlist`].
    hold_cold_playlist: bool,
    /// Operator-declared peak bitrate (bits/second) for this route's HLS
    /// master playlist (see [`HlsOriginConfig::with_declared_bandwidth_bps`]).
    /// `None` means none.
    declared_bandwidth_bps: Option<u64>,
    /// Audio renditions advertised in this route's HLS master playlist
    /// (issue #1428 slice 1).
    audio_renditions: Vec<hls_runtime::server::AudioRenditionDecl>,
    /// Source track per audio rendition for the opt-in split-mux (issue
    /// #1428 slice 2b); empty means no split.
    audio_rendition_sources: Vec<transmux::audio_profile::AudioTrackSelector>,
    /// Sibling (audio/subtitle rendition) trunk capacities (issue #2554): taken
    /// from this route's [`SegmenterConfig::trunk_config`] so a host can size
    /// every sibling trunk consistently. Defaults to the standard capacities.
    trunk_config: TrunkConfig,
    /// `NAME`/`LANGUAGE` overrides for detected closed captions (issue #1428).
    closed_caption_overrides: Vec<hls_runtime::server::ClosedCaptionDecl>,
    /// Operator-declared HE-AAC profile per audio track (issue #1428).
    audio_profiles: Vec<transmux::audio_profile::AudioProfileOverride>,
    /// Serve teletext subtitle pages as WebVTT renditions (issue #1428).
    subtitles: bool,
    /// Notifies waiters when a new program is published via
    /// [`Self::publish_program`] — used by push output tasks that need to
    /// discover a `Trunk` to subscribe to (issue #744).
    program_notify: tokio::sync::Notify,
    /// Which `Trunk` currently holds publish rights for each `ProgramId` —
    /// deliberately tracked separately from `programs` (issue W7: a
    /// route must reject a second concurrent publisher rather than let it
    /// take over or freeze the one it is already serving). Kept apart so a
    /// publisher's own disconnect ([`Self::release_program`]) never disturbs
    /// `programs`' already-served content — a route with no live
    /// publisher stays resolvable with whatever it last served, exactly as
    /// before this check existed; only a *new* [`Self::publish_program`]
    /// call for the same `ProgramId` is affected by this map at all.
    active_publisher: RwLock<HashMap<ProgramId, Arc<Trunk>>>,
    /// The programs of the route this one replaces ([`Self::continue_from`]): each program's
    /// first publication on this route continues their numbering and timeline.
    predecessor: Mutex<HashMap<ProgramId, ProgramContinuation>>,
}

impl RouteHandle {
    /// Build a fresh route: no program known yet, an empty registry, and this
    /// route's target duration/part target/window depth recorded so every
    /// later [`Self::publish_program`] builds each program's `Trunk`/
    /// `HlsOrigin`/`DashState` consistently.
    pub fn new(config: RouteConfig) -> Self {
        let window_segments =
            NonZeroUsize::new(config.window_segments).unwrap_or(NonZeroUsize::MIN);
        let RouteConfig {
            target_duration_secs,
            part_target,
            container,
            ts_cut_policy,
            ts_idr_fallback,
            program_date_time,
            hold_cold_playlist,
            declared_bandwidth_bps,
            audio_renditions,
            audio_rendition_sources,
            closed_caption_overrides,
            audio_profiles,
            subtitles,
            warms,
            ..
        } = config;
        RouteHandle {
            catchup_scans: std::sync::OnceLock::new(),
            warms,
            retired: tokio_util::sync::CancellationToken::new(),
            health: Mutex::new(HealthState::Connecting),
            target_duration_secs,
            part_target,
            window_segments_cap: window_segments,
            container,
            ts_cut_policy,
            ts_idr_fallback,
            ts_idr_status: Mutex::new(None),
            output_error: Mutex::new(None),
            created_at: SystemTime::now(),
            programs: RwLock::new(BTreeMap::new()),
            name: DEFAULT_ROUTE_NAME.to_string(),
            dvr_config: None,
            archive_floor: 0,
            program_date_time,
            hold_cold_playlist,
            declared_bandwidth_bps,
            audio_renditions,
            audio_rendition_sources,
            trunk_config: TrunkConfig::default()
                .with_timed_capacity(nz(DEFAULT_TIMED_CAPACITY))
                .with_sparse_capacity(nz(DEFAULT_SPARSE_CAPACITY))
                .with_segment_capacity(window_segments)
                .with_event_capacity(nz(DEFAULT_EVENT_CAPACITY))
                .with_part_capacity(nz(DEFAULT_PART_CAPACITY)),
            closed_caption_overrides,
            audio_profiles,
            subtitles,
            program_notify: tokio::sync::Notify::new(),
            active_publisher: RwLock::new(HashMap::new()),
            predecessor: Mutex::new(HashMap::new()),
        }
    }

    /// Names this route (see [`Self::name`]'s own field doc) — a consuming
    /// builder so `crate::Server`'s one production call
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

    /// Bounds how many cold-egress warm-ups (`ServedEgress::prepare`: the
    /// I-frame and bitrate scans of a new HLS segment, the first Smooth plan)
    /// this route runs on the blocking pool at once; another route's warm-ups
    /// are unaffected. Clamped up to `1`. Defaults to
    /// [`DEFAULT_MAX_CONCURRENT_WARMS`]; routes set it from
    /// [`crate::config::Config::max_concurrent_warms_per_route`]. Applies to
    /// programs published after the call.
    #[must_use]
    pub fn with_max_concurrent_warms(mut self, permits: usize) -> Self {
        self.warms = Arc::new(tokio::sync::Semaphore::new(permits.max(1)));
        self
    }

    /// Makes this (new) route continue `old`: the first publication of each program `old` served
    /// carries on from it with the next media sequence number, the discontinuity sequence of
    /// `old`'s newest segment, its DASH start number and presentation origin, and the segmenter
    /// marks the first segment discontinuous (`EXT-X-DISCONTINUITY`), exactly as a reconnect on
    /// the same route does. DASH stays in `old`'s Period when the first init is byte-identical to
    /// `old`'s, and opens a new Period at the first segment otherwise (issue #2611). Call before
    /// the first program is published.
    pub(crate) fn continue_from(&self, old: &RouteHandle) {
        let continuations: HashMap<ProgramId, ProgramContinuation> = old
            .programs
            .read()
            .iter()
            .map(|(program, serving)| (*program, serving.continuation()))
            .collect();
        *self.predecessor.lock() = continuations;
    }

    /// The sequence number a replacement of this route must number above: the highest media
    /// sequence number any of its programs has shown (the open segment's included), and what the
    /// DVR archive already held when it started. A replacement built with
    /// [`Self::with_archive_floor`] of this value continues the numbering, so a client's
    /// `_HLS_msn` for this route means the same segment on the replacement.
    pub(crate) fn media_sequence_floor(&self) -> u32 {
        let next = self
            .programs
            .read()
            .values()
            .map(|serving| serving.ll_hls.next_media_sequence())
            .max()
            .unwrap_or(0);
        u32::try_from(next)
            .unwrap_or(u32::MAX)
            .max(self.archive_floor)
    }

    /// Marks this route as replaced or removed: the admin media service drops every request it
    /// is still serving from this route (a parked blocking reload included) and serves it again
    /// against the route registered under the name now.
    pub(crate) fn retire(&self) {
        self.retired.cancel();
    }

    /// The token [`Self::retire`] fires.
    pub(crate) fn retired_token(&self) -> tokio_util::sync::CancellationToken {
        self.retired.clone()
    }

    /// Sets the segment cut rule of this route's classic-TS segmenter
    /// ([`transmux::SegmentCutPolicy`]); `multimux` passes
    /// [`crate::config::Route::ts_hls_cut`] here, whose default is
    /// `IdrOnly`. A bare [`Self::new`] keeps `AnyRap`.
    #[must_use]
    pub fn with_ts_cut_policy(mut self, policy: transmux::SegmentCutPolicy) -> Self {
        self.ts_cut_policy = policy;
        self
    }

    /// The configured classic-TS cut rule (see [`Self::with_ts_cut_policy`]).
    pub fn ts_cut_policy(&self) -> transmux::SegmentCutPolicy {
        self.ts_cut_policy
    }

    /// Select the `idr_only` no-IDR behaviour (see
    /// [`crate::config::Route::ts_hls_idr_fallback`]).
    #[must_use]
    pub fn with_ts_idr_fallback(mut self, fallback: crate::config::TsHlsIdrFallback) -> Self {
        self.ts_idr_fallback = fallback;
        self
    }

    /// The configured no-IDR behaviour.
    pub fn ts_idr_fallback(&self) -> crate::config::TsHlsIdrFallback {
        self.ts_idr_fallback
    }

    /// Record that the classic-TS output passed its IDR-wait bound without an
    /// IDR. Returns `true` only the first time, so the caller logs once.
    pub(crate) fn note_ts_idr_missing(&self, status: TsHlsIdrStatus) -> bool {
        let mut slot = self.ts_idr_status.lock();
        if slot.is_some() {
            return false;
        }
        *slot = Some(status);
        true
    }

    /// Whether the classic-TS output is not IDR-aligned, and how it copes
    /// (`None` while it is, or when there is nothing to report).
    pub fn ts_idr_status(&self) -> Option<TsHlsIdrStatus> {
        *self.ts_idr_status.lock()
    }

    pub(crate) fn fail_output_message(&self, message: String) -> bool {
        let mut slot = self.output_error.lock();
        if slot.is_some() {
            return false;
        }
        *slot = Some(message);
        true
    }

    /// The text of the typed error that failed this route's output, if any.
    pub fn output_error(&self) -> Option<String> {
        self.output_error.lock().clone()
    }

    /// Attach DVR config to this route (issue #746) — a consuming builder,
    /// chained onto [`Self::new`] the same way as [`Self::with_name`].
    /// Every program [`Self::publish_program`] builds from this point on
    /// gets a DVR recorder created alongside its `ProgramServing`.
    pub fn with_dvr(mut self, dvr_config: crate::dvr::DvrConfig) -> Self {
        self.dvr_config = Some(dvr_config);
        self
    }

    /// Start every program's Media Sequence Numbers above `floor`, the highest
    /// sequence number the route's DVR archive already holds (see
    /// `crate::dvr::archive_seq_floor`). Without it a process restart
    /// renumbers from `1` while the archive and any HTTP cache still hold the
    /// previous process's `1..=N` for different media (audit r07-C5/C2, issue
    /// #1083). A consuming builder like [`Self::with_dvr`].
    #[must_use]
    pub fn with_archive_floor(mut self, floor: u32) -> Self {
        self.archive_floor = floor;
        self
    }

    /// Sets whether a media-playlist request without `_HLS_msn`/`_HLS_part`
    /// is held until the first segment closes, and a playlist request made before the route
    /// announces its program until it does (bounded by the blocking-reload timeout, then `503`);
    /// defaults to
    /// [`hls_runtime::server::HOLD_COLD_PLAYLIST_DEFAULT`] and production
    /// routes set it from [`crate::config::Config::hold_cold_playlist`].
    #[must_use]
    pub fn with_hold_cold_playlist(mut self, hold: bool) -> Self {
        self.hold_cold_playlist = hold;
        self
    }

    pub(crate) fn holds_cold_playlist(&self) -> bool {
        self.hold_cold_playlist
    }

    /// Where this route's live HLS playlists get their
    /// `#EXT-X-PROGRAM-DATE-TIME` values from (issue #1423 part B2b) — a
    /// consuming builder, chained onto [`Self::new`] the same way as
    /// [`Self::with_archive_floor`]. Defaults to
    /// [`crate::config::ProgramDateTimeSource::Off`] (no arrival source)
    /// when never called, so a bare [`Self::new`] (tests) keeps the
    /// previous output. Production routes set it from
    /// [`crate::config::Config::program_date_time`].
    #[must_use]
    pub fn with_program_date_time(mut self, source: crate::config::ProgramDateTimeSource) -> Self {
        self.program_date_time = source;
        self
    }

    /// Sets this route's operator-declared peak bitrate (bits/second) for its
    /// HLS master playlist (see [`crate::config::Route::declared_bandwidth_bps`]).
    /// Accepts either a `u64` or an `Option<u64>`.
    pub fn with_declared_bandwidth_bps<V>(mut self, value: V) -> Self
    where
        V: IntoDeclaredBandwidth,
    {
        self.declared_bandwidth_bps = value.into_declared_bandwidth();
        self
    }

    /// Sets this route's audio renditions (issue #1428 slice 1) — a consuming
    /// builder, chained onto [`Self::new`] like [`Self::with_declared_bandwidth_bps`].
    #[must_use]
    pub fn with_audio_renditions(
        mut self,
        value: Vec<hls_runtime::server::AudioRenditionDecl>,
    ) -> Self {
        self.audio_renditions = value;
        self
    }

    /// Sets the source audio track of each audio rendition (issue #1428 slice
    /// 2b), enabling the split-mux when non-empty — a consuming builder.
    #[must_use]
    pub fn with_audio_rendition_sources(
        mut self,
        value: Vec<transmux::audio_profile::AudioTrackSelector>,
    ) -> Self {
        self.audio_rendition_sources = value;
        self
    }

    /// Sets the `NAME`/`LANGUAGE` overrides applied to detected closed
    /// captions (issue #1428) — a consuming builder.
    #[must_use]
    pub fn with_closed_caption_overrides(
        mut self,
        value: Vec<hls_runtime::server::ClosedCaptionDecl>,
    ) -> Self {
        self.closed_caption_overrides = value;
        self
    }

    /// Turns the WebVTT subtitle renditions built from a stream's teletext
    /// subtitle pages on or off (issue #1428; on by default) — a consuming
    /// builder.
    #[must_use]
    pub fn with_subtitles(mut self, value: bool) -> Self {
        self.subtitles = value;
        self
    }

    pub fn subtitles_enabled(&self) -> bool {
        self.subtitles
    }

    /// Sets the [`TrunkConfig`] used for sibling (audio/subtitle rendition)
    /// trunks built by [`Self::publish_program`] (issue #2554): capacities
    /// propagate to every sibling trunk.
    #[must_use]
    pub fn with_trunk_config(mut self, value: TrunkConfig) -> Self {
        self.trunk_config = value;
        self
    }

    pub fn trunk_config(&self) -> &TrunkConfig {
        &self.trunk_config
    }

    /// Declares the HE-AAC profile of audio tracks (issue #1428) — a consuming
    /// builder; an empty list leaves every track as the source states it.
    #[must_use]
    pub fn with_audio_profiles(
        mut self,
        value: Vec<transmux::audio_profile::AudioProfileOverride>,
    ) -> Self {
        self.audio_profiles = value;
        self
    }

    /// `tracks` with this route's declared HE-AAC profiles applied
    /// ([`transmux::audio_profile::apply_overrides`]), logging what did not apply.
    pub(crate) fn with_declared_audio_profiles(&self, tracks: Vec<TrackSpec>) -> Vec<TrackSpec> {
        let applied = transmux::audio_profile::apply_overrides(&self.audio_profiles, tracks);
        for declared in &applied.unmatched {
            tracing::warn!(
                route = self.name.as_str(),
                selector = %declared.track,
                "audio_profiles entry matches no audio track; ignoring until one appears"
            );
        }
        for (declared, error) in &applied.refused {
            tracing::warn!(
                route = self.name.as_str(),
                selector = %declared.track,
                profile = %declared.audio_profile,
                %error,
                "audio_profiles entry cannot be applied to this track; leaving it as the source states it"
            );
        }
        applied.tracks
    }

    pub(crate) fn audio_rendition_sources(&self) -> &[transmux::audio_profile::AudioTrackSelector] {
        &self.audio_rendition_sources
    }

    pub(crate) fn audio_profiles_for_segmenter(
        &self,
    ) -> Vec<transmux::audio_profile::AudioProfileOverride> {
        self.audio_profiles.clone()
    }

    /// This route's configured container (see [`Self::with_container`]) —
    /// needed by [`crate::source::segment::drive_program_segmenters`] to pick
    /// the fMP4 (`LlHlsSegmenter`) or classic-TS
    /// (`transmux::ts_hls::StreamingTsHlsSegmenter`) segmenter path for every
    /// program on this route.
    pub(crate) fn container(&self) -> Container {
        self.container
    }

    /// This route's DVR archive config (see [`Self::with_dvr`]) — `Some`
    /// only when DVR recording is actually enabled
    /// (`crate::dvr::DvrConfig::enabled`), never `Some` with `enabled ==
    /// false`, so a caller never needs to check that field itself. Needed
    /// by [`crate::output::catchup::CatchupOutput`] (issue #900) to find
    /// the on-disk archive root — `crate::dvr::DvrRecorder` itself reads
    /// `self.dvr_config` directly, not through this accessor, since it is
    /// built once at [`Self::publish_program`] time.
    pub(crate) fn catchup_scans(&self) -> &std::sync::OnceLock<Arc<tokio::sync::Semaphore>> {
        &self.catchup_scans
    }

    pub(crate) fn dvr_config(&self) -> Option<&crate::dvr::DvrConfig> {
        self.dvr_config.as_ref().filter(|c| c.enabled)
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
    /// with a stable `Arc`, then repeatedly as a harmless idempotent check.
    ///
    /// A call with a **different** `Arc` while `active_publisher` still holds
    /// a *different* `Trunk` for this `program` — not yet freed by
    /// [`Self::release_program`] — is a second, concurrent publisher trying
    /// to take over a program this route is already actively receiving from —
    /// rejected, not bound: `programs`' existing entry (and the
    /// publisher behind it) is left exactly as it was, and the rejected
    /// caller's own session is expected to be reaped once its driver-level
    /// admission loop notices it never got published (see
    /// `crate::source::report_driver_progress`, whose `Listener`-backed
    /// callers cap concurrent admission at `Listener::max_sessions` but do not
    /// themselves limit *this* route to one active publisher — this check is
    /// what does). Once the active publisher's session ends,
    /// [`Self::release_program`] frees `active_publisher`'s slot (**not**
    /// `programs`' — see that field's own doc), so a *subsequent*
    /// `publish_program` call — a legitimate reconnect, or a backup encoder
    /// taking over from a primary that just died — rebuilds `programs`' entry
    /// over the new `Trunk`, exactly like the plain, no-rival case.
    pub fn publish_program(&self, program: ProgramId, trunk: Arc<Trunk>) {
        {
            let mut active = self.active_publisher.write();
            match active.get(&program) {
                Some(owner) if Arc::ptr_eq(owner, &trunk) => {}
                Some(_) => {
                    tracing::warn!(
                        route = %self.name,
                        ?program,
                        "rejecting a second concurrent publisher for this program — \
                         this route is already receiving from another connection"
                    );
                    return;
                }
                None => {
                    active.insert(program, Arc::clone(&trunk));
                }
            }
        }
        let mut programs = self.programs.write();
        let already_bound = programs
            .get(&program)
            .is_some_and(|existing| Arc::ptr_eq(&existing.trunk, &trunk));
        if already_bound {
            return;
        }
        // A replacement for an already-served program (a reconnect over a
        // fresh `Trunk`, which numbers its segments from 1 again) continues
        // the previous origin's Media Sequence Number, so clients never see
        // it decrease (RFC 8216bis §6.2.2).
        // The same holds for the first program of a route that replaces another one.
        let previous = programs
            .get(&program)
            .map(|previous| previous.continuation())
            .or_else(|| self.predecessor.lock().get(&program).cloned());
        let seq_offset = previous
            .as_ref()
            .map_or(0, |previous| previous.next_media_sequence)
            // ...and never at or below what the DVR archive already holds
            // (a previous process's numbers).
            .max(u64::from(self.archive_floor));
        let previous_epoch = previous.as_ref().map(|previous| previous.epoch);
        let epoch = ProgramEpoch {
            seq_offset,
            media_start_ns: previous_epoch.map_or(0, |e| {
                let slots = seq_offset.saturating_add(1).saturating_sub(e.first_number);
                (slots as f64 * self.target_duration_secs * 1e9) as u64
            }),
            discontinuity_sequence: previous
                .as_ref()
                .map_or(0, |p| p.discontinuity_sequence_after),
            first_number: previous_epoch.map_or(seq_offset.saturating_add(1), |e| e.first_number),
            presentation_origin: previous_epoch
                .map_or_else(SystemTime::now, |e| e.presentation_origin),
        };
        let serving = match ProgramServing::new(Arc::clone(&trunk), self, epoch, previous.as_ref())
        {
            Ok(serving) => serving,
            Err(e) => {
                // An unservable configuration (e.g. a NaN target duration
                // that bypassed `Config::validate`): refuse to publish, and
                // give the publish slot back so a corrected retry can bind.
                tracing::error!(
                    route = %self.name,
                    ?program,
                    error = %e,
                    "cannot build this program's serving state; program not published"
                );
                let mut active = self.active_publisher.write();
                if active.get(&program).is_some_and(|o| Arc::ptr_eq(o, &trunk)) {
                    active.remove(&program);
                }
                return;
            }
        };
        if let Some(previous) = programs.get(&program)
            && previous.dvr_recording.load(Ordering::SeqCst)
            && serving.dvr_recording.load(Ordering::SeqCst)
        {
            *serving.predecessor.lock() = Some(Arc::clone(previous));
        }
        programs.insert(program, serving);
        self.program_notify.notify_waiters();
    }

    /// Test-only: how many programs currently hold an `active_publisher`
    /// slot (M1's assertion that every scaffold exit path releases it).
    #[cfg(test)]
    pub(crate) fn active_publisher_len(&self) -> usize {
        self.active_publisher.read().len()
    }

    /// Test-only: whether a fresh publisher for `program` would be accepted
    /// (i.e. the `active_publisher` slot is free) — the reconnect check I-E
    /// asserts.
    #[cfg(test)]
    pub(crate) fn publish_program_is_free_for_test(&self, program: ProgramId) -> bool {
        !self.active_publisher.read().contains_key(&program)
    }

    /// Release `program`'s `active_publisher` slot, if (and only if)
    /// it still holds exactly `trunk` — the counterpart to
    /// [`Self::publish_program`]'s rejection of a second concurrent
    /// publisher, called once a driver-backed session that owned `trunk` has
    /// been reaped (see `crate::source::release_route`). Deliberately does
    /// **not** touch `programs`: the route keeps serving whatever
    /// that publisher last produced (exactly as it would have before this
    /// active-publisher check existed — a disconnected publisher's content
    /// was never un-served just because the connection ended), only a
    /// *future* [`Self::publish_program`] call is affected, letting a new
    /// publisher for the same `program` bind instead of being rejected. A
    /// no-op when `program`'s slot is already free, or held by a different
    /// `Trunk` (this session lost the take-over race in `publish_program`
    /// and so never held it) — so a caller may call this speculatively,
    /// exactly like [`Self::publish_program`].
    pub fn release_program(&self, program: ProgramId, trunk: &Arc<Trunk>) {
        let mut active = self.active_publisher.write();
        let still_owned = active
            .get(&program)
            .is_some_and(|owner| Arc::ptr_eq(owner, trunk));
        if still_owned {
            active.remove(&program);
        }
    }

    /// Free every program's `active_publisher` slot, leaving `programs`
    /// serving what was last published. For a supervisor whose attempt has
    /// ended: no publisher of this route is alive any more, so the next
    /// attempt's `publish_program` is a legitimate reconnect, not a rival.
    pub(crate) fn release_all_publishers(&self) {
        self.active_publisher.write().clear();
    }

    /// The `Trunk` of this route's lowest-numbered published program — the
    /// one push and WHEP egress follow — or `None` while nothing is published.
    fn first_trunk(&self) -> Option<Arc<Trunk>> {
        self.programs
            .read()
            .iter()
            .min_by_key(|(id, _)| **id)
            .map(|(_, serving)| serving.trunk())
    }

    /// `test-seams` hook over [`Self::first_trunk`]: an integration test that
    /// drives [`crate::source::hls_pull::run_hls_pull`] itself reads back what
    /// the route published, exactly as egress would.
    #[doc(hidden)]
    #[cfg(feature = "test-seams")]
    #[must_use]
    pub fn first_trunk_for_test(&self) -> Option<Arc<Trunk>> {
        self.first_trunk()
    }

    /// Wait until at least one program is published on this route, then return
    /// the first program's `Trunk`. Used by push output tasks (issue #744).
    pub async fn await_first_trunk(&self) -> Arc<Trunk> {
        self.await_trunk_other_than(None).await
    }

    /// Wait until the route's first program is bound to a `Trunk` other than
    /// `current` (a source reconnect publishes a fresh `Trunk`, audit r07-C4,
    /// issue #1083), and return it. Push and WHEP egress use this to notice
    /// that the `Trunk` they were draining is dead and re-subscribe to the new
    /// one instead of going silent.
    pub async fn await_trunk_change(&self, current: &Arc<Trunk>) -> Arc<Trunk> {
        self.await_trunk_other_than(Some(current)).await
    }

    /// [`Self::resolve_program`], parking a `NotYetAnnounced` answer until a program is published
    /// or `timeout` passes when the route holds cold playlist requests (see
    /// [`Self::with_hold_cold_playlist`]); a timeout answers `NotYetAnnounced`.
    pub(crate) async fn resolve_program_held(
        &self,
        program: ProgramId,
        timeout: Duration,
    ) -> ProgramResolution {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.program_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.resolve_program(program) {
                ProgramResolution::NotYetAnnounced if self.hold_cold_playlist => {}
                resolved => return resolved,
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.resolve_program(program);
            }
        }
    }

    async fn await_trunk_other_than(&self, current: Option<&Arc<Trunk>>) -> Arc<Trunk> {
        loop {
            // Register interest *before* looking, so a publish landing
            // between the check and the wait is not lost
            // (`Notify::notify_waiters` stores no permit).
            let notified = self.program_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(trunk) = self.first_trunk()
                && current.is_none_or(|c| !Arc::ptr_eq(c, &trunk))
            {
                return trunk;
            }
            notified.await;
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
        let trunk_config = TrunkConfig::default()
            .with_timed_capacity(nz(DEFAULT_TIMED_CAPACITY))
            .with_sparse_capacity(nz(DEFAULT_SPARSE_CAPACITY))
            .with_segment_capacity(self.window_segments_cap)
            .with_event_capacity(nz(DEFAULT_EVENT_CAPACITY))
            .with_part_capacity(nz(DEFAULT_PART_CAPACITY));
        let trunk = Trunk::new(trunk_config);
        self.publish_program(program, Arc::clone(&trunk));
        trunk
    }

    /// Read-only lookup of `program`'s [`ProgramServing`] bundle, if
    /// registered. The shared implementation behind [`Self::resolve_program`]
    /// and every per-program convenience accessor below.
    pub(crate) fn serving(&self, program: ProgramId) -> Option<Arc<ProgramServing>> {
        self.programs.read().get(&program).cloned()
    }

    /// Resolve `program` against this route's registry — the egress side's
    /// read (see [`ProgramResolution`] for why this returns a three-way enum
    /// rather than `Option<Arc<ProgramServing>>`, and `programs`'s
    /// own doc for why the lock is a `RwLock`). Every migrated egress call
    /// site resolves through `crate::http::resolve_route_program`, which
    /// wraps this.
    pub(crate) fn resolve_program(&self, program: ProgramId) -> ProgramResolution {
        let programs = self.programs.read();
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

    pub(crate) fn set_init_from(
        &self,
        program: ProgramId,
        bytes: impl Into<Bytes>,
        first_segment: media_plane::ids::SegmentSeq,
    ) {
        match self.serving(program) {
            Some(serving) => serving.set_init_from(bytes, first_segment),
            None => tracing::warn!(
                ?program,
                "RouteHandle::set_init_from: program not published yet"
            ),
        }
    }

    /// Hand the caption streams detected in `program`'s video to its master
    /// playlist. `false` when the program is not published yet (the caller
    /// retries).
    pub fn note_closed_captions(
        &self,
        program: ProgramId,
        detected: Vec<broadcast_hls::InstreamId>,
    ) -> bool {
        match self.serving(program) {
            Some(serving) => {
                serving.ll_hls.set_detected_closed_captions(detected);
                true
            }
            None => false,
        }
    }

    /// Hand the `LANGUAGE` (BCP 47) the stream declares for each caption
    /// stream of `program` to its master playlist. `false` when the program
    /// is not published yet.
    pub fn note_closed_caption_languages(
        &self,
        program: ProgramId,
        languages: Vec<(broadcast_hls::InstreamId, String)>,
    ) -> bool {
        match self.serving(program) {
            Some(serving) => {
                serving
                    .ll_hls
                    .set_declared_closed_caption_languages(languages);
                true
            }
            None => false,
        }
    }

    pub(crate) fn set_part_target_ms(&self, program: ProgramId, part_target: Duration) -> bool {
        match self.serving(program) {
            Some(serving) => {
                serving.ll_hls.set_part_target(part_target);
                true
            }
            None => false,
        }
    }

    pub fn set_audio_init(&self, program: ProgramId, index: usize, bytes: impl Into<Bytes>) {
        match self.serving(program) {
            Some(serving) => serving.set_audio_init(index, bytes),
            None => tracing::warn!(
                ?program,
                "RouteHandle::set_audio_init: program not published yet"
            ),
        }
    }

    /// Sibling audio rendition servings for `program` (issue #1428 slice 2b).
    pub fn audio_servings(&self, program: ProgramId) -> Vec<AudioServing> {
        self.serving(program)
            .map(|s| s.audio_servings().to_vec())
            .unwrap_or_default()
    }

    /// Sibling subtitle rendition servings for `program` (issue #1428).
    pub fn subtitle_servings(&self, program: ProgramId) -> Vec<SubtitleServing> {
        self.serving(program)
            .map(|s| s.subtitle_servings())
            .unwrap_or_default()
    }

    /// Declares the subtitle renditions of the program served from `trunk`
    /// and returns their servings, in the order first declared (see
    /// `ProgramServing::declare_subtitles`).
    pub(crate) fn declare_subtitles_for_trunk(
        &self,
        trunk: &Trunk,
        decls: &[hls_runtime::server::SubtitleRenditionDecl],
        identities: &[String],
    ) -> Option<Vec<SubtitleServing>> {
        let serving = self
            .programs
            .read()
            .values()
            .find(|s| std::ptr::eq(std::sync::Arc::as_ptr(&s.trunk), trunk))
            .cloned()?;
        serving.declare_subtitles(decls, identities)
    }

    pub(crate) fn audio_renditions(&self) -> &[hls_runtime::server::AudioRenditionDecl] {
        &self.audio_renditions
    }

    pub(crate) fn audio_servings_for_trunk(&self, trunk: &Trunk) -> Vec<AudioServing> {
        self.programs
            .read()
            .values()
            .find(|s| std::ptr::eq(std::sync::Arc::as_ptr(&s.trunk), trunk))
            .map(|s| s.audio_servings().to_vec())
            .unwrap_or_default()
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
        self.record_track_specs(program, specs, None);
    }

    /// [`Self::set_track_specs`] for a track set that starts a new init
    /// generation at `first_segment`: the specs already recorded keep
    /// describing the segments before it.
    pub(crate) fn begin_track_specs(
        &self,
        program: ProgramId,
        specs: Vec<TrackSpec>,
        first_segment: media_plane::ids::SegmentSeq,
    ) {
        self.record_track_specs(program, specs, Some(first_segment));
    }

    fn record_track_specs(
        &self,
        program: ProgramId,
        specs: Vec<TrackSpec>,
        first_segment: Option<media_plane::ids::SegmentSeq>,
    ) {
        let specs = self.with_declared_audio_profiles(specs);
        match self.serving(program) {
            Some(serving) => {
                let mapped = crate::source::split_audio::resolve_sources(self, &specs);
                serving.set_track_specs(specs, &mapped, first_segment)
            }
            None => {
                tracing::warn!(
                    ?program,
                    "RouteHandle::set_track_specs: program not published yet"
                )
            }
        }
    }

    /// The track specs that describe `segment` of `program`'s window.
    pub(crate) fn track_specs_at(
        &self,
        program: ProgramId,
        segment: media_plane::ids::SegmentSeq,
    ) -> Vec<TrackSpec> {
        self.serving(program)
            .map(|s| s.track_specs_at(segment))
            .unwrap_or_default()
    }

    /// Record whether `program` has muxable media now. `false` after the
    /// source drops every muxable track; the segments already published and
    /// the specs describing them are untouched.
    pub(crate) fn set_tracks_active(&self, program: ProgramId, active: bool) {
        if let Some(serving) = self.serving(program) {
            serving.set_tracks_active(active);
        }
    }

    /// `true` while `program` has muxable media; `false` once the source has
    /// dropped every muxable track (and until it announces one again), or if
    /// `program` is not registered.
    pub fn has_active_tracks(&self, program: ProgramId) -> bool {
        self.serving(program).is_some_and(|s| s.tracks_active())
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
    /// into `program`'s `Trunk` segment log. Returns [`AddSegmentError`]
    /// instead of publishing if `program` has not been published yet, if its
    /// `Trunk`'s segment writer is already held by a real
    /// `crate::source::segment::ProgramSegmenter`, or if the `Trunk` refused
    /// the segment.
    pub fn add_segment(
        &self,
        program: ProgramId,
        info: transmux::ll_hls::SegmentInfo,
    ) -> Result<(), AddSegmentError> {
        match self.serving(program) {
            Some(serving) => serving.add_segment(info),
            None => Err(AddSegmentError::ProgramNotPublished),
        }
    }

    /// `program`'s currently-resident closed-segment window, oldest first —
    /// the `Trunk`-drained replacement for `MediaStore::window_segments`.
    /// Empty if `program` is not registered.
    pub fn window_segments(&self, program: ProgramId) -> Vec<DashWindowSegment> {
        self.serving(program)
            .map(|serving| serving.window_segments())
            .unwrap_or_default()
    }

    /// `program`'s DASH window: [`Self::window_segments`] after the segments of replaced
    /// programs it still lists (see `ProgramServing::dash_window_segments`).
    pub(crate) fn dash_window_segments(&self, program: ProgramId) -> Vec<DashWindowSegment> {
        self.serving(program)
            .map(|serving| serving.dash_window_segments())
            .unwrap_or_default()
    }

    /// Configured target full-segment duration, seconds — route-wide (every
    /// program on a route shares one segmenter configuration), so this needs
    /// no [`ProgramId`].
    pub fn target_duration_secs(&self) -> f64 {
        self.target_duration_secs
    }

    /// Configured LL-HLS part target — route-wide, see
    /// [`Self::target_duration_secs`].
    pub fn part_target(&self) -> Duration {
        self.part_target
    }

    /// The wall-clock instant `program`'s presentation began (its first
    /// `Trunk` was minted; a reconnect keeps it) — `MPD@availabilityStartTime`'s source
    /// (`crate::output::dash`/`crate::output::ll_dash`, ISO/IEC 23009-1
    /// §5.3.1.2). `None` if `program` is not registered. Unlike
    /// [`Self::created_at`] it starts with ingest, not with the route.
    pub fn presentation_origin(&self, program: ProgramId) -> Option<SystemTime> {
        self.serving(program).map(|s| s.presentation_origin())
    }

    pub fn epoch(&self, program: ProgramId) -> Option<ProgramEpoch> {
        self.serving(program).map(|s| s.epoch())
    }

    /// When this route was constructed (admin status only; NOT the DASH
    /// `availabilityStartTime` — see [`Self::presentation_origin`]).
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// This route's current [`HealthState`] — route-wide ingest health, not
    /// per-program.
    pub fn health(&self) -> HealthState {
        *self.health.lock()
    }

    /// Transition this route's [`HealthState`].
    ///
    /// Also mirrors the state into the `crate::prometheus::ROUTE_UP` gauge
    /// for this route's name: `Live` is reached from the ingest facade
    /// (`crate::source::report_driver_progress`), which has no route name of
    /// its own, so the supervisor — the only other writer — never saw the
    /// transition and the gauge never read `1` (issue #1083).
    pub fn set_health(&self, state: HealthState) {
        *self.health.lock() = state;
        let up = if state.is_serving() { 1.0 } else { 0.0 };
        metrics::gauge!(crate::prometheus::ROUTE_UP, "route" => self.name.clone()).set(up);
    }

    /// Drain every published program's DVR pinning cursor (if DVR is
    /// enabled on this route) — called by
    /// [`crate::source::advance_route`] once per iteration, after
    /// `drive_program_segmenters` has published any new segments.
    ///
    /// Async (issue #1083, W3): each program's persist runs on the blocking
    /// pool rather than on the calling runtime worker. Programs are drained
    /// sequentially so their recordings stay in publish order.
    pub(crate) async fn drain_dvr(&self) {
        let servings: Vec<Arc<ProgramServing>> = self.programs.read().values().cloned().collect();
        for serving in servings {
            serving.poll_dvr().await;
        }
    }

    /// Feed raw TS bytes to every published program's DVR recorder for EIT
    /// p/f tracking (issue #903) — called by the TS-carrying ingest
    /// sources (`source::ts_udp`/`source::ts_http`/`source::srt`) with
    /// exactly the same bytes they feed their `IngestDriver`. A no-op for
    /// every route with DVR disabled or `DvrConfig::dvb_service_id` unset
    /// — which is every non-DVB source, since only `source::ts_udp`/
    /// `source::ts_http`/`source::srt` call this at all.
    ///
    /// This route's `DvrConfig` (including `dvb_service_id`) is shared
    /// across every program on the route, so on a genuinely multi-program
    /// (MPTS) route every program's recorder sees the same bytes and only
    /// the one tracking a matching `service_id` reacts — see the module
    /// doc on `dvr` for this limitation.
    pub(crate) fn feed_si_ts(&self, ts_bytes: &[u8]) {
        let programs = self.programs.read();
        for serving in programs.values() {
            serving.feed_si(ts_bytes);
        }
    }
}

/// Conversion helper so [`RouteHandle::with_declared_bandwidth_bps`] accepts
/// either a `u64` or an `Option<u64>`.
pub trait IntoDeclaredBandwidth {
    fn into_declared_bandwidth(self) -> Option<u64>;
}

impl IntoDeclaredBandwidth for u64 {
    fn into_declared_bandwidth(self) -> Option<u64> {
        Some(self)
    }
}

impl IntoDeclaredBandwidth for Option<u64> {
    fn into_declared_bandwidth(self) -> Option<u64> {
        self
    }
}

impl AsRef<RouteHandle> for RouteHandle {
    fn as_ref(&self) -> &RouteHandle {
        self
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
    use crate::testutil::video_spec;

    /// A minimal standalone `Trunk`, distinct in identity from any other
    /// call's — every ring capacity is `1` since these tests never publish
    /// samples/segments/parts/events, only check `Arc` identity through the
    /// registry.
    fn test_trunk() -> Arc<Trunk> {
        Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(1))
                .with_sparse_capacity(nz(1))
                .with_segment_capacity(nz(1))
                .with_event_capacity(nz(1))
                .with_part_capacity(nz(1)),
        )
    }

    /// MUTATION VERIFIED: changing `publish_program` to insert a
    /// freshly-constructed one-slot-per-ring `Trunk` (every `TrunkConfig` capacity set to
    /// `nz(1)`)-backed bundle instead of the `trunk` argument it was
    /// actually given makes this test fail:
    /// `assert_eq!(Arc::as_ptr(&resolved.trunk()), Arc::as_ptr(&trunk))` fails
    /// (via `ProgramServing::trunk()`), comparing two genuinely different heap
    /// addresses (e.g. `0x1055f3ab0 != 0x1055f4120` — exact addresses vary per
    /// run) because the registry silently bound a different `Trunk` to the
    /// published program. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn publish_then_resolve_returns_same_trunk() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
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
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
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
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
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
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
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

    /// Coverage for issue W7 (second concurrent publisher takes over/freezes
    /// a route): once program 1 is bound to `trunk_a`, a second
    /// `publish_program` call for the same `ProgramId` with a *different*
    /// `Arc` (a second, still-live connection publishing concurrently) must
    /// be rejected outright — the route keeps serving `trunk_a`, not
    /// silently rebound to `trunk_b`.
    #[test]
    fn publish_program_rejects_a_second_concurrent_publisher() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let trunk_a = test_trunk();
        let trunk_b = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk_a));
        route.publish_program(ProgramId(1), Arc::clone(&trunk_b));

        match route.resolve_program(ProgramId(1)) {
            ProgramResolution::Found(resolved) => {
                assert_eq!(
                    Arc::as_ptr(&resolved.trunk()),
                    Arc::as_ptr(&trunk_a),
                    "a second concurrent publisher must not take over the route"
                );
            }
            _ => panic!("expected ProgramResolution::Found (the first publisher's binding)"),
        }
    }

    /// A route built over a non-finite target duration (one that bypassed
    /// `Config::validate`) must refuse to publish — logging, not panicking
    /// in `ProgramServing::new` — and must give the publish slot back.
    #[test]
    fn publish_program_with_nan_target_duration_does_not_panic() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(f64::NAN)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let trunk = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk));
        assert!(
            matches!(
                route.resolve_program(ProgramId(1)),
                ProgramResolution::NotYetAnnounced | ProgramResolution::NotFound
            ),
            "an unservable route must not publish the program"
        );
        assert!(
            &route.active_publisher.read().is_empty(),
            "the publish slot must be released so a retry can bind"
        );
    }

    /// `release_program` is the counterpart a reaped session's own
    /// driver-backed source calls: once the first publisher's session has
    /// genuinely ended and released its binding, a *new* publisher (a
    /// reconnect, or a backup encoder taking over) may then bind — this is
    /// what stops the second-publisher rejection above from also permanently
    /// wedging the route after the first publisher legitimately disconnects.
    #[test]
    fn release_program_then_publish_lets_a_new_publisher_take_over() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let trunk_a = test_trunk();
        let trunk_b = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk_a));

        route.release_program(ProgramId(1), &trunk_a);
        route.publish_program(ProgramId(1), Arc::clone(&trunk_b));

        match route.resolve_program(ProgramId(1)) {
            ProgramResolution::Found(resolved) => {
                assert_eq!(
                    Arc::as_ptr(&resolved.trunk()),
                    Arc::as_ptr(&trunk_b),
                    "a new publisher must be able to bind once the old one released"
                );
            }
            _ => panic!("expected ProgramResolution::Found (the new publisher's binding)"),
        }
    }

    /// r09-W11 (#1089): a reconnect binds a fresh `Trunk`, which numbers its
    /// segments from 1 again; the replacement origin must continue the
    /// previous origin's Media Sequence Number instead of restarting it
    /// (RFC 8216bis §6.2.2: it MUST NOT decrease).
    #[test]
    fn reconnect_continues_the_media_sequence_number() {
        use hls_runtime::server::{BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsRequest};
        use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};

        let info = |seq: u32| transmux::ll_hls::SegmentInfo::new(vec![seq as u8; 8], 4.0, seq);
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let first_trunk = route.publish_new_program(ProgramId(1));
        route.add_segment(ProgramId(1), info(1)).expect("add");
        route.add_segment(ProgramId(1), info(2)).expect("add");
        let before = route.ll_hls(ProgramId(1)).expect("origin");
        assert_eq!(before.next_media_sequence(), 3);

        route.release_program(ProgramId(1), &first_trunk);
        route.publish_new_program(ProgramId(1));
        route.add_segment(ProgramId(1), info(1)).expect("add");
        let after = route.ll_hls(ProgramId(1)).expect("replacement origin");
        // Run 1's open segment (number 3) is skipped: the first segment of the
        // replacement is shown as 4.
        assert_eq!(after.next_media_sequence(), 5);

        let playlist = match after.resolve(
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
            broadcast_common::Timestamp::from_nanos(1),
            AwaitPolicy::new(broadcast_common::Timestamp::from_nanos(1_000_000_000)),
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(p),
                ..
            } => p,
            other => panic!("expected a playlist, got {other:?}"),
        };
        assert!(playlist.contains("#EXT-X-MEDIA-SEQUENCE:4\n"), "{playlist}");
        assert!(
            playlist
                .lines()
                .any(|l| l.starts_with("seg-1-") && l.ends_with("-4.m4s")),
            "{playlist}"
        );
    }

    /// A route that replaces another continues its numbering: its first program's media
    /// sequence number starts above the replaced route's next one, its epoch (DASH start number,
    /// media timeline start) carries on, and the replaced route's own numbering is untouched.
    #[test]
    fn a_replacing_route_continues_the_replaced_routes_numbering() {
        let old = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        )
        .with_archive_floor(41);
        let trunk = test_trunk();
        old.publish_program(ProgramId(1), Arc::clone(&trunk));
        let old_next = old
            .ll_hls(ProgramId(1))
            .expect("origin")
            .next_media_sequence();
        assert_eq!(old_next, 42);

        let successor = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        successor.continue_from(&old);
        successor.publish_program(ProgramId(1), test_trunk());
        let origin = successor.ll_hls(ProgramId(1)).expect("origin");
        assert_eq!(origin.media_sequence_offset(), old_next);
        assert_eq!(origin.next_media_sequence(), old_next + 1);
        let (old_epoch, epoch) = (
            old.epoch(ProgramId(1)).expect("old epoch"),
            successor.epoch(ProgramId(1)).expect("epoch"),
        );
        assert_eq!(epoch.first_number, old_epoch.first_number);
        assert_eq!(epoch.presentation_origin, old_epoch.presentation_origin);
        assert!(
            epoch.media_start_ns > 0,
            "the timeline carries on, so the segmenter marks a discontinuity"
        );

        let unrelated = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        unrelated.publish_program(ProgramId(1), test_trunk());
        assert_eq!(
            unrelated
                .ll_hls(ProgramId(1))
                .expect("origin")
                .media_sequence_offset(),
            0,
            "a route that replaces nothing starts from zero"
        );
    }

    /// Audit r07-C5/C2 (#1083): a process restart must not reuse numbers the
    /// DVR archive already holds. With an archive floor the first program's
    /// segments start above it, and a later reconnect continues from whichever
    /// is higher.
    #[test]
    fn archive_floor_lifts_the_first_media_sequence_number() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        )
        .with_archive_floor(41);
        let first = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&first));
        let origin = route.ll_hls(ProgramId(1)).expect("origin");
        // No segment yet: the next one is shown as floor + 1.
        assert_eq!(origin.next_media_sequence(), 42);
        assert_eq!(origin.media_sequence_offset(), 41);

        // A reconnect over a fresh Trunk keeps climbing, skipping the old
        // origin's open segment number (42).
        route.release_program(ProgramId(1), &first);
        route.publish_program(ProgramId(1), test_trunk());
        let replacement = route.ll_hls(ProgramId(1)).expect("origin");
        assert_eq!(replacement.next_media_sequence(), 43);
    }

    /// The names a route's origin advertises (init, segments, parts).
    fn advertised_names(route: &RouteHandle, program: ProgramId) -> Vec<String> {
        use hls_runtime::server::{BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsRequest};
        use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
        let origin = route.ll_hls(program).expect("origin");
        let body = match origin.resolve(
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
            broadcast_common::Timestamp::from_nanos(1),
            AwaitPolicy::new(broadcast_common::Timestamp::from_nanos(1_000_000_000)),
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(p),
                ..
            } => p,
            other => panic!("expected a playlist, got {other:?}"),
        };
        let mut names = Vec::new();
        for line in body.lines() {
            if let Some(at) = line.find("URI=\"") {
                let rest = &line[at + 5..];
                names.push(rest[..rest.find('"').unwrap_or(rest.len())].to_string());
            } else if !line.starts_with('#') && !line.is_empty() {
                names.push(line.to_string());
            }
        }
        names
    }

    /// Issue #2611: a replacement's init generation follows the bytes it is seeded with. An init
    /// identical to the one the replaced route last published stays in that generation (so DASH
    /// keeps one Period); a different one starts the next generation at the replacement's first
    /// segment, which DASH renders as a new Period.
    #[test]
    fn a_replacement_starts_a_new_init_generation_only_when_its_init_differs() {
        let info = |seq: u32, byte: u8| {
            transmux::ll_hls::SegmentInfo::new(vec![byte; 8], 4.0, seq).with_part_count(1)
        };
        let route = || {
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration_secs(4.0)
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
        };
        let replace = |old_init: Vec<u8>, new_init: Vec<u8>| {
            let old = route();
            old.publish_new_program(ProgramId(1));
            old.set_init(ProgramId(1), old_init);
            old.add_segment(ProgramId(1), info(1, 0x11)).expect("add");
            let successor = route();
            successor.continue_from(&old);
            successor.publish_new_program(ProgramId(1));
            successor.set_init(ProgramId(1), new_init);
            successor
                .add_segment(ProgramId(1), info(1, 0x21))
                .expect("add");
            let generation = successor
                .window_segments(ProgramId(1))
                .first()
                .map(|segment| segment.init_generation.value());
            (old, successor, generation)
        };

        let (_, _same, generation) = replace(vec![0xA1; 4], vec![0xA1; 4]);
        assert_eq!(
            generation,
            Some(1),
            "identical bytes stay in the generation the viewer already has"
        );
        let init_url = |route: &RouteHandle| {
            advertised_names(route, ProgramId(1))
                .into_iter()
                .find(|name| name.starts_with("init"))
                .unwrap_or_else(|| panic!("no init in the playlist"))
        };

        let (old, changed, generation) = replace(vec![0xA1; 4], vec![0xB2; 4]);
        assert_eq!(
            generation,
            Some(2),
            "a different init opens the next generation at the replacement's first segment"
        );
        assert_ne!(
            init_url(&old),
            init_url(&changed),
            "a new generation is a new init URL"
        );
        let urls = advertised_names(&changed, ProgramId(1));
        assert_eq!(
            urls.iter().filter(|name| name.starts_with("init")).count(),
            1,
            "one init URL: the first segment after the hand-off: {urls:?}"
        );
    }

    /// **A10-2.** A new init generation begins at the segment the caller names,
    /// not at the trunk's last closed segment: a segment closed but not yet
    /// published (stalled DVR pin) still belongs to the old init.
    #[test]
    fn set_init_from_starts_the_new_init_at_the_named_segment() {
        let info = |seq: u32, byte: u8| {
            transmux::ll_hls::SegmentInfo::new(vec![byte; 8], 4.0, seq).with_part_count(1)
        };
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(8),
        );
        route.publish_new_program(ProgramId(1));
        route.set_init(ProgramId(1), vec![0xA1; 4]);
        route.add_segment(ProgramId(1), info(1, 0x11)).expect("add");
        route.set_init_from(
            ProgramId(1),
            vec![0xB2; 4],
            media_plane::ids::SegmentSeq::new(3),
        );
        route.add_segment(ProgramId(1), info(2, 0x12)).expect("add");
        route.add_segment(ProgramId(1), info(3, 0x13)).expect("add");
        let names = advertised_names(&route, ProgramId(1));
        let maps: Vec<&String> = names.iter().filter(|n| n.starts_with("init")).collect();
        let segment_names: Vec<&String> = names.iter().filter(|n| n.starts_with("seg")).collect();
        assert_eq!(segment_names.len(), 3, "{names:?}");
        let position = |needle: &String| names.iter().position(|n| n == needle);
        let second_init = maps.last().and_then(|m| position(m)).expect("second init");
        assert_eq!(
            position(segment_names[1]).map(|p| p < second_init),
            Some(true),
            "segment 2 must still be served under the first init: {names:?}"
        );
        assert_eq!(
            position(segment_names[2]).map(|p| p > second_init),
            Some(true),
            "segment 3 is the first under the second init: {names:?}"
        );
    }

    /// Audit r09-C2 (#1030), restart with no DVR: a second "process" (a fresh
    /// `RouteHandle` renumbering from 1, with no archive to raise the floor)
    /// advertises none of the names the first one served, so nothing a cache
    /// holds `immutable` is ever reissued for other bytes — even though the
    /// media sequence numbers coincide. Also: the reconnect inside one process
    /// does not reuse the old origin's open segment's parts.
    #[test]
    fn a_restart_without_dvr_and_a_reconnect_never_reuse_an_immutable_name() {
        let info = |seq: u32, byte: u8| {
            transmux::ll_hls::SegmentInfo::new(vec![byte; 8], 4.0, seq).with_part_count(1)
        };
        // "Process" 1: segments 1..=2 served, part 0 of the open segment 3.
        let p1 = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let t1 = p1.publish_new_program(ProgramId(1));
        p1.set_init(ProgramId(1), vec![0xA1; 4]);
        p1.add_segment(ProgramId(1), info(1, 0x11)).expect("add");
        p1.add_segment(ProgramId(1), info(2, 0x12)).expect("add");
        p1.add_part(
            ProgramId(1),
            transmux::ll_hls::PartInfo::new(vec![0x13; 4], 0.5, 3, 0).with_independent(true),
        );
        let names1 = advertised_names(&p1, ProgramId(1));
        assert!(names1.iter().any(|n| n.starts_with("part-")), "{names1:?}");

        // "Process" 2: same numbers from scratch, different init.
        let p2 = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        p2.publish_new_program(ProgramId(1));
        p2.set_init(ProgramId(1), vec![0xB2; 4]);
        p2.add_segment(ProgramId(1), info(1, 0x21)).expect("add");
        p2.add_segment(ProgramId(1), info(2, 0x22)).expect("add");
        p2.add_part(
            ProgramId(1),
            transmux::ll_hls::PartInfo::new(vec![0x23; 4], 0.5, 3, 0).with_independent(true),
        );
        let names2 = advertised_names(&p2, ProgramId(1));
        for n in &names2 {
            assert!(!names1.contains(n), "{n} advertised by both processes");
        }

        // A reconnect inside process 1: the old open segment (3) and its part
        // are not advertised again, under any name.
        p1.release_program(ProgramId(1), &t1);
        p1.publish_new_program(ProgramId(1));
        p1.set_init(ProgramId(1), vec![0xC3; 4]);
        p1.add_segment(ProgramId(1), info(1, 0x31)).expect("add");
        p1.add_part(
            ProgramId(1),
            transmux::ll_hls::PartInfo::new(vec![0x32; 4], 0.5, 2, 0).with_independent(true),
        );
        let names3 = advertised_names(&p1, ProgramId(1));
        for n in &names3 {
            assert!(!names1.contains(n), "{n} reissued after the reconnect");
        }
        assert!(
            names3.iter().all(|n| !n.contains("-3.")),
            "the old open segment's number 3 was reused: {names3:?}"
        );
    }

    /// `release_program` must be a no-op for a `Trunk` that lost the
    /// take-over race in `publish_program` and so was never actually bound —
    /// otherwise a rejected session's own eventual reap could release the
    /// *surviving* publisher's binding out from under it.
    #[test]
    fn release_program_is_a_noop_for_a_rejected_publisher() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let trunk_a = test_trunk();
        let trunk_b = test_trunk();
        route.publish_program(ProgramId(1), Arc::clone(&trunk_a));
        route.publish_program(ProgramId(1), Arc::clone(&trunk_b)); // rejected

        route.release_program(ProgramId(1), &trunk_b);

        match route.resolve_program(ProgramId(1)) {
            ProgramResolution::Found(resolved) => {
                assert_eq!(
                    Arc::as_ptr(&resolved.trunk()),
                    Arc::as_ptr(&trunk_a),
                    "releasing a trunk that was never actually bound must not \
                     evict the surviving publisher"
                );
            }
            _ => panic!("expected ProgramResolution::Found (still trunk_a)"),
        }
    }

    fn seg(seq: u32, duration: f64) -> transmux::ll_hls::SegmentInfo {
        transmux::ll_hls::SegmentInfo::new(vec![seq as u8; 8], duration, seq).with_part_count(1)
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
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(1.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(8),
        );
        let program_a = ProgramId(1);
        let program_b = ProgramId(2);

        route.publish_new_program(program_a);
        route.publish_new_program(program_b);

        route.set_init(program_a, &b"init-1"[..]);
        route.set_init(program_b, &b"init-2"[..]);
        route.set_track_specs(program_a, vec![video_spec(TrackId::new(1))]);
        route.set_track_specs(program_b, vec![video_spec(TrackId::new(2))]);
        route
            .add_segment(program_a, seg(10, 2.0))
            .expect("add_segment");
        route
            .add_segment(program_b, seg(20, 2.0))
            .expect("add_segment");
        route
            .add_segment(program_b, seg(21, 2.0))
            .expect("add_segment");

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
            window_a
                .iter()
                .map(|s| s.segment_seq.value())
                .collect::<Vec<_>>(),
            vec![10],
            "program 1 must show only its own segment"
        );
        assert_eq!(
            window_b
                .iter()
                .map(|s| s.segment_seq.value())
                .collect::<Vec<_>>(),
            vec![20, 21],
            "program 2 must show only its own (different-count) segments"
        );

        assert_eq!(
            route
                .track_specs(program_a)
                .iter()
                .map(|s| s.track_id)
                .collect::<Vec<_>>(),
            vec![TrackId::new(1)],
            "program 1's own track spec"
        );
        assert_eq!(
            route
                .track_specs(program_b)
                .iter()
                .map(|s| s.track_id)
                .collect::<Vec<_>>(),
            vec![TrackId::new(2)],
            "program 2's own, DIFFERENT track spec"
        );
    }

    /// A request for a program that has never been announced on this route
    /// resolves `NotYetAnnounced` — the wait/503 case — never `NotFound`
    /// (404), matching `crate::http::resolve_route_program`'s mapping. See
    /// `crate::output::hls`'s/`crate::origin::resource`'s own
    /// `*_not_yet_announced_is_503_not_404` tests for the same property
    /// proven through the real HTTP handlers.
    #[test]
    fn unannounced_program_is_not_yet_announced_not_not_found() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        assert!(matches!(
            route.resolve_program(ProgramId(7)),
            ProgramResolution::NotYetAnnounced
        ));
    }

    /// `RouteHandle::name` defaults to the documented sentinel and reflects
    /// whatever `with_name` set.
    #[test]
    fn name_defaults_then_reflects_with_name() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        assert_eq!(route.name(), DEFAULT_ROUTE_NAME);
        let route = route.with_name("cam1");
        assert_eq!(route.name(), "cam1");
    }

    /// `RouteHandle::add_segment` for a program id that was never published
    /// returns [`AddSegmentError::ProgramNotPublished`] instead of silently
    /// dropping the segment.
    #[test]
    fn add_segment_for_unpublished_program_returns_error() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(4),
        );
        let err = route
            .add_segment(ProgramId(9), seg(1, 2.0))
            .expect_err("add_segment for a never-published program must return an error");
        assert!(matches!(err, AddSegmentError::ProgramNotPublished));
    }

    /// I1: `DvrRecorder` holds multi-step state (`write_offset`/`index`/
    /// `periods`/`total_bytes`); a panic during persist can leave it
    /// disagreeing, so recording must FAIL CLOSED (stop, count it) rather than
    /// keep writing a corrupt archive. `parking_lot` releases the guard without
    /// poisoning, so the safety property is kept by `catch_unwind` in
    /// `poll_dvr_blocking`, not by poisoning.
    ///
    /// Revert-check: remove the `catch_unwind` wrap (persist the recorder
    /// directly) and the panic unwinds out of `poll_dvr`, so the recorder is
    /// never dropped and no counter increments — both assertions fail.
    #[tokio::test]
    async fn a_panicking_dvr_persist_stops_recording_and_counts_it() {
        let tmp = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "multimux-dvr-fail-closed-{}-{}",
                std::process::id(),
                n
            ));
            let _ = std::fs::create_dir_all(&dir);
            dir
        };
        let dvr_cfg = crate::dvr::DvrConfig {
            enabled: true,
            archive_root: tmp.to_string_lossy().to_string(),
            retention_periods: 5,
            retention_bytes: 0,
            period_duration_secs: 3600,
            overrun: crate::dvr::ArchiveOverrunSerde::Gap,
            dvb_service_id: None,
        };
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(1.0)
                .with_part_target(std::time::Duration::from_millis(250))
                .with_window_segments(8),
        )
        .with_name("panicking-dvr")
        .with_dvr(dvr_cfg);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);

        crate::prometheus::install();
        let route_name = "panicking-dvr";
        let before = dvr_failed_metric(route_name);

        // Arm the injected panic and drive the production entry point.
        let serving = route.serving(SPTS_PROGRAM_ID).expect("program published");
        serving
            .panic_on_persist
            .store(true, std::sync::atomic::Ordering::SeqCst);
        serving.poll_dvr().await;

        assert!(
            serving.dvr.lock().is_none(),
            "a panic during persist must abandon the recorder (fail closed)"
        );
        assert_eq!(
            dvr_failed_metric(route_name),
            before + 1.0,
            "the abandoned recorder must be counted in multimux_dvr_failed_total"
        );

        // A second poll is a no-op (already dropped) — the counter does not
        // climb again, and nothing is written.
        serving.poll_dvr().await;
        assert_eq!(
            dvr_failed_metric(route_name),
            before + 1.0,
            "an already-abandoned recorder must not be counted twice"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn archived_periods(root: &std::path::Path) -> usize {
        fn count(dir: &std::path::Path) -> usize {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        count(&path)
                    } else {
                        usize::from(path.extension().is_some_and(|e| e == "m4s"))
                    }
                })
                .sum()
        }
        count(root)
    }

    /// An init change with no new closed segment is archived at once: `needs_poll` used to
    /// look only at the latest closed segment, so the period roll waited for the next close.
    #[tokio::test]
    async fn a_dvr_init_change_without_a_new_segment_rolls_the_period_at_once() {
        let tmp = std::env::temp_dir().join(format!("multimux-dvr-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let dvr_cfg = crate::dvr::DvrConfig {
            enabled: true,
            archive_root: tmp.to_string_lossy().to_string(),
            retention_periods: 5,
            retention_bytes: 0,
            period_duration_secs: 3600,
            overrun: crate::dvr::ArchiveOverrunSerde::Gap,
            dvb_service_id: None,
        };
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(1.0)
                .with_part_target(std::time::Duration::from_millis(250))
                .with_window_segments(8),
        )
        .with_name("init-roll")
        .with_dvr(dvr_cfg);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        route.add_segment(SPTS_PROGRAM_ID, seg(1, 1.0)).unwrap();
        let serving = route.serving(SPTS_PROGRAM_ID).expect("program published");
        serving.poll_dvr().await;
        assert_eq!(archived_periods(&tmp), 1);

        route.set_init(SPTS_PROGRAM_ID, vec![0xBB; 4]);
        serving.poll_dvr().await;
        assert_eq!(
            archived_periods(&tmp),
            2,
            "the new init waited for the next segment to roll the period"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Read a Prometheus counter value by name+route label (0.0 if absent).
    fn dvr_failed_metric(route_label: &str) -> f64 {
        crate::prometheus::install()
            .render()
            .lines()
            .find(|l| l.starts_with("multimux_dvr_failed_total") && l.contains(route_label))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    }

    /// Audit r07-C4 (#1083): `await_trunk_change` returns only once the first
    /// program is bound to a *different* `Trunk`, and the first program is the
    /// lowest-numbered one, not whichever the map happens to yield.
    #[tokio::test]
    async fn await_trunk_change_waits_for_a_different_trunk() {
        let route = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(1.0)
                .with_part_target(std::time::Duration::from_millis(250))
                .with_window_segments(4),
        ));
        let high = test_trunk();
        route.publish_program(ProgramId(5), Arc::clone(&high));
        let low = test_trunk();
        route.publish_program(ProgramId(2), Arc::clone(&low));
        let first = route.await_first_trunk().await;
        assert!(Arc::ptr_eq(&first, &low), "program 2 is the first program");

        // Still the same Trunk: the wait does not complete.
        let pending = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            route.await_trunk_change(&low),
        )
        .await;
        assert!(pending.is_err(), "no change yet");

        let waiter = {
            let route = Arc::clone(&route);
            let low = Arc::clone(&low);
            tokio::spawn(async move { route.await_trunk_change(&low).await })
        };
        // The source reconnects: slot released, a fresh Trunk takes over.
        route.release_program(ProgramId(2), &low);
        let replacement = test_trunk();
        route.publish_program(ProgramId(2), Arc::clone(&replacement));
        let seen = tokio::time::timeout(std::time::Duration::from_secs(10), waiter)
            .await
            .expect("the wait completes after the replacement")
            .unwrap();
        assert!(Arc::ptr_eq(&seen, &replacement));
    }
}
