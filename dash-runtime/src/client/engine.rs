//! [`DashClient`]: the sans-IO engine for static (VOD) and dynamic (live)
//! presentations. The live half (clock ladder, refresh, join, availability)
//! is in `live`.

mod live;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use container_probe::segment;
use transmux::{TrackId, TrackSpec};
use url::Url;

use super::action::{Action, RepId, ResourceId};
use super::config::{DashConfig, PullLimits};
use super::demux::{self, Staged};
use super::error::{Error, Failure, FailureClass, FailureKind, FatalKind, Rejection, Result};
use super::meta::ResponseMeta;
use super::output::{AudioRendition, DropCause, ManifestSelection, Notice, Output, VariantInfo};
use crate::address::base_index;
use crate::address::{AddressError, PlanError, PlanOptions, SegmentPlan, plan_representation};
use crate::mpd::descriptors::SCHEME_PERIOD_CONTINUITY;
use crate::mpd::inherit::{SegmentInfo, effective_common, resolve_segment_info};
use crate::mpd::model::{ByteRange, Descriptor, Mpd, PresentationType};
use crate::mpd::urls::{BaseCandidate, append_query, representation_candidates, segment_url_query};
use crate::selection::{RepRef, Selection, SetClass, select};
use crate::timing::clock::UtcClock;
use crate::timing::period::period_timings;

/// A missing last segment shorter than this, in microseconds, is the tail a
/// muxer declared past the media (one frame of the slowest common video
/// rate), not a lost segment.
const MISSING_TAIL_MAX_US: i128 = 50_000;
/// Consecutive failures on one BaseURL alternate before the next is used
/// (ETSI TS 103 285 10.8.2).
const ROTATE_AFTER_FAILURES: u32 = 2;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;
const STATUS_TOO_EARLY: u16 = 425;
/// Delivered resources remembered to answer a second delivery with
/// `DuplicateResource`; older ones are forgotten so a long live session stays
/// bounded.
const MAX_REMEMBERED_DONE: usize = 4096;
const START_WITH_SAP_1: u8 = 1;
const START_WITH_SAP_2: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    NeedMpd,
    Running,
    Ended,
    Fatal,
}

