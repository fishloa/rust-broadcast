//! The live half of [`DashClient`]: the clock ladder, `UTCTiming` sampling,
//! MPD refresh, the join near the live edge, availability gating and the
//! classification of early, expired and missing segments.
//!
//! **Spec:** ISO/IEC 23009-1:2022 5.3.9.5.3 (availability), 5.4 (MPD
//! updates and Reset), 5.8.5.7 (`UTCTiming`), Annex A.2, A.7, A.11;
//! DASH-IF IOP v4.3 4.3.3, 4.4.4, 4.7, 4.8; RFC 8470. Verbatim text:
//! `private/specs-verified/dash/23009-1/` and `dashif-iop-4.3/`. Design:
//! `docs/dash-runtime/DESIGN.md` sections 3, 5.2-5.4 and 12.

use std::collections::{BTreeMap, BTreeSet};

use url::Url;

use super::*;
use crate::client::action::{MpdValidators, UtcId};
use crate::client::config::DashClockPolicy;
use crate::client::meta::UtcResponse;
use crate::client::utc_sampler::{Step, UtcSampler};
use crate::timing::clock::{AdoptReason, Anchor, ClockSource, OfferOutcome, OfferPolicy};
use crate::timing::live_edge::{join_index, oldest_in_window, presentation_delay};
use crate::timing::period::duration_micros;
use crate::timing::utc_timing::{Exchange, UtcSample, sample_from_date_header};
use core::time::Duration;

use broadcast_common::stage::Timestamp;
use broadcast_dash::descriptors::SCHEME_RESET;

/// A forced second look at the UTC clock after a segment error is taken at
/// most this often (23009-1 A.7: check the clock first, then the MPD).
const FORCED_RESYNC_MIN_GAP: core::time::Duration = core::time::Duration::from_secs(30);
/// Cap of the early-request cadence (DESIGN 5.4: 250 ms doubling to 1 s).
const EARLY_RETRY_CAP: Duration = Duration::from_secs(1);
/// Expiries in a row on one lane before the client rejoins the live edge.
const EXPIRED_REJOIN_RUN: u32 = 3;
/// A closed Period's lists are final this many segment durations (or one
/// `minimumUpdatePeriod`, if longer) after its end (23009-1 5.4.1).
const PERIOD_FINAL_SEGMENTS: i128 = 2;
/// Bound on the steps one lane evaluation may take.
const MAX_LANE_STEPS: usize = 8;
/// The early-retry doubling stops growing here (the cap applies first).
const EARLY_SHIFT_MAX: u32 = 20;

#[derive(Default)]
pub(super) struct Refresh {
    pub(super) next_at: Option<Timestamp>,
    pub(super) forced_at: Option<Timestamp>,
    pub(super) last_request: Option<Timestamp>,
    pub(super) publish_time: Option<jiff::Timestamp>,
    pub(super) body: Vec<u8>,
    pub(super) validators: MpdValidators,
    pub(super) redirect_url: Option<String>,
    pub(super) awaiting_retry: bool,
}

#[derive(Default, Clone, Copy)]
pub(super) struct Early {
    attempts: u32,
    retry_at: Option<Timestamp>,
}

pub(super) struct Live {
    pub(super) ready: bool,
    sampler: Option<UtcSampler>,
    next_resync: Option<Timestamp>,
    last_forced_resync: Option<Timestamp>,
    pub(super) refresh: Refresh,
    pub(super) early: BTreeMap<ResourceId, Early>,
    missing_seen: BTreeSet<RepId>,
    announced_clock: Option<ClockSource>,
    wake_utc: Option<i128>,
    mpd_exchange: Exchange,
    fetch_utc_us: Option<i128>,
}

impl Live {
    pub(super) fn bookkeeping(&self) -> usize {
        self.early.len() + self.missing_seen.len()
    }

    fn new(now: Timestamp) -> Self {
        Self {
            ready: false,
            sampler: None,
            next_resync: None,
            last_forced_resync: None,
            refresh: Refresh::default(),
            early: BTreeMap::new(),
            missing_seen: BTreeSet::new(),
            announced_clock: None,
            wake_utc: None,
            mpd_exchange: Exchange::new(now, now),
            fetch_utc_us: None,
        }
    }
}

const STATUS_NOT_FOUND: http::StatusCode = http::StatusCode::NOT_FOUND;
const STATUS_GONE: http::StatusCode = http::StatusCode::GONE;
const STATUS_TOO_EARLY: http::StatusCode = http::StatusCode::TOO_EARLY;

pub(super) fn status_of(failure: Failure) -> Option<http::StatusCode> {
    match failure {
        Failure::Status(s) => http::StatusCode::from_u16(s).ok(),
        Failure::Transport | Failure::Timeout | Failure::TooLarge => None,
    }
}

pub(super) fn failure_kind(failure: Failure) -> FailureKind {
    match failure {
        Failure::Status(s) if s == STATUS_NOT_FOUND.as_u16() || s == STATUS_GONE.as_u16() => {
            FailureKind::Missing
        }
        Failure::Status(s) if s == STATUS_TOO_EARLY.as_u16() => FailureKind::TooEarly,
        Failure::Status(_) => FailureKind::Server,
        Failure::Transport | Failure::Timeout => FailureKind::Transport,
        Failure::TooLarge => FailureKind::Invalid,
    }
}

fn i128_from_i64(v: i64) -> i128 {
    i128::from(v)
}

fn timestamp_from_micros_i128(us: i128) -> Option<jiff::Timestamp> {
    let us = i64::try_from(us.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).ok()?;
    jiff::Timestamp::from_microsecond(us).ok()
}