enum Candidate {
    Ready(i128),
    Wait(i128),
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct InitKey {
    url: String,
    range: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitState {
    Wanted,
    InFlight,
    Ready,
}

struct InitEntry {
    state: InitState,
    bytes: Option<Arc<Vec<u8>>>,
    specs: Vec<TrackSpec>,
    skipped_reported: BTreeSet<(Option<u32>, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReqKind {
    Init,
    Index,
    Segment,
}

struct ReqInfo {
    rep: RepId,
    reference: String,
    range: Option<(u64, u64)>,
    kind: ReqKind,
    avail_from_us: Option<i128>,
    avail_until_us: Option<i128>,
}

struct IndexReq {
    range: ByteRange,
}

struct RepState {
    id: RepId,
    at: RepRef,
    candidates: Vec<BaseCandidate>,
    cand: usize,
    cand_failures: u32,
    query: Option<String>,
    plan: Option<SegmentPlan>,
    index: Option<IndexReq>,
    init_key: InitKey,
    next_issue: u64,
    start_time_us: Option<i128>,
    dead: bool,
    set_id: Option<u64>,
    set_pos: usize,
    rep_key: String,
    joined: bool,
    last_time: Option<u64>,
}

enum SlotState {
    Issued,
    Held(Vec<u8>),
    Staged(Staged),
    Skipped { marker: bool },
    Marker,
}

struct Slot {
    id: Option<ResourceId>,
    rep: usize,
    plan_index: u64,
    state: SlotState,
}

struct Lane {
    kind: SetClass,
    set_id: Option<u64>,
    reps: Vec<RepState>,
    queue: VecDeque<Slot>,
    base: u32,
    count: u32,
    announced_rep: Option<usize>,
    announced_init: Option<InitKey>,
    expired_run: u32,
}

impl Lane {
    fn current(&self) -> Option<&RepState> {
        self.reps.last().filter(|r| !r.dead)
    }

    fn exhausted(&self) -> bool {
        match self.current() {
            None => true,
            Some(rep) => rep
                .plan
                .as_ref()
                .is_some_and(|p| p.len().is_some_and(|n| rep.next_issue >= n)),
        }
    }
}

struct PeriodRun {
    index: usize,
    id: Option<String>,
    start_us: i128,
    lanes: Vec<Lane>,
    selection: Selection,
    notices: Vec<Output>,
    started: bool,
    issue_done: bool,
    finished: bool,
}

struct PrevPeriod {
    id: Option<String>,
    lanes: Vec<(SetClass, Option<u64>, InitKey)>,
}

/// The sizes of a [`DashClient`]'s bookkeeping, see [`DashClient::state_size`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct StateSize {
    /// Period runs held (finished ones are dropped).
    pub periods: usize,
    /// Resources with a request on record.
    pub requests: usize,
    /// Requests awaiting an answer.
    pub inflight: usize,
    /// Failed resources awaiting the caller's retry.
    pub parked: usize,
    /// Resources with a consecutive-failure count.
    pub failures: usize,
    /// Delivered resources remembered for duplicate detection.
    pub done: usize,
    /// Initialization Segments held.
    pub inits: usize,
    /// Representations indexed.
    pub reps: usize,
    /// Fetches queued for the next poll.
    pub queued: usize,
    /// Outputs not yet drained.
    pub outputs: usize,
    /// Live-only bookkeeping entries (early waits, missing-segment flags).
    pub live: usize,
}

/// The sans-IO DASH client for static and dynamic presentations.
///
/// Drive it like `hls_runtime::client::HlsClient`: call [`DashClient::poll`]
/// for the next fetch, perform it, feed the result to the matching `on_*`
/// call (or [`DashClient::on_failure`]), and drain
/// [`DashClient::next_output`]. The core never opens a socket and never
/// reads a clock; the `now_us` arguments are the caller's monotonic clock.
/// A static presentation ignores it; a dynamic one maps it to UTC through an
/// anchor (a `UTCTiming` sample the client takes through its own requests,
/// [`DashClient::set_wall_clock`], or the MPD response `Date`) and uses that
/// to join near the live edge, never request a segment before it exists,
/// refresh the MPD and classify early, expired and missing segments. After
/// each batch of calls ask [`DashClient::next_wake`] when to call
/// [`DashClient::poll`] again if nothing else happens first.
pub struct DashClient {
    cfg: PullLimits,
    dash: DashConfig,
    mpd_url: String,
    doc_url: Option<Url>,
    mpd_query: Option<String>,
    phase: Phase,
    want_mpd: bool,
    mpd_inflight: bool,
    mpd: Option<Mpd>,
    periods: Vec<PeriodRun>,
    issue: usize,
    deliver: usize,
    next_rep: u32,
    reqs: BTreeMap<ResourceId, ReqInfo>,
    queue: VecDeque<ResourceId>,
    inflight: BTreeSet<ResourceId>,
    parked: BTreeSet<ResourceId>,
    failures: BTreeMap<ResourceId, u32>,
    done: BTreeSet<ResourceId>,
    inits: BTreeMap<InitKey, InitEntry>,
    rep_loc: BTreeMap<RepId, (usize, usize, usize)>,
    outputs: VecDeque<Output>,
    announced_tracks: Option<String>,
    prev: Option<PrevPeriod>,
    any_started: bool,
    outstanding: usize,
    notices_sent: Vec<Notice>,
    clock: UtcClock,
    mpd_failures: u32,
    live: Option<live::Live>,
    next_utc_id: u32,
    last_was_discontinuity: bool,
    remembered_done: VecDeque<ResourceId>,
}

fn without_query(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => url.to_owned(),
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn address_name(e: &AddressError) -> &'static str {
    match e {
        AddressError::InvalidTemplate { .. } => "invalid_template",
        AddressError::Unsupported(u) => u.name(),
        AddressError::MissingMediaTemplate => "missing_media_template",
        AddressError::MissingDuration => "missing_duration",
        AddressError::TimelineNotIncreasing { .. } => "timeline_not_increasing",
        AddressError::NumberNotIncreasing { .. } => "number_not_increasing",
        AddressError::TimelineTooLong { .. } => "timeline_too_long",
        AddressError::ListTimelineMismatch { .. } => "list_timeline_mismatch",
        AddressError::Sidx(_) => "bad_sidx",
        AddressError::Overflow => "overflow",
        AddressError::IndexRequired => "index_required",
        AddressError::EmptySegmentUrl => "empty_segment_url",
    }
}

fn plan_error_name(e: &PlanError) -> &'static str {
    match e {
        PlanError::NoSuchRepresentation => "no_such_representation",
        PlanError::Inherit(_) => "inherit",
        PlanError::Address(a) => address_name(a),
        PlanError::PeriodUnresolved => "period_unresolved",
        PlanError::MissingAvailabilityStartTime => "missing_availability_start_time",
    }
}

fn range_pair(r: Option<ByteRange>) -> std::result::Result<Option<(u64, u64)>, DropCause> {
    r.map(|r| r.offset_len().map_err(|u| DropCause::Addressing(u.name())))
        .transpose()
}

fn continuity_signals(set: &crate::mpd::model::AdaptationSet, previous: &str) -> bool {
    let named = |d: &Descriptor| {
        d.scheme_id_uri == SCHEME_PERIOD_CONTINUITY && d.value.as_deref() == Some(previous)
    };
    set.supplemental_properties.iter().any(named) || set.essential_properties.iter().any(named)
}

impl DashClient {
    /// A client for the MPD at `mpd_url`. Performs no I/O.
    #[must_use]
    pub fn new(mpd_url: impl Into<String>, dash: DashConfig, limits: PullLimits) -> Self {
        let mpd_url = mpd_url.into();
        let doc_url = Url::parse(&mpd_url).ok();
        let mpd_query = doc_url.as_ref().and_then(|u| u.query()).map(str::to_owned);
        Self {
            cfg: limits,
            dash,
            mpd_url,
            doc_url,
            mpd_query,
            phase: Phase::NeedMpd,
            want_mpd: true,
            mpd_inflight: false,
            mpd: None,
            periods: Vec::new(),
            issue: 0,
            deliver: 0,
            next_rep: 0,
            reqs: BTreeMap::new(),
            queue: VecDeque::new(),
            inflight: BTreeSet::new(),
            parked: BTreeSet::new(),
            failures: BTreeMap::new(),
            done: BTreeSet::new(),
            inits: BTreeMap::new(),
            rep_loc: BTreeMap::new(),
            outputs: VecDeque::new(),
            announced_tracks: None,
            prev: None,
            any_started: false,
            outstanding: 0,
            notices_sent: Vec::new(),
            clock: UtcClock::new(),
            mpd_failures: 0,
            live: None,
            next_utc_id: 0,
            last_was_discontinuity: false,
            remembered_done: VecDeque::new(),
        }
    }

    /// The MPD URL given to [`DashClient::new`].
    #[must_use]
    pub fn mpd_url(&self) -> &str {
        &self.mpd_url
    }

    /// Use `base` (the URL the MPD was finally served from, after redirects)
    /// as the document URL every relative reference resolves against
    /// (23009-1 A.11).
    pub fn set_mpd_base(&mut self, base: impl Into<String>) {
        self.doc_url = Url::parse(&base.into()).ok();
    }

    /// `Some(true)` for a static MPD, `Some(false)` for a dynamic one, `None`
    /// before the MPD arrived.
    #[must_use]
    pub fn is_static(&self) -> Option<bool> {
        self.mpd
            .as_ref()
            .map(|m| m.kind == PresentationType::Static)
    }

    /// Whether [`Output::EndOfStream`] was produced.
    #[must_use]
    pub fn presentation_ended(&self) -> bool {
        self.phase == Phase::Ended
    }

    /// The `@maxSegmentDuration` of the MPD, whole seconds rounded up.
    #[must_use]
    pub fn max_segment_duration_secs(&self) -> Option<u64> {
        let d = self.mpd.as_ref()?.max_segment_duration?;
        Some(d.as_secs() + u64::from(d.subsec_nanos() > 0))
    }

    /// The sizes of the client's bookkeeping, for tests and monitoring: every
    /// collection is bounded by the in-flight window and the Periods in play
    /// (a live session of any length does not grow them).
    #[must_use]
    pub fn state_size(&self) -> StateSize {
        StateSize {
            periods: self.periods.len(),
            requests: self.reqs.len(),
            inflight: self.inflight.len(),
            parked: self.parked.len(),
            failures: self.failures.len(),
            done: self.done.len(),
            inits: self.inits.len(),
            reps: self.rep_loc.len(),
            queued: self.queue.len(),
            outputs: self.outputs.len(),
            live: self.live.as_ref().map_or(0, live::Live::bookkeeping),
        }
    }

    /// Whether the client still waits for the answer to the fetch of `id`.
    /// An MPD reset (or a rejoin) abandons the requests of the old timeline:
    /// a caller may cancel those downloads; a late answer is rejected with
    /// [`Error::UnrequestedResource`] either way.
    #[must_use]
    pub fn awaits(&self, id: ResourceId) -> bool {
        self.inflight.contains(&id)
    }

    /// The next drained output, if any.
    pub fn next_output(&mut self) -> Option<Output> {
        self.outputs.pop_front()
    }

    fn stopped(&self) -> bool {
        matches!(self.phase, Phase::Ended | Phase::Fatal)
    }

    fn ensure_live(&self) -> Result<()> {
        if self.stopped() {
            Err(Error::Stopped)
        } else {
            Ok(())
        }
    }

    fn go_fatal(&mut self, kind: FatalKind) {
        if self.phase == Phase::Fatal {
            return;
        }
        self.phase = Phase::Fatal;
        self.queue.clear();
        self.outputs.push_back(Output::Fatal(kind));
    }

    fn discontinuity(&mut self) {
        if !self.last_was_discontinuity {
            self.last_was_discontinuity = true;
            self.outputs.push_back(Output::Discontinuity);
        }
    }

    fn mark_done(&mut self, id: ResourceId) {
        if self.done.insert(id) {
            self.remembered_done.push_back(id);
            while self.remembered_done.len() > MAX_REMEMBERED_DONE {
                if let Some(old) = self.remembered_done.pop_front() {
                    self.done.remove(&old);
                }
            }
        }
    }

    fn notice_once(&mut self, n: Notice) {
        if !self.notices_sent.contains(&n) {
            self.notices_sent.push(n);
            self.outputs.push_back(Output::Notice(n));
        }
    }

    // MPD ------------------------------------------------------------------

    /// Feed the MPD body of a response with no headers of interest (a static
    /// presentation, or a caller with no `Date`, `ETag` or redirect to
    /// report). See [`DashClient::on_mpd_with`].
    ///
    /// # Errors
    /// As [`DashClient::on_mpd_with`].
    pub fn on_mpd(&mut self, now_us: u64, bytes: &[u8]) -> Result<()> {
        self.on_mpd_with(now_us, bytes, &ResponseMeta::at(now_us))
    }

    /// Feed the MPD body with what the caller measured and read: the first
    /// MPD starts the presentation, a later one is a refresh of a dynamic
    /// presentation.
    ///
    /// # Errors
    /// [`Error::Mpd`] for a body that does not parse (request it again with
    /// [`DashClient::retry_mpd`]; the MPD already held stays in force);
    /// [`Error::MpdNotAwaited`] when no MPD is awaited; [`Error::Stopped`].
    pub fn on_mpd_with(
        &mut self,
        now_us: u64,
        bytes: &[u8],
        meta: &ResponseMeta<'_>,
    ) -> Result<()> {
        self.ensure_live()?;
        if !self.mpd_inflight {
            return Err(Error::MpdNotAwaited);
        }
        self.mpd_inflight = false;
        let mpd = match Mpd::parse_with(bytes, &self.cfg.limits) {
            Ok(m) => m,
            Err(e) => {
                self.mpd_failed(now_us);
                return Err(Error::Mpd(e));
            }
        };
        if self.mpd.is_some() {
            self.refresh_mpd(now_us, mpd, bytes, meta);
            return Ok(());
        }
        if mpd.periods.is_empty() {
            self.go_fatal(FatalKind::MpdInvalid {
                reason: "no_period",
            });
            return Ok(());
        }
        let dynamic = mpd.kind == PresentationType::Dynamic;
        if dynamic && mpd.availability_start_time_us.is_none() {
            self.go_fatal(FatalKind::MpdInvalid {
                reason: "missing_availability_start_time",
            });
            return Ok(());
        }
        let features = mpd.features;
        self.mpd = Some(mpd);
        self.phase = Phase::Running;
        if features.event_stream || features.inband_event_stream {
            self.notice_once(Notice::EventStreamIgnored);
        }
        if features.preselection {
            self.notice_once(Notice::PreselectionIgnored);
        }
        if features.patch_location {
            self.notice_once(Notice::PatchIgnored);
        }
        if dynamic && features.low_latency {
            self.notice_once(Notice::LowLatencyIgnored);
        }
        if dynamic {
            self.start_live(now_us, bytes, meta);
        } else {
            if let Some(u) = meta.final_url.and_then(|f| Url::parse(f).ok()) {
                self.doc_url = Some(u);
            }
            self.activate(0);
            self.issue = 0;
        }
        Ok(())
    }

    /// The MPD fetch failed; nothing is counted against the presentation, an
    /// MPD is never given up on. The caller backs off and calls
    /// [`DashClient::retry_mpd`]. The returned class carries the count of
    /// consecutive MPD failures.
    pub fn on_mpd_failure(&mut self, now_us: u64, failure: Failure) -> FailureClass {
        self.mpd_inflight = false;
        self.want_mpd = false;
        let attempt = self.mpd_failed(now_us);
        let kind = match failure {
            Failure::Status(STATUS_NOT_FOUND | STATUS_GONE) => FailureKind::Missing,
            Failure::Status(STATUS_TOO_EARLY) => FailureKind::TooEarly,
            Failure::Status(_) => FailureKind::Server,
            Failure::Transport | Failure::Timeout => FailureKind::Transport,
            Failure::TooLarge => FailureKind::Invalid,
        };
        FailureClass::Retry { attempt, kind }
    }

    /// Ask for the MPD again after a rejected body or a failed fetch.
    pub fn retry_mpd(&mut self, _now_us: u64) {
        if self.mpd_inflight || self.stopped() {
            return;
        }
        let waiting = self.phase == Phase::NeedMpd
            || self.live.as_ref().is_some_and(|l| l.refresh.awaiting_retry);
        if waiting {
            self.want_mpd = true;
        }
    }

    // Poll -----------------------------------------------------------------

    /// The next fetch the caller must perform, if any. Call it until it
    /// returns `None`, then wait for a response or for
    /// [`DashClient::next_wake`], whichever is first.
    pub fn poll(&mut self, now_us: u64) -> Option<Action> {
        if self.stopped() {
            return None;
        }
        self.tick(now_us);
        if self.stopped() {
            return None;
        }
        if let Some(a) = self.mpd_action(now_us) {
            return Some(a);
        }
        if self.phase != Phase::Running {
            return None;
        }
        if let Some(a) = self.utc_action(now_us) {
            return Some(a);
        }
        loop {
            if self.queue.is_empty() {
                self.fill_queue(now_us);
                if self.queue.is_empty() {
                    self.drain();
                }
            }
            let id = self.queue.pop_front()?;
            if let Some(action) = self.emit(id) {
                return Some(action);
            }
            if self.stopped() {
                return None;
            }
        }
    }

    fn window_open(&self) -> bool {
        self.outstanding < self.cfg.window()
    }

    fn fill_queue(&mut self, now_us: u64) {
        self.reset_wake();
        loop {
            let Some(pi) = (self.issue < self.periods.len()).then_some(self.issue) else {
                return;
            };
            if self.live_blocked() {
                return;
            }
            let exhausted = self.periods[pi].lanes.iter().all(Lane::exhausted);
            if self.periods[pi].issue_done || (exhausted && !self.period_open(pi, now_us)) {
                self.periods[pi].issue_done = true;
                let next = self.periods[pi].index + 1;
                if self.can_activate(next) {
                    self.activate(next);
                    self.issue = self.periods.len() - 1;
                    continue;
                }
                return;
            }
            if exhausted {
                self.note_exhausted(now_us);
                return;
            }
            if !self.window_open() {
                return;
            }
            let utc = self.utc_now(now_us);
            let mut pick: Option<(usize, i128)> = None;
            for li in 0..self.periods[pi].lanes.len() {
                match self.lane_candidate(pi, li, utc, now_us) {
                    Candidate::Ready(start_us) => {
                        if pick.is_none_or(|(_, best)| start_us < best) {
                            pick = Some((li, start_us));
                        }
                    }
                    Candidate::Wait(at_utc_us) => self.note_wake(at_utc_us),
                    Candidate::Idle => {}
                }
            }
            let Some((li, _)) = pick else {
                return;
            };
            if !self.issue_segment(pi, li) {
                return;
            }
        }
    }

    fn lane_candidate(
        &mut self,
        pi: usize,
        li: usize,
        utc: Option<i128>,
        now_us: u64,
    ) -> Candidate {
        match utc {
            Some(utc) if self.is_live_mpd() => self.live_lane_candidate(pi, li, utc, now_us),
            _ => {
                let lane = &self.periods[pi].lanes[li];
                let Some(rep) = lane.current() else {
                    return Candidate::Idle;
                };
                let Some(plan) = rep.plan.as_ref() else {
                    return Candidate::Idle;
                };
                plan.get(rep.next_issue)
                    .map_or(Candidate::Idle, |s| Candidate::Ready(s.start_us))
            }
        }
    }

    fn issue_segment(&mut self, pi: usize, li: usize) -> bool {
        let lane = &mut self.periods[pi].lanes[li];
        let ri = lane.reps.len() - 1;
        let rep = &mut lane.reps[ri];
        let Some(seg) = rep.plan.as_ref().and_then(|p| p.get(rep.next_issue)) else {
            return false;
        };
        let range = match range_pair(seg.byte_range) {
            Ok(r) => r,
            Err(cause) => {
                let reason = match cause {
                    DropCause::Addressing(r) => r,
                    _ => "unusable_url",
                };
                self.go_fatal(FatalKind::UnsupportedAddressing { reason });
                return false;
            }
        };
        let id = ResourceId::Segment {
            rep: rep.id,
            number: seg.number,
        };
        let plan_index = rep.next_issue;
        rep.next_issue += 1;
        rep.last_time = Some(seg.time_ticks);
        let rep_id = rep.id;
        lane.queue.push_back(Slot {
            id: Some(id),
            rep: ri,
            plan_index,
            state: SlotState::Issued,
        });
        self.outstanding += 1;
        self.reqs.insert(
            id,
            ReqInfo {
                rep: rep_id,
                reference: seg.reference,
                range,
                kind: ReqKind::Segment,
                avail_from_us: seg.available_from_us,
                avail_until_us: seg.available_until_us,
            },
        );
        self.queue.push_back(id);
        true
    }

    fn rep(&self, id: RepId) -> Option<&RepState> {
        let (p, l, r) = *self.rep_loc.get(&id)?;
        self.periods.get(p)?.lanes.get(l)?.reps.get(r)
    }

    fn rep_mut(&mut self, id: RepId) -> Option<&mut RepState> {
        let (p, l, r) = *self.rep_loc.get(&id)?;
        self.periods.get_mut(p)?.lanes.get_mut(l)?.reps.get_mut(r)
    }

    fn resolve(
        &self,
        rep: &RepState,
        reference: &str,
        media: bool,
    ) -> std::result::Result<String, DropCause> {
        let cand = rep
            .candidates
            .get(rep.cand % rep.candidates.len().max(1))
            .ok_or(DropCause::UnusableUrl)?;
        let mut url = cand
            .resolve(self.doc_url.as_ref(), reference)
            .ok_or(DropCause::UnusableUrl)?;
        if media && let Some(q) = &rep.query {
            url = append_query(&url, q);
        }
        if !self.dash.cross_origin_urls
            && let Some(doc) = &self.doc_url
        {
            let parsed = Url::parse(&url).map_err(|_| DropCause::UnusableUrl)?;
            if !same_origin(doc, &parsed) {
                return Err(DropCause::CrossOrigin);
            }
        }
        Ok(url)
    }

    fn emit(&mut self, id: ResourceId) -> Option<Action> {
        let (rep_id, reference, range, kind) = {
            let info = self.reqs.get(&id)?;
            (info.rep, info.reference.clone(), info.range, info.kind)
        };
        let url = {
            let rep = self.rep(rep_id)?;
            match self.resolve(rep, &reference, kind == ReqKind::Segment) {
                Ok(u) => u,
                Err(cause) => {
                    let reason = if cause == DropCause::CrossOrigin {
                        "cross_origin"
                    } else {
                        "unusable_url"
                    };
                    self.go_fatal(FatalKind::UnsupportedAddressing { reason });
                    return None;
                }
            }
        };
        self.parked.remove(&id);
        self.inflight.insert(id);
        match kind {
            ReqKind::Init => {
                let key = self.rep(rep_id)?.init_key.clone();
                if let Some(e) = self.inits.get_mut(&key) {
                    e.state = InitState::InFlight;
                }
                Some(Action::FetchInit {
                    id,
                    url,
                    byte_range: range,
                })
            }
            ReqKind::Index => Some(Action::FetchIndex {
                id,
                url,
                byte_range: range,
            }),
            ReqKind::Segment => Some(Action::FetchSegment {
                id,
                url,
                byte_range: range,
            }),
        }
    }

    // Period activation ----------------------------------------------------

    fn activate(&mut self, index: usize) {
        let Some(mpd) = self.mpd.as_ref() else { return };
        let timing = period_timings(mpd).get(index).copied();
        let start_us = timing.and_then(|t| t.start_us).unwrap_or(0);
        let selection = select(mpd, index, &self.cfg.policy);
        let period_id = mpd.periods.get(index).and_then(|p| p.id.clone());
        let mut notices = Vec::new();
        let mut encryption = false;
        for d in &selection.dropped {
            if d.reason == crate::selection::DropReason::UnsupportedEncryption {
                encryption = true;
            }
            notices.push(Output::TrackDropped {
                period: index,
                set: d.set,
                rep: d.rep,
                cause: DropCause::Selection(d.reason),
            });
        }
        if encryption {
            notices.push(Output::Notice(Notice::UnsupportedEncryption));
        }
        if selection
            .dropped
            .iter()
            .any(|d| d.reason == crate::selection::DropReason::UnresolvedXlink)
        {
            notices.push(Output::Notice(Notice::XlinkSkipped));
        }
        let video = selection
            .chosen_variant
            .and_then(|i| selection.variants.get(i).copied());
        let audio = selection.chosen_audio;
        let mut run = PeriodRun {
            index,
            id: period_id,
            start_us,
            lanes: Vec::new(),
            selection,
            notices,
            started: false,
            issue_done: false,
            finished: false,
        };
        let pi = self.periods.len();
        for (kind, at) in [(SetClass::Video, video), (SetClass::Audio, audio)] {
            let Some(at) = at else { continue };
            match self.build_rep(at) {
                Ok(rep) => {
                    let set_id = at
                        .resolve(self.mpd.as_ref().expect("mpd present"))
                        .and_then(|(_, s, _)| s.id);
                    run.lanes.push(Lane {
                        kind,
                        set_id,
                        reps: vec![rep],
                        queue: VecDeque::new(),
                        base: 0,
                        count: 0,
                        announced_rep: None,
                        announced_init: None,
                        expired_run: 0,
                    });
                }
                Err(cause) => run.notices.push(Output::TrackDropped {
                    period: index,
                    set: at.set,
                    rep: Some(at.rep),
                    cause,
                }),
            }
        }
        self.periods.push(run);
        for li in 0..self.periods[pi].lanes.len() {
            self.register_rep(pi, li, 0);
        }
    }

    fn init_reference(
        mpd: &Mpd,
        at: RepRef,
        planned: &std::result::Result<crate::address::PlannedRepresentation, PlanError>,
    ) -> Option<(String, Option<ByteRange>)> {
        match planned {
            Ok(p) => p.plan.init().map(|r| (r.reference, r.byte_range)),
            Err(PlanError::Address(AddressError::IndexRequired)) => {
                let (period, set, rep) = at.resolve(mpd)?;
                match resolve_segment_info(period, set, rep).ok()? {
                    SegmentInfo::Base(b) => b
                        .initialization
                        .as_ref()
                        .map(|u| (u.source_url.clone().unwrap_or_default(), u.range)),
                    SegmentInfo::Template(_) | SegmentInfo::List(_) => None,
                }
            }
            Err(_) => None,
        }
    }

    fn build_rep(&mut self, at: RepRef) -> std::result::Result<RepState, DropCause> {
        let mpd = self.mpd.as_ref().expect("mpd present");
        let (period, set, rep) = at
            .resolve(mpd)
            .ok_or(DropCause::Addressing("no_such_representation"))?;
        let candidates = representation_candidates(mpd, period, set, rep);
        let query = segment_url_query(mpd, period, set, rep, self.mpd_query.as_deref())
            .map_err(|_| DropCause::UnusableUrl)?;
        let planned = plan_representation(mpd, at, &self.plan_options());
        let init = Self::init_reference(mpd, at, &planned).ok_or(DropCause::NoInitialization)?;
        let (plan, index) = match planned {
            Ok(p) => {
                if p.plan.len().is_none() && mpd.kind != PresentationType::Dynamic {
                    return Err(DropCause::UnboundedPlan);
                }
                (Some(p.plan), None)
            }
            Err(PlanError::Address(AddressError::IndexRequired)) => {
                let info = resolve_segment_info(period, set, rep)
                    .map_err(|_| DropCause::Addressing("inherit"))?;
                let range = match info {
                    SegmentInfo::Base(b) => b.index_range,
                    SegmentInfo::Template(_) | SegmentInfo::List(_) => None,
                };
                let range = range.ok_or(DropCause::Addressing("index_required"))?;
                (None, Some(IndexReq { range }))
            }
            Err(e) => return Err(DropCause::Addressing(plan_error_name(&e))),
        };
        let init_range = range_pair(init.1)?;
        let id = RepId(self.next_rep);
        self.next_rep = self.next_rep.saturating_add(1);
        let mut state = RepState {
            id,
            at,
            candidates,
            cand: 0,
            cand_failures: 0,
            query,
            plan,
            index,
            init_key: InitKey {
                url: String::new(),
                range: init_range,
            },
            next_issue: 0,
            start_time_us: None,
            dead: false,
            set_id: set.id,
            set_pos: at.set,
            rep_key: rep.id.clone(),
            joined: mpd.kind != PresentationType::Dynamic,
            last_time: None,
        };
        let init_url = self.resolve(&state, &init.0, false)?;
        state.init_key.url = without_query(&init_url);
        if let Some(ix) = &state.index {
            range_pair(Some(ix.range))?;
            self.resolve(&state, "", false)?;
        }
        if let Some(seg) = state.plan.as_ref().and_then(|p| p.get(0)) {
            range_pair(seg.byte_range)?;
            self.resolve(&state, &seg.reference, true)?;
        }
        Ok(state)
    }

    fn register_rep(&mut self, pi: usize, li: usize, ri: usize) {
        let rep = &self.periods[pi].lanes[li].reps[ri];
        let rep_id = rep.id;
        let key = rep.init_key.clone();
        let has_index = rep.index.is_some();
        let index_range = rep.index.as_ref().map(|i| i.range);
        let init_reference = {
            let mpd = self.mpd.as_ref().expect("mpd present");
            let planned = rep.plan.as_ref().map(|p| p.init());
            match planned {
                Some(init) => init.map(|r| (r.reference, r.byte_range)),
                None => Self::init_reference(
                    mpd,
                    rep.at,
                    &Err(PlanError::Address(AddressError::IndexRequired)),
                ),
            }
        };
        self.rep_loc.insert(rep_id, (pi, li, ri));
        if let std::collections::btree_map::Entry::Vacant(v) = self.inits.entry(key) {
            v.insert(InitEntry {
                state: InitState::Wanted,
                bytes: None,
                specs: Vec::new(),
                skipped_reported: BTreeSet::new(),
            });
            let (reference, range) = init_reference.unwrap_or_default();
            let init_id = ResourceId::Init { rep: rep_id };
            self.reqs.insert(
                init_id,
                ReqInfo {
                    rep: rep_id,
                    reference,
                    range: range.and_then(|r| r.offset_len().ok()),
                    kind: ReqKind::Init,
                    avail_from_us: None,
                    avail_until_us: None,
                },
            );
            self.queue.push_back(init_id);
        }
        if has_index && let Some(r) = index_range {
            let index_id = ResourceId::Index { rep: rep_id };
            self.reqs.insert(
                index_id,
                ReqInfo {
                    rep: rep_id,
                    reference: String::new(),
                    range: r.offset_len().ok(),
                    kind: ReqKind::Index,
                    avail_from_us: None,
                    avail_until_us: None,
                },
            );
            self.queue.push_back(index_id);
        }
    }

    // Deliveries -----------------------------------------------------------

    fn requested(&self, id: ResourceId) -> Result<()> {
        if self.done.contains(&id) {
            return Err(Error::DuplicateResource { id });
        }
        if !self.inflight.contains(&id) {
            return Err(Error::UnrequestedResource { id });
        }
        Ok(())
    }

    fn body_rejection(&self, id: ResourceId, bytes: &[u8]) -> Option<Rejection> {
        if bytes.len() > self.cfg.max_segment_bytes {
            return Some(Rejection::TooLarge);
        }
        let range = self.reqs.get(&id).and_then(|r| r.range)?;
        (u64::try_from(bytes.len()).ok() != Some(range.1)).then_some(Rejection::WrongLength)
    }

    fn max_failures(&self, id: ResourceId) -> u32 {
        match id {
            ResourceId::Segment { .. } => self.cfg.max_segment_failures,
            ResourceId::Init { .. } | ResourceId::Index { .. } => self.cfg.max_init_failures,
        }
    }

    fn count_failure(
        &mut self,
        now_us: u64,
        id: ResourceId,
        kind: FailureKind,
        status: Option<u16>,
    ) -> FailureClass {
        self.inflight.remove(&id);
        let mut rotated = false;
        let count = {
            let n = self.failures.entry(id).or_insert(0);
            *n = n.saturating_add(1);
            *n
        };
        if count >= self.max_failures(id) {
            if self.is_live_mpd() && matches!(id, ResourceId::Segment { .. }) {
                return self.give_up_live_segment(id, status);
            }
            let fatal = match id {
                ResourceId::Segment { .. } => FatalKind::SegmentFetchFailed { id, status },
                ResourceId::Init { .. } | ResourceId::Index { .. } => {
                    FatalKind::InitFetchFailed { id, status }
                }
            };
            self.go_fatal(fatal);
            return FailureClass::GiveUp;
        }
        if let Some(rep) = self.rep_mut(id.rep()) {
            rep.cand_failures += 1;
            if rep.cand_failures >= ROTATE_AFTER_FAILURES && rep.candidates.len() > 1 {
                rep.cand = (rep.cand + 1) % rep.candidates.len();
                rep.cand_failures = 0;
                rotated = true;
            }
        }
        if rotated {
            self.after_rotation(now_us);
        }
        self.parked.insert(id);
        FailureClass::Retry {
            attempt: count,
            kind,
        }
    }

    fn reject(&mut self, now_us: u64, id: ResourceId, reason: Rejection) -> Error {
        let class = self.count_failure(now_us, id, FailureKind::Invalid, None);
        Error::Rejected { id, reason, class }
    }

    fn clear_success(&mut self, id: ResourceId) {
        self.inflight.remove(&id);
        self.parked.remove(&id);
        self.failures.remove(&id);
        self.mark_done(id);
    }

    /// Feed an Initialization Segment.
    ///
    /// # Errors
    /// [`Error::Rejected`] for an unusable body (counted; see
    /// [`FailureClass`]), also for a segment held for this init that the
    /// demuxer then refused; [`Error::UnrequestedResource`],
    /// [`Error::DuplicateResource`], [`Error::Stopped`].
    pub fn on_init(&mut self, now_us: u64, id: ResourceId, bytes: &[u8]) -> Result<()> {
        self.ensure_live()?;
        let ResourceId::Init { rep } = id else {
            return Err(Error::UnrequestedResource { id });
        };
        self.requested(id)?;
        if let Some(r) = self.body_rejection(id, bytes) {
            return Err(self.reject(now_us, id, r));
        }
        if !segment::progressive_movie(bytes) {
            return Err(self.reject(now_us, id, Rejection::InitUndecodable));
        }
        let info = match demux::init_info(bytes) {
            Ok(i) => i,
            Err(_) => return Err(self.reject(now_us, id, Rejection::DemuxFailed)),
        };
        let key = self
            .rep(rep)
            .ok_or(Error::UnrequestedResource { id })?
            .init_key
            .clone();
        if let Some(e) = self.inits.get_mut(&key) {
            e.state = InitState::Ready;
            e.bytes = Some(Arc::new(bytes.to_vec()));
            e.specs = info.specs;
            for s in info.skipped {
                let k = (s.track_id.map(TrackId::value), s.fourcc.clone());
                if e.skipped_reported.insert(k) {
                    self.outputs.push_back(Output::TrackSkipped {
                        track_id: None,
                        fourcc: s.fourcc,
                        reason: s.reason,
                    });
                }
            }
        }
        self.clear_success(id);
        self.reqs.remove(&id);
        let held = self.demux_held(now_us, &key);
        self.drain();
        held
    }

    fn demux_held(&mut self, now_us: u64, key: &InitKey) -> Result<()> {
        let Some(init) = self.inits.get(key).and_then(|e| e.bytes.clone()) else {
            return Ok(());
        };
        let mut first_error = None;
        let from = self.deliver.min(self.periods.len());
        for pi in from..self.periods.len() {
            for li in 0..self.periods[pi].lanes.len() {
                let n = self.periods[pi].lanes[li].queue.len();
                for qi in 0..n {
                    let (id, matches) = {
                        let lane = &self.periods[pi].lanes[li];
                        let slot = &lane.queue[qi];
                        let m = matches!(slot.state, SlotState::Held(_))
                            && lane.reps.get(slot.rep).is_some_and(|r| &r.init_key == key);
                        (slot.id, m)
                    };
                    if !matches {
                        continue;
                    }
                    let slot = &mut self.periods[pi].lanes[li].queue[qi];
                    let SlotState::Held(bytes) =
                        std::mem::replace(&mut slot.state, SlotState::Issued)
                    else {
                        continue;
                    };
                    match demux::demux_segment(&init, &bytes) {
                        Ok(staged) => {
                            self.periods[pi].lanes[li].queue[qi].state = SlotState::Staged(staged);
                        }
                        Err(_) => {
                            if let Some(id) = id {
                                self.inflight.insert(id);
                                let e = self.reject(now_us, id, Rejection::DemuxFailed);
                                first_error.get_or_insert(e);
                            }
                        }
                    }
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Feed the `sidx` of a `SegmentBase` Representation.
    ///
    /// # Errors
    /// As [`DashClient::on_init`].
    pub fn on_index(&mut self, now_us: u64, id: ResourceId, bytes: &[u8]) -> Result<()> {
        self.ensure_live()?;
        let ResourceId::Index { rep } = id else {
            return Err(Error::UnrequestedResource { id });
        };
        self.requested(id)?;
        if let Some(r) = self.body_rejection(id, bytes) {
            return Err(self.reject(now_us, id, r));
        }
        let sidx = match base_index::parse_index(bytes) {
            Ok(s) => s,
            Err(_) => return Err(self.reject(now_us, id, Rejection::BadIndex)),
        };
        let (at, range) = {
            let r = self.rep(rep).ok_or(Error::UnrequestedResource { id })?;
            let range = r.index.as_ref().map(|i| i.range);
            (r.at, range)
        };
        let Some(range) = range else {
            return Err(Error::UnrequestedResource { id });
        };
        let anchor = match base_index::anchor_after(&range) {
            Ok(a) => a,
            Err(_) => return Err(self.reject(now_us, id, Rejection::BadIndex)),
        };
        let planned = {
            let mpd = self.mpd.as_ref().expect("mpd present");
            plan_representation(mpd, at, &self.plan_options().with_index(&sidx, anchor))
        };
        self.clear_success(id);
        self.reqs.remove(&id);
        match planned {
            Ok(p) => {
                if let Some(r) = self.rep_mut(rep) {
                    let start = r.start_time_us.take();
                    r.next_issue = start
                        .and_then(|t| p.plan.index_at_or_before(t))
                        .unwrap_or(0);
                    r.plan = Some(p.plan);
                }
            }
            Err(e) => {
                if let Some(r) = self.rep_mut(rep) {
                    r.dead = true;
                }
                if let Some((pi, _, _)) = self.rep_loc.get(&rep).copied() {
                    self.outputs.push_back(Output::TrackDropped {
                        period: self.periods[pi].index,
                        set: at.set,
                        rep: Some(at.rep),
                        cause: DropCause::Addressing(plan_error_name(&e)),
                    });
                }
            }
        }
        self.drain();
        Ok(())
    }

    /// Feed a Media Segment.
    ///
    /// # Errors
    /// As [`DashClient::on_init`]; a progressive MP4 additionally ends the
    /// presentation.
    pub fn on_segment(&mut self, now_us: u64, id: ResourceId, bytes: &[u8]) -> Result<()> {
        self.ensure_live()?;
        let ResourceId::Segment { rep, .. } = id else {
            return Err(Error::UnrequestedResource { id });
        };
        self.requested(id)?;
        if let Some(r) = self.body_rejection(id, bytes) {
            return Err(self.reject(now_us, id, r));
        }
        if segment::progressive_movie(bytes) {
            self.inflight.remove(&id);
            self.go_fatal(FatalKind::NonFragmentedMp4 { id });
            return Err(Error::Rejected {
                id,
                reason: Rejection::NonFragmentedMp4,
                class: FailureClass::GiveUp,
            });
        }
        if !segment::media_fragment(bytes) {
            return Err(self.reject(now_us, id, Rejection::NotMedia));
        }
        let key = self
            .rep(rep)
            .ok_or(Error::UnrequestedResource { id })?
            .init_key
            .clone();
        let init = self.inits.get(&key).and_then(|e| e.bytes.clone());
        let state = match init {
            Some(init) => match demux::demux_segment(&init, bytes) {
                Ok(staged) => SlotState::Staged(staged),
                Err(_) => return Err(self.reject(now_us, id, Rejection::DemuxFailed)),
            },
            None => SlotState::Held(bytes.to_vec()),
        };
        let Some(slot) = self.slot_mut(id) else {
            return Err(Error::UnrequestedResource { id });
        };
        slot.state = state;
        self.inflight.remove(&id);
        self.parked.remove(&id);
        self.failures.remove(&id);
        self.mark_done(id);
        self.segment_delivered(id);
        self.drain();
        Ok(())
    }

    fn slot_mut(&mut self, id: ResourceId) -> Option<&mut Slot> {
        let (p, l, _) = *self.rep_loc.get(&id.rep())?;
        self.periods
            .get_mut(p)?
            .lanes
            .get_mut(l)?
            .queue
            .iter_mut()
            .find(|s| s.id == Some(id))
    }

    /// Report a failed fetch.
    ///
    /// # Errors
    /// [`Error::UnrequestedResource`] when `id` is not outstanding;
    /// [`Error::Stopped`].
    pub fn on_failure(
        &mut self,
        now_us: u64,
        id: ResourceId,
        failure: Failure,
    ) -> Result<FailureClass> {
        self.ensure_live()?;
        if !self.inflight.contains(&id) {
            return Err(Error::UnrequestedResource { id });
        }
        if self.is_live_mpd() && matches!(id, ResourceId::Segment { .. }) {
            return self.live_segment_failure(now_us, id, failure);
        }
        let status = match failure {
            Failure::Status(s) => Some(s),
            Failure::Transport | Failure::Timeout | Failure::TooLarge => None,
        };
        if matches!(status, Some(STATUS_NOT_FOUND | STATUS_GONE)) && self.is_missing_tail(id) {
            self.inflight.remove(&id);
            self.failures.remove(&id);
            self.mark_done(id);
            if let Some(slot) = self.slot_mut(id) {
                slot.state = SlotState::Skipped { marker: false };
            }
            self.drain();
            return Ok(FailureClass::EndTail);
        }
        Ok(self.count_failure(now_us, id, live::failure_kind(failure), status))
    }

    fn is_missing_tail(&self, id: ResourceId) -> bool {
        let ResourceId::Segment { rep, .. } = id else {
            return false;
        };
        let Some((pi, li, _)) = self.rep_loc.get(&rep).copied() else {
            return false;
        };
        let lane = &self.periods[pi].lanes[li];
        let Some(slot) = lane.queue.iter().find(|s| s.id == Some(id)) else {
            return false;
        };
        let Some(r) = lane.reps.get(slot.rep) else {
            return false;
        };
        let Some(plan) = r.plan.as_ref() else {
            return false;
        };
        let last = plan.len().and_then(|n| n.checked_sub(1));
        last == Some(slot.plan_index)
            && plan.len().is_some_and(|n| n > 1)
            && plan
                .get(slot.plan_index)
                .is_some_and(|s| s.duration_us < MISSING_TAIL_MAX_US)
    }

    /// Request again a resource that [`DashClient::on_failure`] or a
    /// rejection classified [`FailureClass::Retry`], after the caller's
    /// backoff. `false` when `id` is not waiting for a retry.
    pub fn retry(&mut self, _now_us: u64, id: ResourceId) -> bool {
        if self.stopped() || !self.parked.remove(&id) {
            return false;
        }
        self.queue.push_front(id);
        true
    }

    /// Give up on one segment: it counts as delivered and a
    /// [`Output::Discontinuity`] marks the gap. `false` when `id` is not a
    /// segment waiting for a retry.
    pub fn skip(&mut self, _now_us: u64, id: ResourceId) -> bool {
        if self.stopped() || !matches!(id, ResourceId::Segment { .. }) || !self.parked.remove(&id) {
            return false;
        }
        self.failures.remove(&id);
        self.mark_done(id);
        if let Some(slot) = self.slot_mut(id) {
            slot.state = SlotState::Skipped { marker: true };
        }
        self.drain();
        true
    }

    // Delivery -------------------------------------------------------------

    fn drain(&mut self) {
        loop {
            if self.phase != Phase::Running {
                return;
            }
            let Some(pi) = (self.deliver < self.periods.len()).then_some(self.deliver) else {
                return;
            };
            if !self.periods[pi].started {
                if !self.ready_to_start(pi) {
                    return;
                }
                self.start_period(pi);
            }
            self.deliver_slots(pi);
            let p = &self.periods[pi];
            let complete = p.issue_done && p.lanes.iter().all(|l| l.queue.is_empty());
            if !complete {
                return;
            }
            self.periods[pi].finished = true;
            self.periods[pi].lanes.clear();
            let total = self.mpd.as_ref().map_or(0, |m| m.periods.len());
            let last_of_mpd = self.periods[pi].index + 1 >= total;
            self.deliver += 1;
            self.prune_finished();
            if last_of_mpd {
                if self.live_open() {
                    return;
                }
                if self.any_started {
                    self.outputs.push_back(Output::EndOfStream);
                    self.phase = Phase::Ended;
                } else {
                    self.go_fatal(FatalKind::MpdInvalid {
                        reason: "no_playable_representation",
                    });
                }
                return;
            }
        }
    }

    fn prune_finished(&mut self) {
        while self.deliver > 0 && self.issue > 0 && self.periods.first().is_some_and(|p| p.finished)
        {
            self.periods.remove(0);
            self.deliver -= 1;
            self.issue -= 1;
            self.rep_loc.retain(|_, loc| loc.0 != 0);
            for loc in self.rep_loc.values_mut() {
                loc.0 -= 1;
            }
        }
        let mut keep: BTreeSet<InitKey> = self
            .periods
            .iter()
            .flat_map(|p| p.lanes.iter())
            .flat_map(|l| l.reps.iter())
            .map(|r| r.init_key.clone())
            .collect();
        if let Some(prev) = &self.prev {
            keep.extend(prev.lanes.iter().map(|(_, _, k)| k.clone()));
        }
        self.inits.retain(|k, _| keep.contains(k));
    }

    fn ready_to_start(&self, pi: usize) -> bool {
        !self.live_blocked()
            && self.periods[pi].lanes.iter().all(|lane| {
                lane.current().is_none_or(|rep| {
                    self.inits
                        .get(&rep.init_key)
                        .is_some_and(|e| e.state == InitState::Ready)
                })
            })
    }

    fn start_period(&mut self, pi: usize) {
        self.periods[pi].started = true;
        let mut base = 1u32;
        for li in 0..self.periods[pi].lanes.len() {
            let n = self.periods[pi].lanes[li]
                .current()
                .and_then(|r| self.inits.get(&r.init_key))
                .map_or(0, |e| u32::try_from(e.specs.len()).unwrap_or(u32::MAX));
            let lane = &mut self.periods[pi].lanes[li];
            lane.base = base;
            lane.count = n;
            base = base.saturating_add(n);
        }
        let notices = std::mem::take(&mut self.periods[pi].notices);
        self.outputs.extend(notices);
        let live: Vec<usize> = (0..self.periods[pi].lanes.len())
            .filter(|&li| self.periods[pi].lanes[li].current().is_some())
            .collect();
        if live.is_empty() {
            self.periods[pi].issue_done = true;
            return;
        }
        let first = !self.any_started;
        self.any_started = true;
        let continuous = !first && self.is_continuous(pi);
        let manifest = self.manifest(pi);
        let p = &self.periods[pi];
        self.outputs.push_back(Output::Manifest(manifest));
        self.outputs.push_back(Output::PeriodChanged {
            index: p.index,
            id: p.id.clone(),
            start_us: p.start_us,
            continuous,
        });
        if !first && !continuous {
            self.discontinuity();
        }
        let mut summary = Vec::new();
        for li in live {
            let lane = &mut self.periods[pi].lanes[li];
            let ri = lane.reps.len() - 1;
            let key = lane.reps[ri].init_key.clone();
            summary.push((lane.kind, lane.set_id, key.clone()));
            if !continuous && let Some(bytes) = self.inits.get(&key).and_then(|e| e.bytes.as_ref())
            {
                self.outputs.push_back(Output::Init(bytes.as_ref().clone()));
            }
            lane.announced_init = Some(key);
            lane.announced_rep = Some(ri);
        }
        self.announce_tracks(pi);
        self.prev = Some(PrevPeriod {
            id: self.periods[pi].id.clone(),
            lanes: summary,
        });
    }

    fn is_continuous(&self, pi: usize) -> bool {
        let Some(prev) = &self.prev else { return false };
        let Some(prev_id) = prev.id.as_deref() else {
            return false;
        };
        let Some(mpd) = self.mpd.as_ref() else {
            return false;
        };
        let p = &self.periods[pi];
        let live: Vec<&Lane> = p.lanes.iter().filter(|l| l.current().is_some()).collect();
        if live.len() != prev.lanes.len() {
            return false;
        }
        live.iter()
            .zip(&prev.lanes)
            .all(|(lane, (kind, set_id, key))| {
                let Some(rep) = lane.current() else {
                    return false;
                };
                let Some((_, set, _)) = rep.at.resolve(mpd) else {
                    return false;
                };
                lane.kind == *kind
                    && lane.set_id.is_some()
                    && lane.set_id == *set_id
                    && &rep.init_key == key
                    && continuity_signals(set, prev_id)
            })
    }

    fn announce_tracks(&mut self, pi: usize) {
        let mut specs: Vec<TrackSpec> = Vec::new();
        for lane in &self.periods[pi].lanes {
            let Some(rep) = lane.current() else { continue };
            let Some(entry) = self.inits.get(&rep.init_key) else {
                continue;
            };
            for (pos, spec) in entry.specs.iter().enumerate() {
                let pos = u32::try_from(pos).unwrap_or(u32::MAX);
                if pos >= lane.count {
                    break;
                }
                let mut s = spec.clone();
                s.track_id = TrackId::new(lane.base.saturating_add(pos));
                specs.push(s);
            }
        }
        let fingerprint = format!("{specs:?}");
        if self.announced_tracks.as_deref() != Some(fingerprint.as_str()) {
            self.announced_tracks = Some(fingerprint);
            self.outputs.push_back(Output::Tracks(specs));
        }
    }

    fn deliver_slots(&mut self, pi: usize) {
        for li in 0..self.periods[pi].lanes.len() {
            while self.periods[pi].lanes[li]
                .queue
                .front()
                .is_some_and(|f| !matches!(f.state, SlotState::Issued | SlotState::Held(_)))
            {
                let Some(slot) = self.periods[pi].lanes[li].queue.pop_front() else {
                    break;
                };
                if !matches!(slot.state, SlotState::Marker) {
                    self.outstanding = self.outstanding.saturating_sub(1);
                }
                if let Some(id) = slot.id {
                    self.reqs.remove(&id);
                }
                match slot.state {
                    SlotState::Marker => self.discontinuity(),
                    SlotState::Skipped { marker } => {
                        if marker {
                            self.discontinuity();
                        }
                    }
                    SlotState::Staged(staged) => {
                        self.emit_staged(pi, li, slot.rep, slot.id, staged);
                    }
                    SlotState::Issued | SlotState::Held(_) => {}
                }
            }
        }
    }

    fn emit_staged(
        &mut self,
        pi: usize,
        li: usize,
        rep_idx: usize,
        id: Option<ResourceId>,
        staged: Staged,
    ) {
        if self.periods[pi].lanes[li].announced_rep != Some(rep_idx) {
            let key = self.periods[pi].lanes[li].reps[rep_idx].init_key.clone();
            if self.periods[pi].lanes[li].announced_init.as_ref() != Some(&key) {
                if let Some(bytes) = self.inits.get(&key).and_then(|e| e.bytes.as_ref()) {
                    self.outputs.push_back(Output::Init(bytes.as_ref().clone()));
                }
                self.periods[pi].lanes[li].announced_init = Some(key);
            }
            self.periods[pi].lanes[li].announced_rep = Some(rep_idx);
            self.announce_tracks(pi);
        }
        let lane = &self.periods[pi].lanes[li];
        let key = &lane.reps[rep_idx].init_key;
        let locals: Vec<u32> = self
            .inits
            .get(key)
            .map(|e| e.specs.iter().map(|s| s.track_id.value()).collect())
            .unwrap_or_default();
        let (base, count) = (lane.base, lane.count);
        let map = |local: u32| -> Option<TrackId> {
            let pos = locals.iter().position(|l| *l == local)?;
            let pos = u32::try_from(pos).ok()?;
            (pos < count).then(|| TrackId::new(base.saturating_add(pos)))
        };
        if staged.damaged > 0
            && let Some(id) = id
        {
            self.outputs.push_back(Output::FragmentDamaged {
                id,
                skipped: staged.damaged,
            });
        }
        for s in staged.skipped {
            let k = (s.track_id.map(TrackId::value), s.fourcc.clone());
            let first = self
                .inits
                .get_mut(&self.periods[pi].lanes[li].reps[rep_idx].init_key.clone())
                .is_some_and(|e| e.skipped_reported.insert(k));
            if first {
                self.outputs.push_back(Output::TrackSkipped {
                    track_id: s.track_id.and_then(|t| map(t.value())),
                    fourcc: s.fourcc,
                    reason: s.reason,
                });
            }
        }
        for (local, samples) in staged.tracks {
            if samples.is_empty() {
                continue;
            }
            if let Some(track_id) = map(local) {
                self.last_was_discontinuity = false;
                self.outputs
                    .push_back(Output::Samples { track_id, samples });
            }
        }
    }

    // Manifest and switching ----------------------------------------------

    fn init_url_of(&self, at: RepRef) -> Option<String> {
        let mpd = self.mpd.as_ref()?;
        let planned = plan_representation(mpd, at, &PlanOptions::default());
        let (reference, _) = Self::init_reference(mpd, at, &planned)?;
        let (period, set, rep) = at.resolve(mpd)?;
        let cands = representation_candidates(mpd, period, set, rep);
        cands.first()?.resolve(self.doc_url.as_ref(), &reference)
    }

    fn manifest(&self, pi: usize) -> ManifestSelection {
        let mpd = self.mpd.as_ref().expect("mpd present");
        let p = &self.periods[pi];
        let period = &mpd.periods[p.index];
        let audio_group = p
            .selection
            .chosen_audio
            .and_then(|a| period.adaptation_sets.get(a.set))
            .and_then(|s| s.group.or(s.id))
            .map(|g| g.to_string());
        let variants = p
            .selection
            .variants
            .iter()
            .filter_map(|&at| {
                let (_, set, rep) = at.resolve(mpd)?;
                let common = effective_common(set, rep);
                Some(VariantInfo {
                    rep: at,
                    init_url: self.init_url_of(at),
                    bandwidth: rep.bandwidth,
                    resolution: common.width.zip(common.height),
                    codecs: common.codecs,
                    audio_group: audio_group.clone(),
                })
            })
            .collect();
        let audio: Vec<AudioRendition> = p
            .selection
            .audio_sets
            .iter()
            .filter_map(|a| {
                let set = period.adaptation_sets.get(a.set)?;
                let first = a.reps.first()?;
                let (_, _, first_rep) = first.resolve(mpd)?;
                let chosen = p
                    .selection
                    .chosen_audio
                    .filter(|c| c.set == a.set)
                    .unwrap_or(*first);
                Some(AudioRendition {
                    set: a.set,
                    group: set.group.or(set.id).map(|g| g.to_string()),
                    name: set
                        .labels
                        .first()
                        .cloned()
                        .or_else(|| set.lang.clone())
                        .unwrap_or_else(|| first_rep.id.clone()),
                    language: set.lang.clone(),
                    default: a.is_main,
                    autoselect: true,
                    init_url: self.init_url_of(chosen),
                    reps: a.reps.clone(),
                })
            })
            .collect();
        let chosen_audio = p
            .selection
            .chosen_audio
            .and_then(|c| p.selection.audio_sets.iter().position(|a| a.set == c.set));
        ManifestSelection {
            period: p.index,
            variants,
            chosen: p.selection.chosen_variant,
            audio,
            chosen_audio,
        }
    }

    /// Switch the video lane to variant `index` of the latest
    /// [`Output::Manifest`]. Segments already requested keep their
    /// Representation; the new one continues from the segment containing the
    /// next unrequested media time, its init is fetched, and the track set is
    /// re-announced (DESIGN 5.6). A switch that is not segment-aligned
    /// (`@segmentAlignment` with `@startWithSAP` 1 or 2 on the new set) is
    /// marked with a [`Output::Discontinuity`].
    ///
    /// # Errors
    /// [`Error::NoSuchVariant`]; [`Error::NotSwitchable`] when the Period
    /// being played is not the one being fetched, there is no video lane, or
    /// nothing is left to fetch; [`Error::Stopped`].
    pub fn select_representation(&mut self, index: usize) -> Result<()> {
        self.ensure_live()?;
        let pi = self.deliver;
        if pi != self.issue || self.periods.get(pi).is_none_or(|p| !p.started) {
            return Err(Error::NotSwitchable);
        }
        let at = *self.periods[pi]
            .selection
            .variants
            .get(index)
            .ok_or(Error::NoSuchVariant)?;
        let li = self.periods[pi]
            .lanes
            .iter()
            .position(|l| l.kind == SetClass::Video && l.current().is_some())
            .ok_or(Error::NotSwitchable)?;
        let (current_at, switch_us) = {
            let lane = &self.periods[pi].lanes[li];
            let cur = lane.current().ok_or(Error::NotSwitchable)?;
            let plan = cur.plan.as_ref().ok_or(Error::NotSwitchable)?;
            let at_us = match plan.get(cur.next_issue) {
                Some(seg) => seg.start_us,
                None => plan
                    .len()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|last| plan.get(last))
                    .map(|seg| seg.start_us.saturating_add(seg.duration_us))
                    .ok_or(Error::NotSwitchable)?,
            };
            (cur.at, at_us)
        };
        if current_at == at {
            return Ok(());
        }
        let aligned = {
            let mpd = self.mpd.as_ref().expect("mpd present");
            at.resolve(mpd).is_some_and(|(_, set, _)| {
                set.segment_alignment
                    && matches!(
                        set.start_with_sap,
                        Some(START_WITH_SAP_1 | START_WITH_SAP_2)
                    )
            })
        };
        let mut rep = match self.build_rep(at) {
            Ok(r) => r,
            Err(cause) => {
                self.outputs.push_back(Output::TrackDropped {
                    period: self.periods[pi].index,
                    set: at.set,
                    rep: Some(at.rep),
                    cause,
                });
                return Err(Error::NotSwitchable);
            }
        };
        match rep.plan.as_ref() {
            Some(plan) => {
                rep.next_issue = plan.index_at_or_before(switch_us).unwrap_or(0);
                rep.joined = true;
                rep.last_time = rep
                    .next_issue
                    .checked_sub(1)
                    .and_then(|i| plan.get(i))
                    .map(|s| s.time_ticks);
            }
            None => rep.start_time_us = Some(switch_us),
        }
        if !aligned {
            self.periods[pi].lanes[li].queue.push_back(Slot {
                id: None,
                rep: 0,
                plan_index: 0,
                state: SlotState::Marker,
            });
        }
        let ri = self.periods[pi].lanes[li].reps.len();
        self.periods[pi].lanes[li].reps.push(rep);
        self.periods[pi].selection.chosen_variant = Some(index);
        self.periods[pi].issue_done = false;
        self.register_rep(pi, li, ri);
        Ok(())
    }
}