fn micros(d: core::time::Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

fn span_micros(d: core::time::Duration) -> i128 {
    i128::from(micros(d))
}

fn resync_deadline(resync: Option<core::time::Duration>, now: Timestamp) -> Option<Timestamp> {
    now.checked_add(resync.unwrap_or(core::time::Duration::MAX))
}

impl DashClient {
    // Public surface ---------------------------------------------------------

    /// At the monotonic instant `now` the host's UTC clock read `unix`
    /// (POSIX, no leap seconds). Both reads must be taken back to back. Callable at any time and repeatedly
    /// (at open, every ten minutes, on a system clock-change notification):
    /// a dynamic presentation uses it when no `UTCTiming` answer is available
    /// (or first, with [`DashClockPolicy::PreferHost`]); a static one ignores
    /// it.
    pub fn set_wall_clock(&mut self, now: Timestamp, unix: jiff::Timestamp) {
        if self.stopped() {
            return;
        }
        let sample = UtcSample {
            mono: now,
            utc: unix,
            uncertainty: self.dash.host_clock_uncertainty,
        };
        let allow_downgrade = self.dash.clock_policy == DashClockPolicy::PreferHost;
        let outcome = self.clock.offer(
            sample,
            ClockSource::Host,
            now,
            OfferPolicy::new_or_never(self.dash.utc_resync, allow_downgrade),
        );
        let stepped = matches!(outcome, OfferOutcome::Adopted(AdoptReason::Step));
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if allow_downgrade && !live.ready {
            let gating = live.sampler.as_ref().is_some_and(|s| s.gating);
            if gating {
                live.sampler = None;
                self.finish_ladder(now);
            }
        }
        if stepped {
            self.force_refresh(now);
        }
        self.announce_clock();
    }

    /// The current clock anchor of a dynamic presentation.
    #[must_use]
    pub fn clock_anchor(&self) -> Option<Anchor> {
        self.clock.anchor().copied()
    }

    /// UTC at the monotonic instant `now`, once an anchor exists.
    #[must_use]
    pub fn utc_at(&self, now: Timestamp) -> Option<jiff::Timestamp> {
        self.clock.utc_at(now)
    }

    /// The earliest monotonic instant at which [`DashClient::poll`] may have
    /// something to do although no response arrived: a refresh due, a
    /// segment becoming available, an early retry, a `UTCTiming` timeout or
    /// resync. `None` when only a response (or a delivery) can move the client.
    #[must_use]
    pub fn next_wake(&self) -> Option<Timestamp> {
        let live = self.live.as_ref()?;
        if self.stopped() {
            return None;
        }
        let mut best: Option<Timestamp> = None;
        let mut take = |t: Timestamp| best = Some(best.map_or(t, |b| b.min(t)));
        if let Some(t) = self.refresh_due_at() {
            take(t);
        }
        if let Some(s) = live.sampler.as_ref() {
            if let Some(d) = s.deadline() {
                take(d);
            }
            if let Some(w) = s.wait_deadline.filter(|_| s.gating) {
                take(w);
            }
        } else if live.ready
            && let Some(t) = live.next_resync
        {
            take(t);
        }
        for e in live.early.values() {
            if let Some(t) = e.retry_at {
                take(t);
            }
        }
        if let Some(w) = live.wake_utc
            && let Some(ts) = timestamp_from_micros_i128(w)
            && let Some(m) = self.clock.mono_for_utc(ts)
        {
            take(m);
        }
        best
    }

    /// Report a `UTCTiming` answer.
    ///
    /// # Errors
    /// [`Error::UnrequestedUtc`] when `id` is not the request awaited;
    /// [`Error::Stopped`].
    pub fn on_utc_response(
        &mut self,
        now: Timestamp,
        id: UtcId,
        response: &UtcResponse<'_>,
    ) -> Result<()> {
        self.ensure_live()?;
        let dash = self.dash;
        let sample = {
            let sampler = self
                .live
                .as_mut()
                .and_then(|l| l.sampler.as_mut())
                .filter(|s| s.awaits(id))
                .ok_or(Error::UnrequestedUtc { id })?;
            sampler.on_response(id, response, &dash)
        };
        if let Some(sample) = sample {
            self.utc_sample(now, sample);
        }
        Ok(())
    }

    /// Report a failed `UTCTiming` request (the client abandons it by itself
    /// after `DashConfig::utc_timeout`; a late answer then returns
    /// [`Error::UnrequestedUtc`], which the caller ignores).
    ///
    /// # Errors
    /// [`Error::UnrequestedUtc`] when `id` is not the request awaited;
    /// [`Error::Stopped`].
    pub fn on_utc_failure(&mut self, _now: Timestamp, id: UtcId, failure: Failure) -> Result<()> {
        self.ensure_live()?;
        let sampler = self
            .live
            .as_mut()
            .and_then(|l| l.sampler.as_mut())
            .filter(|s| s.awaits(id))
            .ok_or(Error::UnrequestedUtc { id })?;
        sampler.on_failure(id, failure);
        Ok(())
    }

    /// The MPD request was answered `304`: the held MPD is current. The
    /// exchange moves `FetchTime` forward (23009-1 A.2).
    ///
    /// # Errors
    /// [`Error::MpdNotAwaited`]; [`Error::Stopped`].
    pub fn on_mpd_unchanged(&mut self, now: Timestamp, meta: &ResponseMeta<'_>) -> Result<()> {
        self.ensure_live()?;
        if !self.mpd_inflight || self.live.is_none() {
            return Err(Error::MpdNotAwaited);
        }
        self.mpd_inflight = false;
        self.note_exchange(now, meta);
        self.schedule_refresh(now);
        Ok(())
    }

    // Predicates --------------------------------------------------------------

    pub(super) fn is_live_mpd(&self) -> bool {
        self.mpd
            .as_ref()
            .is_some_and(|m| m.kind == PresentationType::Dynamic)
    }

    pub(super) fn live_blocked(&self) -> bool {
        self.is_live_mpd() && self.live.as_ref().is_none_or(|l| !l.ready)
    }

    pub(super) fn live_open(&self) -> bool {
        self.mpd.as_ref().is_some_and(|m| {
            m.kind == PresentationType::Dynamic
                && m.minimum_update_period.is_some()
                && m.media_presentation_duration.is_none()
        })
    }

    /// Whether more segments may still be added to Period `pi` by a refresh:
    /// its end is unknown, or it is closed but its lists do not yet reach the
    /// end and the grace after which 5.4.1 makes them final has not passed.
    pub(super) fn period_open(&self, pi: usize, now: Timestamp) -> bool {
        if !self.live_open() {
            return false;
        }
        let Some(mpd) = self.mpd.as_ref() else {
            return false;
        };
        let run = &self.periods[pi];
        let Some(end_us) = period_timings(mpd)
            .get(run.index)
            .and_then(|t| t.end.map(duration_micros))
        else {
            return true;
        };
        let len = end_us.saturating_sub(run.start_us);
        let reaches = run.lanes.iter().filter_map(Lane::current).all(|r| {
            r.plan.as_ref().is_some_and(|p| {
                p.len()
                    .and_then(|n| n.checked_sub(1))
                    .map_or(Ok(None), |last| p.get(last))
                    .ok()
                    .flatten()
                    .is_some_and(|s| {
                        s.start
                            .as_micros()
                            .saturating_add(duration_micros(s.duration))
                            .saturating_add(duration_micros(MISSING_TAIL_MAX))
                            >= len
                    })
            })
        });
        if reaches {
            return false;
        }
        let Some(utc) = self.utc_now(now) else {
            return false;
        };
        let ast = mpd
            .availability_start_time
            .map_or(0, |t| i128_from_i64(t.as_microsecond()));
        let mup = mpd.minimum_update_period.map_or(0, duration_micros);
        let seg = mpd.max_segment_duration.map_or(0, duration_micros);
        let grace = mup.max(seg.saturating_mul(PERIOD_FINAL_SEGMENTS));
        utc < ast.saturating_add(end_us).saturating_add(grace)
    }

    pub(super) fn can_activate(&self, next: usize) -> bool {
        let Some(mpd) = self.mpd.as_ref() else {
            return false;
        };
        if next >= mpd.periods.len() {
            return false;
        }
        mpd.kind == PresentationType::Static
            || period_timings(mpd)
                .get(next)
                .is_some_and(|t| t.start.is_some())
    }

    pub(super) fn utc_now(&self, now: Timestamp) -> Option<i128> {
        self.clock
            .utc_at(now)
            .map(|t| i128_from_i64(t.as_microsecond()))
    }

    pub(super) fn plan_options(&self) -> PlanOptions<'static> {
        let mut o = PlanOptions::default()
            .with_availability_time_offset(self.dash.use_availability_time_offset);
        if let Some(h) = self.horizon() {
            o = o.with_horizon(h);
        }
        o
    }

    fn horizon(&self) -> Option<jiff::Timestamp> {
        let mpd = self.mpd.as_ref()?;
        let mup = duration_micros(mpd.minimum_update_period?);
        let fetch = self.live.as_ref()?.fetch_utc_us?;
        timestamp_from_micros_i128(fetch.saturating_add(mup))
    }

    // Wake bookkeeping --------------------------------------------------------

    pub(super) fn reset_wake(&mut self) {
        if let Some(l) = self.live.as_mut() {
            l.wake_utc = None;
        }
    }

    pub(super) fn note_wake(&mut self, at_utc_us: i128) {
        if let Some(l) = self.live.as_mut() {
            l.wake_utc = Some(l.wake_utc.map_or(at_utc_us, |w| w.min(at_utc_us)));
        }
    }

    pub(super) fn note_exhausted(&mut self, now: Timestamp) {
        if !self.live_open() {
            return;
        }
        let mut failed: Vec<(usize, DropCause)> = Vec::new();
        let tail = self
            .periods
            .get(self.issue)
            .map(|p| {
                p.lanes
                    .iter()
                    .enumerate()
                    .filter_map(|(li, l)| {
                        let r = l.current()?;
                        let plan = r.plan.as_ref()?;
                        let last = plan.len()?.checked_sub(1)?;
                        match plan.get(last) {
                            Ok(s) => s.map(|s| s.duration),
                            Err(e) => {
                                failed.push((li, DropCause::Addressing(address_name(&e))));
                                None
                            }
                        }
                    })
                    .max()
                    .unwrap_or(Duration::ZERO)
            })
            .unwrap_or(Duration::ZERO);
        for (li, cause) in failed {
            self.drop_rep(self.issue, li, cause);
        }
        self.force_refresh_after(now, tail);
    }

    // Timers ------------------------------------------------------------------

    fn refresh_due_at(&self) -> Option<Timestamp> {
        let live = self.live.as_ref()?;
        if !self.live_open() || self.mpd_inflight || self.want_mpd || live.refresh.awaiting_retry {
            return None;
        }
        match (live.refresh.next_at, live.refresh.forced_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub(super) fn tick(&mut self, now: Timestamp) {
        if self.live.is_none() {
            return;
        }
        let gate_expired = self
            .live
            .as_ref()
            .and_then(|l| l.sampler.as_ref())
            .is_some_and(|s| s.gating && s.wait_deadline.is_some_and(|d| now >= d));
        if gate_expired {
            if let Some(l) = self.live.as_mut() {
                l.sampler = None;
                l.next_resync = resync_deadline(self.dash.utc_resync, now);
            }
            self.finish_ladder(now);
            if self.stopped() {
                return;
            }
        }
        let resync_due = self.live.as_ref().is_some_and(|l| {
            l.ready && l.sampler.is_none() && l.next_resync.is_some_and(|t| now >= t)
        });
        if resync_due {
            if let Some(l) = self.live.as_mut() {
                l.next_resync = None;
            }
            self.start_sampler(now, false);
        }
        let mut due: Vec<ResourceId> = Vec::new();
        if let Some(l) = self.live.as_mut() {
            for (id, e) in &mut l.early {
                if e.retry_at.is_some_and(|t| now >= t) {
                    e.retry_at = None;
                    due.push(*id);
                }
            }
        }
        for id in due.into_iter().rev() {
            if self.reqs.contains_key(&id) && !self.inflight.contains(&id) {
                self.queue.push_front(id);
            }
        }
        if self.refresh_due_at().is_some_and(|t| now >= t) {
            self.want_mpd = true;
        }
    }

    pub(super) fn mpd_action(&mut self, now: Timestamp) -> Option<Action> {
        if !self.want_mpd || self.mpd_inflight {
            return None;
        }
        self.want_mpd = false;
        self.mpd_inflight = true;
        let mut url = self.mpd_url.clone();
        let mut validators = MpdValidators::default();
        if let Some(live) = self.live.as_mut() {
            live.refresh.last_request = Some(now);
            live.refresh.next_at = None;
            live.refresh.forced_at = None;
            validators = live.refresh.validators.clone();
        }
        if let Some(u) = self.update_url() {
            url = u;
        }
        Some(Action::FetchMpd { url, validators })
    }

    fn update_url(&self) -> Option<Url> {
        let mpd = self.mpd.as_ref()?;
        if let Some(loc) = mpd.locations.first() {
            let base = self.doc_url.as_ref();
            let resolved = match base {
                Some(b) => b.join(loc).ok(),
                None => Url::parse(loc).ok(),
            };
            if let Some(parsed) = resolved
                && matches!(parsed.scheme(), "http" | "https")
            {
                let pin = !self.dash.cross_origin_urls;
                let origin_ok = !pin
                    || self
                        .doc_url
                        .as_ref()
                        .is_none_or(|d| same_origin(d, &parsed));
                if origin_ok {
                    return Some(parsed);
                }
            }
        }
        self.live
            .as_ref()?
            .refresh
            .redirect_url
            .as_deref()
            .and_then(|r| Url::parse(r).ok())
    }

    pub(super) fn utc_action(&mut self, now: Timestamp) -> Option<Action> {
        let dash = self.dash;
        let step = {
            let live = self.live.as_mut()?;
            let sampler = live.sampler.as_mut()?;
            sampler.poll(now, &mut self.next_utc_id, &dash)
        };
        match step {
            Step::Fetch(a) => Some(a),
            Step::Sample(sample) => {
                self.utc_sample(now, sample);
                None
            }
            Step::Exhausted => {
                self.sampler_exhausted(now);
                None
            }
            Step::Idle => None,
        }
    }

    // Clock ladder --------------------------------------------------------------

    pub(super) fn start_live(&mut self, now: Timestamp, bytes: &[u8], meta: &ResponseMeta<'_>) {
        self.live = Some(Live::new(now));
        if let Some(f) = meta.final_url
            && let Some(u) = self.accept_final_url(f)
        {
            if let Some(l) = self.live.as_mut() {
                l.refresh.redirect_url = Some(u.to_string());
            }
            self.doc_url = Some(u);
        }
        self.note_exchange(now, meta);
        if let Some(l) = self.live.as_mut() {
            l.refresh.body = bytes.to_vec();
            l.refresh.publish_time = self.mpd.as_ref().and_then(|m| m.publish_time);
            l.refresh.last_request = Some(meta.sent.min(meta.received));
        }
        self.speculative_activate(now);
        let want_sampler = self.begin_sampler(now, meta);
        if !want_sampler {
            self.finish_ladder(now);
        }
        self.schedule_refresh(now);
    }

    fn note_exchange(&mut self, now: Timestamp, meta: &ResponseMeta<'_>) {
        let adopted = meta.final_url.and_then(|f| self.accept_final_url(f));
        let Some(live) = self.live.as_mut() else {
            return;
        };
        live.mpd_exchange = Exchange::new(meta.sent.min(meta.received), meta.received);
        if let Some(e) = meta.etag.clone() {
            use headers::HeaderMapExt;
            let mut map = http::HeaderMap::new();
            map.typed_insert(e);
            if let Some(text) = map.get(http::header::ETAG).and_then(|v| v.to_str().ok()) {
                live.refresh.validators.etag = Some(text.to_owned());
            }
        }
        if let Some(m) = meta.last_modified {
            use headers::HeaderMapExt;
            let mut map = http::HeaderMap::new();
            map.typed_insert(m);
            if let Some(text) = map
                .get(http::header::LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
            {
                live.refresh.validators.last_modified = Some(text.to_owned());
            }
        }
        if let Some(u) = adopted {
            live.refresh.redirect_url = Some(u.to_string());
            self.doc_url = Some(u);
        }
        if let Some(date) = meta.date {
            let exch = live.mpd_exchange;
            if let Ok(sample) = sample_from_date_header(date, exch, self.dash.utc_max_rtt) {
                self.clock.offer(
                    sample,
                    ClockSource::MpdDate,
                    now,
                    OfferPolicy::new_or_never(self.dash.utc_resync, false),
                );
            }
        }
        let mid = self.live.as_ref().map_or(now, |l| {
            let half = l.mpd_exchange.received.saturating_sub(l.mpd_exchange.sent) / 2;
            l.mpd_exchange
                .sent
                .checked_add(half)
                .unwrap_or(l.mpd_exchange.sent)
        });
        let fetch_utc = self
            .clock
            .utc_at(mid)
            .map(|t| i128_from_i64(t.as_microsecond()))
            .or_else(|| {
                self.mpd
                    .as_ref()
                    .and_then(|m| m.publish_time)
                    .map(|t| i128_from_i64(t.as_microsecond()))
            });
        if let Some(l) = self.live.as_mut() {
            l.fetch_utc_us = fetch_utc;
        }
    }

    fn begin_sampler(&mut self, now: Timestamp, meta: &ResponseMeta<'_>) -> bool {
        let _ = meta;
        let host_first = self.dash.clock_policy == DashClockPolicy::PreferHost
            && self
                .clock
                .anchor()
                .is_some_and(|a| a.source == ClockSource::Host);
        if host_first {
            return false;
        }
        self.start_sampler(now, true)
    }

    fn start_sampler(&mut self, now: Timestamp, gating: bool) -> bool {
        let host_holds = self.dash.clock_policy == DashClockPolicy::PreferHost
            && self
                .clock
                .anchor()
                .is_some_and(|a| a.source == ClockSource::Host);
        if !gating && (host_holds || !self.is_live_mpd()) {
            return false;
        }
        let Some(mpd) = self.mpd.as_ref() else {
            return false;
        };
        let elements = mpd.utc_timings.clone();
        let unsupported = elements.iter().any(|e| {
            matches!(
                e.scheme,
                crate::timing::utc_timing::UtcScheme::Ntp
                    | crate::timing::utc_timing::UtcScheme::Sntp
                    | crate::timing::utc_timing::UtcScheme::Unknown
            )
        });
        let doc = self.doc_url.clone();
        let pin = !self.dash.cross_origin_urls;
        let mut cross = false;
        let exchange = self
            .live
            .as_ref()
            .map_or(Exchange::new(now, now), |l| l.mpd_exchange);
        let mut accept = |value: &str| -> Option<Url> {
            let url = match &doc {
                Some(d) => d.join(value).ok()?,
                None => Url::parse(value).ok()?,
            };
            if pin
                && let Some(d) = &doc
                && !same_origin(d, &url)
            {
                cross = true;
                return None;
            }
            matches!(url.scheme(), "http" | "https").then_some(url)
        };
        let mut sampler = UtcSampler::new(&elements, &mut accept, exchange);
        if unsupported {
            self.notice_once(Notice::UtcSchemeUnsupported);
        }
        if cross {
            self.notice_once(Notice::UtcCrossOrigin);
        }
        if sampler.is_empty() {
            return false;
        }
        sampler.gating = gating;
        if gating {
            sampler.wait_deadline = now.checked_add(self.dash.utc_wait);
        }
        if let Some(l) = self.live.as_mut() {
            l.sampler = Some(sampler);
        }
        true
    }

    fn utc_sample(&mut self, now: Timestamp, sample: UtcSample) {
        let outcome = self.clock.offer(
            sample,
            ClockSource::UtcTiming,
            now,
            OfferPolicy::new_or_never(self.dash.utc_resync, false),
        );
        let stepped = matches!(outcome, OfferOutcome::Adopted(AdoptReason::Step));
        let ready = {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            live.sampler = None;
            live.next_resync = resync_deadline(self.dash.utc_resync, now);
            live.ready
        };
        if stepped {
            self.force_refresh(now);
        }
        if ready {
            self.announce_clock();
        } else {
            self.finish_ladder(now);
        }
    }

    fn sampler_exhausted(&mut self, now: Timestamp) {
        let ready = {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            live.sampler = None;
            live.next_resync = resync_deadline(self.dash.utc_resync, now);
            live.ready
        };
        if !ready {
            self.finish_ladder(now);
        }
    }

    fn finish_ladder(&mut self, now: Timestamp) {
        if self.clock.anchor().is_none() {
            self.go_fatal(FatalKind::ClockUnavailable);
            return;
        }
        if let Some(live) = self.live.as_mut() {
            if live.ready {
                return;
            }
            live.ready = true;
        }
        self.announce_clock();
        self.on_clock_ready(now);
    }

    fn announce_clock(&mut self) {
        if self.stopped() {
            return;
        }
        let Some(anchor) = self.clock.anchor().copied() else {
            return;
        };
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !live.ready || live.announced_clock == Some(anchor.source) {
            return;
        }
        live.announced_clock = Some(anchor.source);
        self.outputs.push_back(Output::ClockSynced {
            source: anchor.source,
            uncertainty: anchor.uncertainty,
        });
    }

    fn request_resync(&mut self, now: Timestamp) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !live.ready || live.sampler.is_some() {
            return;
        }
        let spaced = live.last_forced_resync.is_none_or(|t| {
            t.checked_add(FORCED_RESYNC_MIN_GAP)
                .is_none_or(|gap| now >= gap)
        });
        if spaced {
            live.last_forced_resync = Some(now);
            live.next_resync = Some(now);
        }
    }

    // Join ----------------------------------------------------------------------

    fn delay(&self, plan_hint: Duration) -> Duration {
        let Some(mpd) = self.mpd.as_ref() else {
            return Duration::ZERO;
        };
        let max_seg = mpd.max_segment_duration.unwrap_or(plan_hint);
        presentation_delay(
            mpd.suggested_presentation_delay,
            max_seg,
            mpd.time_shift_buffer_depth,
            self.dash.live_delay,
        )
    }

    fn choose_join_period(&self, utc: i128) -> Option<usize> {
        let mpd = self.mpd.as_ref()?;
        let ast = mpd
            .availability_start_time
            .map(|t| i128_from_i64(t.as_microsecond()))?;
        let target = utc
            .saturating_sub(duration_micros(self.delay(Duration::ZERO)))
            .saturating_sub(ast);
        let timings = period_timings(mpd);
        let mut chosen: Option<usize> = None;
        for (i, t) in timings.iter().enumerate() {
            let Some(start) = t.start else { continue };
            if chosen.is_none() || duration_micros(start) <= target {
                chosen = Some(i);
            }
        }
        chosen
    }

    fn speculative_activate(&mut self, now: Timestamp) {
        let provisional = self.utc_now(now);
        let pick = provisional
            .and_then(|u| self.choose_join_period(u))
            .or_else(|| {
                let mpd = self.mpd.as_ref()?;
                period_timings(mpd).iter().rposition(|t| t.start.is_some())
            });
        if let Some(index) = pick {
            self.activate(index);
            self.issue = self.periods.len() - 1;
        }
    }

    fn on_clock_ready(&mut self, now: Timestamp) {
        let Some(utc) = self.utc_now(now) else {
            return;
        };
        let target = self.choose_join_period(utc);
        let current = self.periods.last().map(|p| p.index);
        let started = self.periods.last().is_some_and(|p| p.started);
        match (target, current) {
            (Some(t), Some(c)) if t != c && !started => {
                self.abandon_last_run();
                self.activate(t);
            }
            (Some(t), None) => self.activate(t),
            _ => {}
        }
        if !self.periods.is_empty() {
            self.issue = self.periods.len() - 1;
            if self.deliver >= self.periods.len() {
                self.deliver = self.periods.len() - 1;
            }
        }
        self.replan_all();
        self.drain();
    }

    fn abandon_last_run(&mut self) {
        let Some(run) = self.periods.pop() else {
            return;
        };
        let pi = self.periods.len();
        let mut ids: Vec<RepId> = Vec::new();
        for lane in &run.lanes {
            for rep in &lane.reps {
                ids.push(rep.id);
            }
        }
        self.rep_loc.retain(|_, (p, _, _)| *p != pi);
        let rep_set: BTreeSet<RepId> = ids.iter().copied().collect();
        let gone: Vec<ResourceId> = self
            .reqs
            .iter()
            .filter(|(_, i)| rep_set.contains(&i.rep))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            self.reqs.remove(&id);
            self.inflight.remove(&id);
            self.parked.remove(&id);
            self.queue.retain(|q| *q != id);
        }
        let used: BTreeSet<InitKey> = self
            .periods
            .iter()
            .flat_map(|p| p.lanes.iter())
            .flat_map(|l| l.reps.iter())
            .map(|r| r.init_key.clone())
            .collect();
        self.inits
            .retain(|k, e| e.state == InitState::Ready || used.contains(k));
        self.issue = self.periods.len().saturating_sub(1);
    }

    pub(super) fn live_lane_candidate(
        &mut self,
        pi: usize,
        li: usize,
        utc: i128,
        now: Timestamp,
    ) -> Candidate {
        let margin = span_micros(self.dash.availability_margin);
        for _ in 0..MAX_LANE_STEPS {
            let lane = &self.periods[pi].lanes[li];
            let Some(rep) = lane.current() else {
                return Candidate::Idle;
            };
            let Some(plan) = rep.plan.as_ref() else {
                return Candidate::Idle;
            };
            if !rep.joined {
                let hint = match plan.get(0) {
                    Ok(Some(s)) => s.duration,
                    Ok(None) => Duration::ZERO,
                    Err(e) => return Candidate::Unaddressable(address_name(&e)),
                };
                let delay = self.delay(hint);
                let utc_ts = match timestamp_from_micros_i128(utc) {
                    Some(t) => t,
                    None => return Candidate::Idle,
                };
                match join_index(plan, utc_ts, delay) {
                    Some(idx) => {
                        let last = match idx.checked_sub(1).map_or(Ok(None), |i| plan.get(i)) {
                            Ok(s) => s.map(|s| s.time_ticks),
                            Err(e) => return Candidate::Unaddressable(address_name(&e)),
                        };
                        let lane = &mut self.periods[pi].lanes[li];
                        let ri = lane.reps.len() - 1;
                        let rep = &mut lane.reps[ri];
                        rep.next_issue = idx;
                        rep.joined = true;
                        rep.last_time = last;
                        continue;
                    }
                    None => {
                        let first = match plan.get(0) {
                            Ok(s) => s,
                            Err(e) => return Candidate::Unaddressable(address_name(&e)),
                        };
                        let first = first
                            .and_then(|s| s.available_from)
                            .map(|a| i128_from_i64(a.as_microsecond()).saturating_add(margin));
                        return match first {
                            Some(a) if a > utc => Candidate::Wait(a),
                            _ => {
                                self.force_refresh(now);
                                self.request_resync(now);
                                Candidate::Idle
                            }
                        };
                    }
                }
            }
            let seg = match plan.get(rep.next_issue) {
                Ok(Some(seg)) => seg,
                Ok(None) => return Candidate::Idle,
                Err(e) => return Candidate::Unaddressable(address_name(&e)),
            };
            if seg
                .available_until
                .is_some_and(|u| i128_from_i64(u.as_microsecond()) < utc)
            {
                self.expire_before_request(pi, li, utc);
                continue;
            }
            let from = seg
                .available_from
                .map(|a| i128_from_i64(a.as_microsecond()).saturating_add(margin))
                .unwrap_or(i128::MIN);
            if utc < from {
                return Candidate::Wait(from);
            }
            return Candidate::Ready(seg.start.as_micros());
        }
        Candidate::Idle
    }

    fn expire_before_request(&mut self, pi: usize, li: usize, utc: i128) {
        let utc_ts = timestamp_from_micros_i128(utc);
        let (oldest, next) = {
            let lane = &self.periods[pi].lanes[li];
            let Some(rep) = lane.current() else { return };
            let Some(plan) = rep.plan.as_ref() else {
                return;
            };
            let oldest = utc_ts.and_then(|t| oldest_in_window(plan, t));
            (oldest, rep.next_issue)
        };
        self.outputs.push_back(Output::Notice(Notice::FellBehind));
        let lane = &mut self.periods[pi].lanes[li];
        lane.expired_run = lane.expired_run.saturating_add(1);
        let rejoin = lane.expired_run >= EXPIRED_REJOIN_RUN || oldest.is_none();
        match oldest {
            Some(o) if o > next && !rejoin => {
                let ri = lane.reps.len() - 1;
                lane.reps[ri].next_issue = o;
                let read = {
                    let rep = &lane.reps[ri];
                    rep.plan.as_ref().map(|p| {
                        let at_o = p.get(o);
                        let before = o.checked_sub(1).map_or(Ok(None), |i| p.get(i));
                        (at_o, before)
                    })
                };
                let fail = match &read {
                    Some((Err(e), _)) | Some((_, Err(e))) => {
                        Some(DropCause::Addressing(address_name(e)))
                    }
                    _ => None,
                };
                let last = match &read {
                    Some((Ok(_), Ok(last))) => last.as_ref().map(|s| s.time_ticks),
                    _ => None,
                };
                lane.reps[ri].last_time = last;
                push_marker(lane);
                if let Some(cause) = fail {
                    self.drop_rep(pi, li, cause);
                }
            }
            _ => self.rejoin_lane(pi, li),
        }
    }

    fn rejoin_lane(&mut self, pi: usize, li: usize) {
        let lane = &mut self.periods[pi].lanes[li];
        lane.expired_run = 0;
        if let Some(rep) = lane.reps.last_mut() {
            rep.joined = false;
        }
        push_marker(lane);
    }

    pub(super) fn segment_delivered(&mut self, id: ResourceId) {
        let rep = id.rep();
        if let Some(live) = self.live.as_mut() {
            live.early.remove(&id);
            live.missing_seen.remove(&rep);
        }
        if let Some((pi, li, _)) = self.rep_loc.get(&rep).copied()
            && let Some(lane) = self.periods.get_mut(pi).and_then(|p| p.lanes.get_mut(li))
        {
            lane.expired_run = 0;
        }
    }

    // Failures ------------------------------------------------------------------

    pub(super) fn live_segment_failure(
        &mut self,
        now: Timestamp,
        id: ResourceId,
        failure: Failure,
    ) -> Result<FailureClass> {
        let status = status_of(failure);
        if matches!(
            status,
            Some(STATUS_NOT_FOUND | STATUS_GONE | STATUS_TOO_EARLY)
        ) && let Some(utc) = self.utc_now(now)
        {
            let (from, until) = self
                .reqs
                .get(&id)
                .map_or((None, None), |r| (r.avail_from, r.avail_until));
            let grace = self.dash.early_grace;
            let past_window = until.is_some_and(|u| utc > i128_from_i64(u.as_microsecond()));
            let refused_past_window = past_window
                && status == Some(STATUS_TOO_EARLY)
                && self
                    .clock
                    .anchor()
                    .is_some_and(|a| a.source == ClockSource::UtcTiming);
            let early = !refused_past_window
                && (status == Some(STATUS_TOO_EARLY)
                    || from.is_some_and(|f| {
                        f.saturating_add(
                            jiff::SignedDuration::try_from(grace)
                                .unwrap_or(jiff::SignedDuration::MAX),
                        )
                        .map_or(true, |fw| utc < i128_from_i64(fw.as_microsecond()))
                    }));
            if early {
                return Ok(self.early_wait(now, id, from, utc));
            }
            if past_window {
                return Ok(self.expired_segment(now, id, utc));
            }
        }
        let kind = failure_kind(failure);
        if kind == FailureKind::Missing {
            let first = self
                .live
                .as_mut()
                .is_some_and(|l| l.missing_seen.insert(id.rep()));
            if first {
                self.force_refresh(now);
                self.request_resync(now);
            }
        }
        Ok(self.count_failure(now, id, kind, status))
    }

    fn early_wait(
        &mut self,
        now: Timestamp,
        id: ResourceId,
        from: Option<jiff::Timestamp>,
        utc: i128,
    ) -> FailureClass {
        self.inflight.remove(&id);
        let base = self.dash.early_retry;
        let cap = EARLY_RETRY_CAP.max(base);
        let retry_at = {
            let Some(live) = self.live.as_mut() else {
                return FailureClass::Retry {
                    attempt: 1,
                    kind: FailureKind::TooEarly,
                };
            };
            let e = live.early.entry(id).or_default();
            e.attempts = e.attempts.saturating_add(1);
            let shift = e.attempts.saturating_sub(1).min(EARLY_SHIFT_MAX);
            let delay = base.saturating_mul(1u32 << shift).min(cap);
            let at = now.checked_add(delay);
            e.retry_at = at;
            at
        };
        if from.is_some_and(|f| utc >= i128_from_i64(f.as_microsecond())) {
            self.request_resync(now);
        }
        match retry_at {
            Some(retry_at) => FailureClass::NotYetAvailable { retry_at },
            None => self.count_failure(now, id, FailureKind::TooEarly, None),
        }
    }

    fn expired_segment(&mut self, _now: Timestamp, id: ResourceId, utc: i128) -> FailureClass {
        self.inflight.remove(&id);
        self.failures.remove(&id);
        if let Some(live) = self.live.as_mut() {
            live.early.remove(&id);
        }
        self.mark_done(id);
        if let Some(slot) = self.slot_mut(id) {
            slot.state = SlotState::Skipped { marker: true };
        }
        self.outputs.push_back(Output::Notice(Notice::FellBehind));
        let utc_ts = timestamp_from_micros_i128(utc);
        if let Some((pi, li, _)) = self.rep_loc.get(&id.rep()).copied() {
            let (oldest, next) = {
                let lane = &self.periods[pi].lanes[li];
                match lane.current().and_then(|r| r.plan.as_ref().map(|p| (r, p))) {
                    Some((r, p)) => (utc_ts.and_then(|t| oldest_in_window(p, t)), r.next_issue),
                    None => (None, 0),
                }
            };
            let lane = &mut self.periods[pi].lanes[li];
            lane.expired_run = lane.expired_run.saturating_add(1);
            let rejoin = lane.expired_run >= EXPIRED_REJOIN_RUN || oldest.is_none();
            match oldest {
                Some(o) if o > next && !rejoin => {
                    let ri = lane.reps.len() - 1;
                    lane.reps[ri].next_issue = o;
                    let read = lane.reps[ri]
                        .plan
                        .as_ref()
                        .map(|p| o.checked_sub(1).map_or(Ok(None), |i| p.get(i)))
                        .unwrap_or(Ok(None));
                    let fail = match read {
                        Ok(s) => {
                            lane.reps[ri].last_time = s.map(|s| s.time_ticks);
                            None
                        }
                        Err(e) => Some(DropCause::Addressing(address_name(&e))),
                    };
                    if let Some(cause) = fail {
                        self.drop_rep(pi, li, cause);
                    }
                }
                Some(o) if o <= next && !rejoin => {}
                _ => self.rejoin_lane(pi, li),
            }
        }
        self.drain();
        FailureClass::Expired
    }

    pub(super) fn give_up_live_segment(
        &mut self,
        id: ResourceId,
        status: Option<http::StatusCode>,
    ) -> FailureClass {
        self.inflight.remove(&id);
        self.failures.remove(&id);
        if let Some(live) = self.live.as_mut() {
            live.early.remove(&id);
        }
        self.mark_done(id);
        if let Some(slot) = self.slot_mut(id) {
            slot.state = SlotState::Skipped { marker: true };
        }
        self.outputs.push_back(Output::SegmentLost { id, status });
        self.drain();
        FailureClass::SegmentSkipped
    }

    pub(super) fn after_rotation(&mut self, now: Timestamp) {
        if self.is_live_mpd() {
            self.force_refresh(now);
        }
    }

    pub(super) fn mpd_failed(&mut self, _now: Timestamp) -> u32 {
        self.mpd_failures = self.mpd_failures.saturating_add(1);
        if let Some(live) = self.live.as_mut() {
            live.refresh.awaiting_retry = true;
        }
        self.mpd_failures
    }

    // Refresh ---------------------------------------------------------------------

    fn force_refresh(&mut self, now: Timestamp) {
        self.force_refresh_after(now, Duration::ZERO);
    }

    fn force_refresh_after(&mut self, now: Timestamp, extra: Duration) {
        let floor = self.dash.min_mpd_refresh.max(extra);
        let live_open = self.live_open();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !live_open {
            return;
        }
        let earliest = live
            .refresh
            .last_request
            .and_then(|t| t.checked_add(floor))
            .map_or(now, |t| t.max(now));
        live.refresh.forced_at = Some(live.refresh.forced_at.map_or(earliest, |f| f.min(earliest)));
    }

    fn schedule_refresh(&mut self, _now: Timestamp) {
        let floor = self.dash.min_mpd_refresh;
        let mup = self.mpd.as_ref().and_then(|m| m.minimum_update_period);
        let Some(live) = self.live.as_mut() else {
            return;
        };
        self.mpd_failures = 0;
        live.refresh.awaiting_retry = false;
        let ex = live.mpd_exchange;
        let half = ex.received.saturating_sub(ex.sent) / 2;
        let mid = ex.sent.checked_add(half).unwrap_or(ex.sent);
        live.refresh.next_at = mup.and_then(|m| mid.checked_add(m.max(floor)));
    }

    pub(super) fn refresh_mpd(
        &mut self,
        now: Timestamp,
        new: Mpd,
        bytes: &[u8],
        meta: &ResponseMeta<'_>,
    ) {
        if self.live.is_none() {
            return;
        }
        self.note_exchange(now, meta);
        let (stale, same) = {
            let live = self.live.as_ref().expect("live present");
            let stale = matches!(
                (new.publish_time, live.refresh.publish_time),
                (Some(n), Some(o)) if n < o
            );
            (stale, live.refresh.body == bytes)
        };
        if stale {
            self.schedule_refresh(now);
            let floor = self.dash.min_mpd_refresh;
            if let Some(l) = self.live.as_mut() {
                l.refresh.next_at = now.checked_add(floor);
            }
            return;
        }
        self.schedule_refresh(now);
        if same {
            return;
        }
        if let Some(l) = self.live.as_mut() {
            l.refresh.body = bytes.to_vec();
            l.refresh.publish_time = new.publish_time.or(l.refresh.publish_time);
        }
        self.apply_mpd(now, new);
        self.refresh_schedule_after_apply(now);
    }

    fn refresh_schedule_after_apply(&mut self, now: Timestamp) {
        self.schedule_refresh(now);
    }

    fn apply_mpd(&mut self, now: Timestamp, new: Mpd) {
        if new.periods.is_empty() {
            return;
        }
        let dynamic = new.kind == PresentationType::Dynamic;
        if dynamic && new.availability_start_time.is_none() {
            return;
        }
        let patch_location = new.has_patch_location();
        let event_streams = new.has_event_streams();
        let reset = {
            let old = self.mpd.as_ref().expect("mpd present");
            let ids_differ = matches!((&old.id, &new.id), (Some(a), Some(b)) if a != b);
            let has_reset = new
                .essential_properties
                .iter()
                .chain(new.supplemental_properties.iter())
                .any(|d| d.scheme_id_uri == SCHEME_RESET);
            let old_dynamic = old.kind == PresentationType::Dynamic;
            (old_dynamic && dynamic && old.availability_start_time != new.availability_start_time)
                || ids_differ
                || has_reset
        };
        self.mpd = Some(new);
        if patch_location {
            self.notice_once(Notice::PatchIgnored);
        }
        if event_streams {
            self.notice_once(Notice::EventStreamIgnored);
        }
        if reset || !self.remap_periods() {
            self.reset_presentation(now);
            return;
        }
        self.replan_all();
        self.drain();
    }

    fn remap_periods(&mut self) -> bool {
        let Some(mpd) = self.mpd.as_ref() else {
            return false;
        };
        let timings = period_timings(mpd);
        let mut moves: Vec<(usize, usize)> = Vec::new();
        for (pi, run) in self.periods.iter().enumerate() {
            if run.finished {
                continue;
            }
            let found = mpd
                .periods
                .iter()
                .enumerate()
                .position(|(i, p)| match (&run.id, &p.id) {
                    (Some(a), Some(b)) => a == b,
                    (None, None) => {
                        timings.get(i).and_then(|t| t.start).map(duration_micros)
                            == Some(run.start_us)
                    }
                    _ => false,
                });
            match found {
                Some(ni) => moves.push((pi, ni)),
                None => return false,
            }
        }
        let mut vanished: Vec<(usize, usize)> = Vec::new();
        for (pi, ni) in moves {
            let policy = self.cfg.policy.clone();
            let mpd = self.mpd.as_ref().expect("mpd present");
            let selection = select(mpd, ni, &policy);
            let period = &mpd.periods[ni];
            let run = &mut self.periods[pi];
            run.index = ni;
            let mut kept_audio: Option<Option<RepRef>> = None;
            for (li, lane) in run.lanes.iter_mut().enumerate() {
                let audio_lane = lane.kind == SetClass::Audio;
                let Some(ri) = lane.reps.len().checked_sub(1) else {
                    continue;
                };
                let rep = &mut lane.reps[ri];
                let new_set = period
                    .adaptation_sets
                    .iter()
                    .position(|s| match rep.set_id {
                        Some(id) => s.id == Some(id),
                        None => false,
                    });
                let new_set = new_set.or_else(|| {
                    (rep.set_id.is_none() && rep.set_pos < period.adaptation_sets.len())
                        .then_some(rep.set_pos)
                });
                let found = new_set.and_then(|si| {
                    period.adaptation_sets[si]
                        .representations
                        .iter()
                        .position(|r| r.id == rep.rep_key)
                        .map(|rj| (si, rj))
                });
                match found {
                    Some((si, rj)) => {
                        rep.at = RepRef::at(ni, si, rj);
                        rep.set_pos = si;
                        if audio_lane {
                            kept_audio = Some((!rep.dead).then_some(rep.at));
                        }
                        if let Some((p, s, r)) = rep.at.resolve(mpd) {
                            rep.candidates = super::filter_candidates(
                                representation_candidates(mpd, p, s, r),
                                self.doc_url.as_ref(),
                                self.dash.cross_origin_urls,
                            );
                        }
                    }
                    None => vanished.push((pi, li)),
                }
            }
            let chosen = run
                .lanes
                .iter()
                .filter_map(|l| l.current())
                .map(|r| r.at)
                .collect::<Vec<_>>();
            run.selection = selection;
            if let Some(audio) = kept_audio {
                run.selection.chosen_audio = audio;
            }
            for at in chosen {
                if let Some(pos) = run.selection.variants.iter().position(|v| *v == at) {
                    run.selection.chosen_variant = Some(pos);
                }
            }
        }
        for (pi, li) in vanished {
            self.replace_vanished(pi, li);
        }
        true
    }

    fn replace_vanished(&mut self, pi: usize, li: usize) {
        let (kind, index) = {
            let lane = &self.periods[pi].lanes[li];
            (lane.kind, self.periods[pi].index)
        };
        let replacement = {
            let sel = &self.periods[pi].selection;
            match kind {
                SetClass::Video => sel
                    .chosen_variant
                    .and_then(|i| sel.variants.get(i).copied()),
                _ => sel.chosen_audio,
            }
        };
        let switch = {
            let lane = &self.periods[pi].lanes[li];
            lane.current()
                .and_then(|c| c.plan.as_ref().map(|p| p.get(c.next_issue)))
        };
        let switch_us = match switch {
            Some(Ok(seg)) => seg.map(|s| s.start),
            Some(Err(e)) => {
                self.drop_rep(pi, li, DropCause::Addressing(address_name(&e)));
                None
            }
            None => None,
        };
        let Some(at) = replacement.filter(|a| a.period == index) else {
            if let Some(rep) = self.periods[pi].lanes[li].reps.last_mut() {
                rep.dead = true;
            }
            return;
        };
        match self.build_rep(at) {
            Ok(mut rep) => {
                if let (Some(plan), Some(us)) = (rep.plan.as_ref(), switch_us) {
                    rep.next_issue = match plan.index_at_or_before(us) {
                        Ok(Some(i)) => i,
                        _ => 0,
                    };
                    rep.joined = true;
                    match rep
                        .next_issue
                        .checked_sub(1)
                        .map_or(Ok(None), |i| plan.get(i))
                    {
                        Ok(seg) => {
                            rep.last_time = seg.map(|s| s.time_ticks);
                        }
                        Err(e) => {
                            self.outputs.push_back(Output::TrackDropped {
                                period: index,
                                set: at.set,
                                rep: Some(at.rep),
                                cause: DropCause::Addressing(address_name(&e)),
                            });
                            if let Some(r) = self.periods[pi].lanes[li].reps.last_mut() {
                                r.dead = true;
                            }
                            return;
                        }
                    }
                }
                let ri = self.periods[pi].lanes[li].reps.len();
                push_marker(&mut self.periods[pi].lanes[li]);
                self.periods[pi].lanes[li].reps.push(rep);
                self.periods[pi].issue_done = false;
                self.register_rep(pi, li, ri);
            }
            Err(cause) => {
                self.outputs.push_back(Output::TrackDropped {
                    period: index,
                    set: at.set,
                    rep: Some(at.rep),
                    cause,
                });
                if let Some(rep) = self.periods[pi].lanes[li].reps.last_mut() {
                    rep.dead = true;
                }
            }
        }
    }

    pub(super) fn replan_all(&mut self) {
        let mut targets: Vec<(usize, usize, usize)> = Vec::new();
        for (pi, run) in self.periods.iter().enumerate() {
            if run.finished {
                continue;
            }
            for (li, lane) in run.lanes.iter().enumerate() {
                if let Some(ri) = lane.reps.len().checked_sub(1)
                    && !lane.reps[ri].dead
                {
                    targets.push((pi, li, ri));
                }
            }
        }
        for (pi, li, ri) in targets {
            self.replan_rep(pi, li, ri);
        }
    }

    fn replan_rep(&mut self, pi: usize, li: usize, ri: usize) {
        let (at, has_index) = {
            let rep = &self.periods[pi].lanes[li].reps[ri];
            (rep.at, rep.index.is_some())
        };
        if has_index {
            return;
        }
        let planned = {
            let mpd = self.mpd.as_ref().expect("mpd present");
            plan_representation(mpd, at, &self.plan_options())
        };
        if let Ok(p) = planned {
            let marker = {
                let rep = &mut self.periods[pi].lanes[li].reps[ri];
                if rep.joined {
                    let bound = p.plan.len().unwrap_or(u64::MAX / 2);
                    rep.next_issue = match rep.last_time {
                        Some(t) => p
                            .plan
                            .leading_true(bound, |s| s.time_ticks <= t)
                            .unwrap_or(0),
                        None => 0,
                    };
                }
                rep.plan = Some(p.plan);
                rep.joined && rep.next_issue == 0 && rep.last_time.is_some()
            };
            if marker {
                push_marker(&mut self.periods[pi].lanes[li]);
            }
        }
    }

    fn reset_presentation(&mut self, now: Timestamp) {
        self.periods.clear();
        self.issue = 0;
        self.deliver = 0;
        self.queue.clear();
        self.reqs.clear();
        self.inflight.clear();
        self.parked.clear();
        self.failures.clear();
        self.done.clear();
        self.remembered_done.clear();
        self.outstanding = 0;
        self.rep_loc.clear();
        self.prev = None;
        self.inits.clear();
        if let Some(live) = self.live.as_mut() {
            live.early.clear();
            live.missing_seen.clear();
        }
        self.on_clock_ready(now);
    }
}

fn push_marker(lane: &mut Lane) {
    if !lane
        .queue
        .back()
        .is_some_and(|s| matches!(s.state, SlotState::Marker))
    {
        lane.queue.push_back(Slot {
            id: None,
            rep: 0,
            plan_index: 0,
            state: SlotState::Marker,
        });
    }
}

#[cfg(test)]
mod live_decision_tests {
    use super::*;
    use crate::client::config::{DashClockPolicy, DashConfig};
    use crate::client::engine::DashClientConfig;
    use crate::client::meta::ResponseMeta;
    use crate::client::{DashClient, PullLimits};
    use crate::timing::clock::ClockSource;

    const NS: &str = "urn:mpeg:dash:schema:mpd:2011";
    const WALL_US: i64 = 1_767_225_600 * 1_000_000;

    fn mono(us: u64) -> Timestamp {
        Timestamp::from_micros(us)
    }

    fn wall() -> jiff::Timestamp {
        jiff::Timestamp::from_microsecond(WALL_US).unwrap()
    }

    fn mpd_of(attrs: &str, body: &str) -> String {
        format!(r#"<?xml version="1.0"?><MPD xmlns="{NS}" {attrs}>{body}</MPD>"#)
    }

    fn dynamic(attrs: &str, body: &str) -> String {
        mpd_of(
            &format!(
                r#"type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" minBufferTime="PT2S" {attrs}"#
            ),
            body,
        )
    }

    fn single_period(mup: bool) -> String {
        let mup = if mup {
            r#" minimumUpdatePeriod="PT2S""#
        } else {
            ""
        };
        dynamic(
            mup,
            r#"<Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" initialization="seg/init.mp4" media="seg/$Number$.m4s"/>
               <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
        )
    }

    fn utc_element(body: &str) -> String {
        format!(
            r#"<UTCTiming schemeIdUri="urn:mpeg:dash:utc:http-xsdate:2014" value="http://dash.test/utc"/>{body}"#
        )
    }

    fn fresh(now: Timestamp, text: &str) -> DashClient {
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        let _ = c.poll(now);
        c.on_mpd(now, text.as_bytes()).unwrap();
        c
    }

    #[test]
    fn failure_kind_maps_the_status_that_matters() {
        assert_eq!(failure_kind(Failure::Status(425)), FailureKind::TooEarly);
        assert_eq!(failure_kind(Failure::Status(404)), FailureKind::Missing);
        assert_eq!(failure_kind(Failure::Status(410)), FailureKind::Missing);
        assert_eq!(failure_kind(Failure::Status(500)), FailureKind::Server);
        assert_eq!(failure_kind(Failure::Transport), FailureKind::Transport);
        assert_eq!(failure_kind(Failure::TooLarge), FailureKind::Invalid);
    }

    #[test]
    fn live_bookkeeping_counts_the_early_waits_and_the_missing_flags() {
        let mut live = Live::new(mono(0));
        assert_eq!(live.bookkeeping(), 0);
        live.early.insert(
            ResourceId::Segment {
                rep: crate::client::action::RepId(1),
                number: 7,
            },
            Early::default(),
        );
        live.missing_seen.insert(crate::client::action::RepId(2));
        assert_eq!(live.bookkeeping(), 2);
        live.missing_seen.insert(crate::client::action::RepId(3));
        assert_eq!(live.bookkeeping(), 3);
    }

    #[test]
    fn utc_at_is_none_until_a_clock_exists_and_exact_afterwards() {
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        assert_eq!(c.utc_at(mono(0)), None);
        c.set_wall_clock(mono(1_000), wall());
        assert_eq!(c.utc_at(mono(1_000)), Some(wall()));
        assert_eq!(
            c.utc_at(mono(1_500)),
            Some(jiff::Timestamp::from_microsecond(WALL_US + 500).unwrap())
        );
    }

    #[test]
    fn a_host_clock_alone_does_not_drop_a_gating_sampler_under_auto() {
        let now = mono(0);
        let mut c = fresh(
            now,
            &dynamic(
                r#"minimumUpdatePeriod="PT2S""#,
                &utc_element(
                    r#"<Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
                   <SegmentTemplate timescale="1000" duration="2000" media="seg/$Number$.m4s"/>
                   <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
                ),
            ),
        );
        assert!(
            c.start_sampler(now, true),
            "the gating sampler starts when the MPD announces a UTCTiming"
        );
        c.set_wall_clock(mono(1_000), wall());
        assert!(
            c.live.as_ref().and_then(|l| l.sampler.as_ref()).is_some(),
            "Auto keeps asking the server"
        );
    }

    #[test]
    fn prefer_host_drops_the_gating_sampler_and_finishes_the_ladder() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig {
                    clock_policy: DashClockPolicy::PreferHost,
                    ..DashConfig::default()
                })
                .with_limits(PullLimits::default()),
        );
        let _ = c.poll(now);
        c.on_mpd(
            now,
            dynamic(
                r#"minimumUpdatePeriod="PT2S""#,
                &utc_element(
                    r#"<Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
                   <SegmentTemplate timescale="1000" duration="2000" media="seg/$Number$.m4s"/>
                   <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
                ),
            )
            .as_bytes(),
        )
        .unwrap();
        assert!(c.start_sampler(now, true));
        c.set_wall_clock(mono(1_000), wall());
        let live = c.live.as_ref().expect("live");
        assert!(live.sampler.is_none(), "PreferHost takes the host clock");
        assert!(live.ready, "the ladder finished on the host clock");
    }

    #[test]
    fn on_mpd_unchanged_needs_an_in_flight_mpd_and_a_live_presentation() {
        let now = mono(0);
        let meta = ResponseMeta::new(mono(0), mono(10));
        let mut c = fresh(now, &single_period(true));
        assert_eq!(
            c.on_mpd_unchanged(now, &meta),
            Err(Error::MpdNotAwaited),
            "no MPD fetch is outstanding"
        );
        let mut static_c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        let _ = static_c.poll(now);
        assert_eq!(
            static_c.on_mpd_unchanged(now, &meta),
            Err(Error::MpdNotAwaited),
            "a static presentation has no live state"
        );
    }

    #[test]
    fn can_activate_needs_a_known_period_start_for_a_dynamic_mpd() {
        let now = mono(0);
        let early = dynamic(
            r#"minimumUpdatePeriod="PT2S""#,
            r#"<Period id="A" start="PT0S"/><Period id="B"/>"#,
        );
        let c = fresh(now, &early);
        assert!(!c.can_activate(1), "Period B has no PeriodStart yet");
        assert!(!c.can_activate(2), "past the end");
        let known = dynamic(
            r#"minimumUpdatePeriod="PT2S""#,
            r#"<Period id="A" start="PT0S"/><Period id="B" start="PT4S"/>"#,
        );
        let c = fresh(now, &known);
        assert!(c.can_activate(1));
    }

    #[test]
    fn refresh_due_at_honours_each_slot_and_the_gates() {
        let now = mono(0);
        let mut c = fresh(now, &single_period(true));
        {
            let live = c.live.as_mut().expect("live");
            live.refresh.next_at = Some(mono(5_000));
            live.refresh.forced_at = Some(mono(3_000));
        }
        assert_eq!(c.refresh_due_at(), Some(mono(3_000)));
        {
            let live = c.live.as_mut().expect("live");
            live.refresh.forced_at = None;
        }
        assert_eq!(c.refresh_due_at(), Some(mono(5_000)));
        c.mpd_inflight = true;
        assert_eq!(c.refresh_due_at(), None, "an MPD fetch is outstanding");
        c.mpd_inflight = false;
        c.want_mpd = true;
        assert_eq!(c.refresh_due_at(), None, "an MPD fetch is already wanted");
        c.want_mpd = false;
        if let Some(live) = c.live.as_mut() {
            live.refresh.awaiting_retry = true;
        }
        assert_eq!(c.refresh_due_at(), None, "the caller is backing off");
        if let Some(live) = c.live.as_mut() {
            live.refresh.awaiting_retry = false;
        }
        assert_eq!(c.refresh_due_at(), Some(mono(5_000)));
    }

    #[test]
    fn refresh_is_never_due_when_the_period_cannot_grow() {
        let now = mono(0);
        let mut c = fresh(now, &single_period(false));
        if let Some(live) = c.live.as_mut() {
            live.refresh.next_at = Some(mono(1));
        }
        assert_eq!(
            c.refresh_due_at(),
            None,
            "a dynamic MPD without a minimumUpdatePeriod never refreshes"
        );
    }

    #[test]
    fn tick_keeps_a_sampler_that_does_not_gate_the_ladder() {
        let now = mono(10_000);
        let mut c = fresh(
            now,
            &dynamic(
                r#"minimumUpdatePeriod="PT2S""#,
                &utc_element(
                    r#"<Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
                   <SegmentTemplate timescale="1000" duration="2000" media="seg/$Number$.m4s"/>
                   <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
                ),
            ),
        );
        c.set_wall_clock(mono(0), wall());
        assert!(
            c.start_sampler(mono(0), false),
            "a non-gating resync sampler starts"
        );
        if let Some(s) = c.live.as_mut().and_then(|l| l.sampler.as_mut()) {
            s.gating = false;
            s.wait_deadline = Some(mono(5_000));
        }
        c.tick(now);
        assert!(
            c.live.as_ref().and_then(|l| l.sampler.as_ref()).is_some(),
            "a non-gating sampler is not dropped by the gate timer"
        );
    }

    #[test]
    fn a_due_early_retry_with_no_request_on_record_queues_nothing() {
        let mut c = lane_client(10);
        let id = ResourceId::Segment {
            rep: crate::client::action::RepId(999),
            number: 1,
        };
        c.live.as_mut().expect("live").early.insert(
            id,
            Early {
                attempts: 1,
                retry_at: Some(mono(0)),
            },
        );
        c.queue.clear();
        c.tick(mono(10));
        assert!(
            c.queue.is_empty(),
            "an early entry whose request is gone is not requeued"
        );
    }

    #[test]
    fn the_fetch_instant_is_the_exchange_midpoint() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(mono(0), wall());
        let _ = c.poll(now);
        let meta = ResponseMeta::new(mono(0), mono(4));
        c.on_mpd_with(now, single_period(true).as_bytes(), &meta)
            .unwrap();
        assert_eq!(
            c.live.as_ref().and_then(|l| l.fetch_utc_us),
            Some(i128::from(WALL_US) + 2),
            "the MPD FetchTime sits at sent + rtt/2"
        );
    }

    #[test]
    fn announce_clock_waits_until_the_ladder_is_ready() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        assert_eq!(c.clock.anchor().map(|a| a.source), Some(ClockSource::Host));
        let before = c
            .outputs
            .iter()
            .filter(|o| matches!(o, Output::ClockSynced { .. }))
            .count();
        assert_eq!(before, 0, "not ready yet");
        c.announce_clock();
        assert_eq!(
            c.outputs
                .iter()
                .filter(|o| matches!(o, Output::ClockSynced { .. }))
                .count(),
            0,
            "announcing again while not ready changes nothing"
        );
    }

    #[test]
    fn request_resync_waits_until_the_ladder_is_ready() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        let mut l = Live::new(now);
        l.ready = false;
        c.live = Some(l);
        c.request_resync(now);
        assert_eq!(
            c.live.as_ref().and_then(|l| l.next_resync),
            None,
            "not ready: no resync is requested"
        );
        if let Some(l) = c.live.as_mut() {
            l.ready = true;
        }
        c.request_resync(now);
        assert_eq!(c.live.as_ref().and_then(|l| l.next_resync), Some(now));
    }

    #[test]
    fn an_exhausted_sampler_without_a_clock_is_fatal() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.live = Some(Live::new(now));
        c.sampler_exhausted(now);
        assert!(
            matches!(
                c.outputs.back(),
                Some(Output::Fatal(FatalKind::ClockUnavailable))
            ),
            "{:?}",
            c.outputs
        );
    }

    #[test]
    fn the_speculative_period_is_activated_at_index_len_minus_one() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.mpd = Some(Mpd::parse(single_period(true).as_bytes()).expect("MPD parses"));
        c.speculative_activate(now);
        assert_eq!(c.periods.len(), 1);
        assert_eq!(c.issue, 0, "the issue cursor is the last Period");
    }

    #[test]
    fn on_clock_ready_without_a_period_is_a_no_op() {
        let now = mono(0);
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig::default())
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        c.on_clock_ready(now);
        assert!(c.periods.is_empty());
    }

    #[test]
    fn on_clock_ready_keeps_the_deliver_cursor_within_the_periods() {
        let now = mono(0);
        let two = dynamic(
            r#"minimumUpdatePeriod="PT2S""#,
            r#"<Period id="A" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" media="a/$Number$.m4s"/>
               <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>
               <Period id="B" start="PT30S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" media="b/$Number$.m4s"/>
               <Representation id="W" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
        );
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig {
                    clock_policy: DashClockPolicy::PreferHost,
                    ..DashConfig::default()
                })
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        let _ = c.poll(now);
        c.on_mpd(now, two.as_bytes()).unwrap();
        c.periods.clear();
        c.issue = 0;
        c.rep_loc.clear();
        c.activate(0);
        assert_eq!(c.periods.len(), 1);
        if let Some(p) = c.periods.last_mut() {
            p.started = true;
        }
        c.deliver = 0;
        c.on_clock_ready(now);
        assert_eq!(c.deliver, 0, "a deliver cursor inside the runs is kept");
        c.deliver = 7;
        c.on_clock_ready(now);
        assert_eq!(c.deliver, c.periods.len() - 1);
    }

    #[test]
    fn abandoning_the_last_run_drops_its_bookkeeping() {
        let now = mono(0);
        let mut c = fresh(now, &single_period(true));
        assert_eq!(c.periods.len(), 1);
        assert!(!c.rep_loc.is_empty(), "the period registered its reps");
        assert!(!c.queue.is_empty(), "its init fetch is queued");
        let key = c
            .periods
            .first()
            .and_then(|p| p.lanes.first())
            .and_then(|l| l.reps.first())
            .map(|r| r.init_key.clone());
        if let Some(key) = key {
            c.inits.insert(
                key,
                crate::client::engine::InitEntry {
                    state: crate::client::engine::InitState::Ready,
                    bytes: None,
                    specs: Vec::new(),
                    skipped_reported: BTreeSet::new(),
                },
            );
        }
        c.abandon_last_run();
        assert!(c.periods.is_empty(), "the run is gone");
        assert!(
            c.rep_loc.is_empty(),
            "its representation locations are gone"
        );
        assert!(c.queue.is_empty(), "its queued fetches are gone");
        assert_eq!(
            c.inits
                .values()
                .filter(|e| e.state == crate::client::engine::InitState::Ready)
                .count(),
            1,
            "a Ready init the presentation no longer uses is kept"
        );
    }

    const SEC: i128 = 1_000_000;

    fn lane_client(tsb_s: u64) -> DashClient {
        fresh(
            mono(0),
            &dynamic(
                &format!(r#"minimumUpdatePeriod="PT2S" timeShiftBufferDepth="PT{tsb_s}S""#),
                r#"<Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
                   <SegmentTemplate timescale="1000" duration="2000" initialization="seg/init.mp4" media="seg/$Number$.m4s"/>
                   <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
            ),
        )
    }

    fn wall_us() -> i128 {
        i128::from(WALL_US)
    }

    fn set_issue(c: &mut DashClient, next_issue: u64, expired_run: u32) {
        let lane = &mut c.periods[0].lanes[0];
        let rep = lane.reps.last_mut().expect("a representation");
        rep.joined = true;
        rep.next_issue = next_issue;
        lane.expired_run = expired_run;
    }

    fn lane_state(c: &DashClient) -> (u64, bool, u32, Option<u64>, bool) {
        let lane = &c.periods[0].lanes[0];
        let rep = lane.reps.last().expect("a representation");
        (
            rep.next_issue,
            rep.joined,
            lane.expired_run,
            rep.last_time,
            c.outputs
                .iter()
                .any(|o| matches!(o, Output::Notice(Notice::FellBehind))),
        )
    }

    #[test]
    fn a_segment_at_its_availability_instant_is_ready() {
        let mut c = lane_client(10);
        set_issue(&mut c, 3, 0);
        let from = wall_us() + 4 * 2 * SEC;
        let mut c1 = c;
        let ready = c1.live_lane_candidate(0, 0, from, mono(0));
        assert!(
            matches!(ready, Candidate::Ready(_)),
            "at the availability instant the segment is ready"
        );
        let waiting = c1.live_lane_candidate(0, 0, from - 1, mono(0));
        assert!(
            matches!(waiting, Candidate::Wait(a) if a == from),
            "one microsecond earlier it waits"
        );
    }

    #[test]
    fn a_segment_whose_window_ends_now_is_not_expired() {
        let mut c = lane_client(10);
        set_issue(&mut c, 3, 0);
        let until = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        let v = c.live_lane_candidate(0, 0, until, mono(0));
        assert!(
            matches!(v, Candidate::Ready(_)),
            "the window end itself is still inside"
        );
        assert!(!lane_state(&c).4, "no FellBehind at the boundary");
        let _ = c.live_lane_candidate(0, 0, until + 1, mono(0));
        assert!(lane_state(&c).4, "past the window end is a FellBehind");
    }

    #[test]
    fn a_lane_that_has_not_joined_waits_for_its_first_segment() {
        let mut c = lane_client(10);
        {
            let lane = &mut c.periods[0].lanes[0];
            let rep = lane.reps.last_mut().expect("a representation");
            rep.joined = false;
        }
        let from = wall_us() + 2 * SEC;
        let v = c.live_lane_candidate(0, 0, from - 1, mono(0));
        assert!(matches!(v, Candidate::Wait(a) if a == from));
        let v = c.live_lane_candidate(0, 0, from, mono(0));
        assert!(
            matches!(v, Candidate::Ready(_)),
            "at the first availability the join lands on the oldest segment"
        );
    }

    #[test]
    fn expiring_before_the_request_jumps_to_the_oldest_segment() {
        let mut c = lane_client(10);
        set_issue(&mut c, 0, 0);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        c.expire_before_request(0, 0, utc);
        let (next, joined, _, last, noticed) = lane_state(&c);
        assert_eq!(next, 3, "the lane jumps to the oldest segment in window");
        assert!(joined, "it keeps its join");
        assert_eq!(last, Some(4000), "the previous segment's media time");
        assert!(noticed);
    }

    #[test]
    fn expiring_exactly_at_the_next_segment_rejoins() {
        let mut c = lane_client(10);
        set_issue(&mut c, 3, 0);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        c.expire_before_request(0, 0, utc);
        let (next, joined, run, _, _) = lane_state(&c);
        assert_eq!(next, 3);
        assert!(!joined, "nothing was skipped, so the lane rejoins");
        assert_eq!(run, 0);
    }

    #[test]
    fn three_expiries_in_a_row_rejoin_even_past_a_skippable_gap() {
        let mut c = lane_client(10);
        set_issue(&mut c, 0, 3);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        c.expire_before_request(0, 0, utc);
        let (next, joined, run, _, _) = lane_state(&c);
        assert_eq!(next, 0, "a rejoin does not jump");
        assert!(!joined);
        assert_eq!(run, 0);
    }

    #[test]
    fn two_expiries_in_a_row_still_jump() {
        let mut c = lane_client(10);
        set_issue(&mut c, 0, 1);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        c.expire_before_request(0, 0, utc);
        let (next, joined, run, _, _) = lane_state(&c);
        assert_eq!(next, 3, "two is below the rejoin run");
        assert!(joined);
        assert_eq!(run, 2);
    }

    fn seg_id(c: &DashClient) -> ResourceId {
        let rep = c.periods[0].lanes[0].reps.last().expect("a rep").id;
        ResourceId::Segment { rep, number: 1 }
    }

    #[test]
    fn an_expired_response_jumps_to_the_oldest_segment() {
        let mut c = lane_client(10);
        set_issue(&mut c, 0, 0);
        let id = seg_id(&c);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        let class = c.expired_segment(mono(0), id, utc);
        assert_eq!(class, FailureClass::Expired);
        let (next, joined, _, last, noticed) = lane_state(&c);
        assert_eq!(next, 3);
        assert!(joined);
        assert_eq!(last, Some(4000));
        assert!(noticed);
    }

    #[test]
    fn an_expired_response_behind_the_next_segment_changes_nothing() {
        let mut c = lane_client(10);
        set_issue(&mut c, 3, 0);
        let id = seg_id(&c);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        let class = c.expired_segment(mono(0), id, utc);
        assert_eq!(class, FailureClass::Expired);
        let (next, joined, _, _, _) = lane_state(&c);
        assert_eq!(next, 3, "the cursor is not moved backwards");
        assert!(joined, "nothing was skipped");
    }

    #[test]
    fn a_run_of_expired_responses_rejoins_the_live_edge() {
        let mut c = lane_client(10);
        set_issue(&mut c, 0, 2);
        let id = seg_id(&c);
        let utc = wall_us() + (3 + 2) * 2 * SEC + 10 * SEC;
        let class = c.expired_segment(mono(0), id, utc);
        assert_eq!(class, FailureClass::Expired);
        let (next, joined, run, _, _) = lane_state(&c);
        assert_eq!(next, 0, "a rejoin does not jump");
        assert!(!joined);
        assert_eq!(run, 0);
    }

    fn request_at(c: &mut DashClient, from: jiff::Timestamp, until: jiff::Timestamp) -> ResourceId {
        let id = seg_id(c);
        let rep = id.rep();
        c.reqs.insert(
            id,
            crate::client::engine::ReqInfo {
                rep,
                reference: String::new(),
                range: None,
                kind: crate::client::engine::ReqKind::Segment,
                avail_from: Some(from),
                avail_until: Some(until),
            },
        );
        id
    }

    fn ts(us: i128) -> jiff::Timestamp {
        jiff::Timestamp::from_microsecond(i64::try_from(us).unwrap()).unwrap()
    }

    #[test]
    fn a_404_at_the_window_end_is_still_counted_not_expired() {
        let mut c = lane_client(10);
        let id = request_at(&mut c, ts(wall_us() - SEC), ts(wall_us()));
        let class = c
            .live_segment_failure(mono(0), id, Failure::Status(404))
            .unwrap();
        assert!(
            matches!(
                class,
                FailureClass::Retry {
                    kind: FailureKind::Missing,
                    ..
                }
            ),
            "the window end itself has not passed: {class:?}"
        );
    }

    #[test]
    fn a_404_exactly_at_the_end_of_the_grace_is_counted_not_early() {
        let mut c = lane_client(10);
        let id = request_at(&mut c, ts(wall_us() - SEC), ts(wall_us() + 10 * SEC));
        let class = c
            .live_segment_failure(mono(0), id, Failure::Status(404))
            .unwrap();
        assert!(
            matches!(
                class,
                FailureClass::Retry {
                    kind: FailureKind::Missing,
                    ..
                }
            ),
            "the grace has run out: {class:?}"
        );
    }

    #[test]
    fn a_closed_period_stays_open_for_the_grace_after_its_end() {
        let text = format!(
            r#"<?xml version="1.0"?><MPD xmlns="{NS}" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" minimumUpdatePeriod="PT2S" maxSegmentDuration="PT2S" minBufferTime="PT2S"><Period id="P0" start="PT0S" duration="PT10S"><AdaptationSet contentType="video" mimeType="video/mp4"><SegmentTemplate timescale="1000" initialization="seg/init.mp4" media="seg/$Time$.m4s"><SegmentTimeline><S t="0" d="2000" r="1"/></SegmentTimeline></SegmentTemplate><Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period></MPD>"#
        );
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig {
                    clock_policy: DashClockPolicy::PreferHost,
                    ..DashConfig::default()
                })
                .with_limits(PullLimits::default()),
        );
        let at = |us: i64| jiff::Timestamp::from_microsecond(WALL_US + us).unwrap();
        c.set_wall_clock(mono(0), at(13_000_000));
        let _ = c.poll(mono(0));
        c.on_mpd(mono(0), text.as_bytes()).unwrap();
        c.periods.clear();
        c.issue = 0;
        c.rep_loc.clear();
        c.activate(0);
        assert!(c.live_open());
        c.set_wall_clock(mono(1_000), at(14_000_000));
        assert!(
            c.period_open(0, mono(999)),
            "one microsecond inside the grace"
        );
        assert!(
            !c.period_open(0, mono(1_000)),
            "the grace (4 s) after a 10 s Period is over"
        );
    }

    #[test]
    fn on_clock_ready_does_not_retarget_a_period_that_has_started() {
        let now = mono(0);
        let two = dynamic(
            r#"minimumUpdatePeriod="PT2S""#,
            r#"<Period id="A" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" media="a/$Number$.m4s"/>
               <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>
               <Period id="B" start="PT30S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" media="b/$Number$.m4s"/>
               <Representation id="W" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
        );
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig {
                    clock_policy: DashClockPolicy::PreferHost,
                    live_delay: Some(Duration::ZERO),
                    ..DashConfig::default()
                })
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        let _ = c.poll(now);
        c.on_mpd(now, two.as_bytes()).unwrap();
        c.periods.clear();
        c.issue = 0;
        c.rep_loc.clear();
        c.activate(0);
        if let Some(p) = c.periods.last_mut() {
            p.started = true;
        }
        c.set_wall_clock(
            mono(1_000),
            jiff::Timestamp::from_microsecond(WALL_US + 40 * 1_000_000).unwrap(),
        );
        c.on_clock_ready(mono(1_000));
        assert_eq!(
            c.periods.last().map(|p| p.index),
            Some(0),
            "a started Period is not abandoned"
        );
    }

    #[test]
    fn on_clock_ready_retargets_a_period_that_has_not_started() {
        let now = mono(0);
        let two = dynamic(
            r#"minimumUpdatePeriod="PT2S""#,
            r#"<Period id="A" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" media="a/$Number$.m4s"/>
               <Representation id="V" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>
               <Period id="B" start="PT30S"><AdaptationSet contentType="video" mimeType="video/mp4">
               <SegmentTemplate timescale="1000" duration="2000" media="b/$Number$.m4s"/>
               <Representation id="W" bandwidth="1" codecs="avc1.4d400d"/></AdaptationSet></Period>"#,
        );
        let mut c = DashClient::new(
            DashClientConfig::new(Url::parse("http://dash.test/manifest.mpd").unwrap())
                .with_dash(DashConfig {
                    clock_policy: DashClockPolicy::PreferHost,
                    live_delay: Some(Duration::ZERO),
                    ..DashConfig::default()
                })
                .with_limits(PullLimits::default()),
        );
        c.set_wall_clock(now, wall());
        let _ = c.poll(now);
        c.on_mpd(now, two.as_bytes()).unwrap();
        c.periods.clear();
        c.issue = 0;
        c.rep_loc.clear();
        c.activate(0);
        assert_eq!(c.periods.last().map(|p| p.index), Some(0));
        c.set_wall_clock(
            mono(1_000),
            jiff::Timestamp::from_microsecond(WALL_US + 40 * 1_000_000).unwrap(),
        );
        c.on_clock_ready(mono(1_000));
        assert_eq!(
            c.periods.last().map(|p| p.index),
            Some(1),
            "the join moves to the Period that contains the live edge"
        );
    }
}
