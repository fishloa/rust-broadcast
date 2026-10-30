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
use crate::mpd::descriptors::SCHEME_RESET;
use crate::timing::clock::{AdoptReason, Anchor, ClockSource, OfferOutcome, OfferPolicy};
use crate::timing::live_edge::{join_index, oldest_in_window, presentation_delay_us};
use crate::timing::period::duration_us;
use crate::timing::utc_timing::{Exchange, UtcSample, sample_from_date_header};

/// A forced second look at the UTC clock after a segment error is taken at
/// most this often (23009-1 A.7: check the clock first, then the MPD).
const FORCED_RESYNC_MIN_GAP_US: u64 = 30_000_000;
/// Cap of the early-request cadence (DESIGN 5.4: 250 ms doubling to 1 s).
const EARLY_RETRY_CAP_US: u64 = 1_000_000;
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
    pub(super) next_at: Option<u64>,
    pub(super) forced_at: Option<u64>,
    pub(super) last_request_us: Option<u64>,
    pub(super) publish_time_us: Option<i64>,
    pub(super) body: Vec<u8>,
    pub(super) validators: MpdValidators,
    pub(super) redirect_url: Option<String>,
    pub(super) awaiting_retry: bool,
}

#[derive(Default, Clone, Copy)]
struct Early {
    attempts: u32,
    retry_at: Option<u64>,
}

pub(super) struct Live {
    pub(super) ready: bool,
    sampler: Option<UtcSampler>,
    next_resync_us: Option<u64>,
    last_forced_resync_us: Option<u64>,
    pub(super) refresh: Refresh,
    early: BTreeMap<ResourceId, Early>,
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

    fn new(now_us: u64) -> Self {
        Self {
            ready: false,
            sampler: None,
            next_resync_us: None,
            last_forced_resync_us: None,
            refresh: Refresh::default(),
            early: BTreeMap::new(),
            missing_seen: BTreeSet::new(),
            announced_clock: None,
            wake_utc: None,
            mpd_exchange: Exchange::new(now_us, now_us),
            fetch_utc_us: None,
        }
    }
}

fn status_of(failure: Failure) -> Option<u16> {
    match failure {
        Failure::Status(s) => Some(s),
        Failure::Transport | Failure::Timeout | Failure::TooLarge => None,
    }
}

pub(super) fn failure_kind(failure: Failure) -> FailureKind {
    match failure {
        Failure::Status(STATUS_NOT_FOUND | STATUS_GONE) => FailureKind::Missing,
        Failure::Status(STATUS_TOO_EARLY) => FailureKind::TooEarly,
        Failure::Status(_) => FailureKind::Server,
        Failure::Transport | Failure::Timeout => FailureKind::Transport,
        Failure::TooLarge => FailureKind::Invalid,
    }
}

fn i128_from_i64(v: i64) -> i128 {
    i128::from(v)
}

fn unix_i64(us: i128) -> i64 {
    i64::try_from(us.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap_or(i64::MAX)
}

impl DashClient {
    // Public surface ---------------------------------------------------------

    /// At the monotonic instant `now_us` the host's UTC clock read `unix_us`
    /// microseconds since 1970-01-01T00:00:00Z (POSIX, no leap seconds). Both
    /// reads must be taken back to back. Callable at any time and repeatedly
    /// (at open, every ten minutes, on a system clock-change notification):
    /// a dynamic presentation uses it when no `UTCTiming` answer is available
    /// (or first, with [`DashClockPolicy::PreferHost`]); a static one ignores
    /// it.
    pub fn set_wall_clock(&mut self, now_us: u64, unix_us: u64) {
        if self.stopped() {
            return;
        }
        let sample = UtcSample {
            mono_us: now_us,
            utc_us: unix_i64(i128::from(unix_us)),
            uncertainty_us: self.dash.host_clock_uncertainty_us,
        };
        let allow_downgrade = self.dash.clock_policy == DashClockPolicy::PreferHost;
        let outcome = self.clock.offer(
            sample,
            ClockSource::Host,
            now_us,
            OfferPolicy::new(self.dash.utc_resync_us, allow_downgrade),
        );
        let stepped = matches!(outcome, OfferOutcome::Adopted(AdoptReason::Step));
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if allow_downgrade && !live.ready {
            let gating = live.sampler.as_ref().is_some_and(|s| s.gating);
            if gating {
                live.sampler = None;
                self.finish_ladder(now_us);
            }
        }
        if stepped {
            self.force_refresh(now_us);
        }
        self.announce_clock();
    }

    /// The current clock anchor of a dynamic presentation.
    #[must_use]
    pub fn clock_anchor(&self) -> Option<Anchor> {
        self.clock.anchor().copied()
    }

    /// UTC in Unix microseconds at the monotonic instant `now_us`, once an
    /// anchor exists.
    #[must_use]
    pub fn utc_at(&self, now_us: u64) -> Option<i64> {
        self.clock.utc_at(now_us)
    }

    /// The earliest monotonic instant at which [`DashClient::poll`] may have
    /// something to do although no response arrived: a refresh due, a
    /// segment becoming available, an early retry, a `UTCTiming` timeout or
    /// resync. `None` when only a response (or a delivery) can move the client.
    #[must_use]
    pub fn next_wake(&self) -> Option<u64> {
        let live = self.live.as_ref()?;
        if self.stopped() {
            return None;
        }
        let mut best: Option<u64> = None;
        let mut take = |t: u64| best = Some(best.map_or(t, |b| b.min(t)));
        if let Some(t) = self.refresh_due_at() {
            take(t);
        }
        if let Some(s) = live.sampler.as_ref() {
            if let Some(d) = s.deadline_us() {
                take(d);
            }
            if let Some(w) = s.wait_deadline_us.filter(|_| s.gating) {
                take(w);
            }
        } else if live.ready
            && let Some(t) = live.next_resync_us
        {
            take(t);
        }
        for e in live.early.values() {
            if let Some(t) = e.retry_at {
                take(t);
            }
        }
        if let Some(w) = live.wake_utc
            && let Some(m) = self.clock.mono_for_utc(unix_i64(w))
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
        now_us: u64,
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
            self.utc_sample(now_us, sample);
        }
        Ok(())
    }

    /// Report a failed `UTCTiming` request (the client abandons it by itself
    /// after `DashConfig::utc_timeout_us`; a late answer then returns
    /// [`Error::UnrequestedUtc`], which the caller ignores).
    ///
    /// # Errors
    /// [`Error::UnrequestedUtc`] when `id` is not the request awaited;
    /// [`Error::Stopped`].
    pub fn on_utc_failure(&mut self, _now_us: u64, id: UtcId, failure: Failure) -> Result<()> {
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
    pub fn on_mpd_unchanged(&mut self, now_us: u64, meta: &ResponseMeta<'_>) -> Result<()> {
        self.ensure_live()?;
        if !self.mpd_inflight || self.live.is_none() {
            return Err(Error::MpdNotAwaited);
        }
        self.mpd_inflight = false;
        self.note_exchange(now_us, meta);
        self.schedule_refresh(now_us);
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
    pub(super) fn period_open(&self, pi: usize, now_us: u64) -> bool {
        if !self.live_open() {
            return false;
        }
        let Some(mpd) = self.mpd.as_ref() else {
            return false;
        };
        let run = &self.periods[pi];
        let Some(end) = period_timings(mpd).get(run.index).and_then(|t| t.end_us) else {
            return true;
        };
        let len = end.saturating_sub(run.start_us);
        let reaches = run.lanes.iter().filter_map(Lane::current).all(|r| {
            r.plan.as_ref().is_some_and(|p| {
                p.len()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|last| p.get(last))
                    .is_some_and(|s| {
                        s.start_us
                            .saturating_add(s.duration_us)
                            .saturating_add(MISSING_TAIL_MAX_US)
                            >= len
                    })
            })
        });
        if reaches {
            return false;
        }
        let Some(utc) = self.utc_now(now_us) else {
            return false;
        };
        let ast = i128::from(mpd.availability_start_time_us.unwrap_or(0));
        let mup = mpd.minimum_update_period.map_or(0, duration_us);
        let seg = mpd.max_segment_duration.map_or(0, duration_us);
        let grace = mup.max(seg.saturating_mul(PERIOD_FINAL_SEGMENTS));
        utc < ast.saturating_add(end).saturating_add(grace)
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
                .is_some_and(|t| t.start_us.is_some())
    }

    pub(super) fn utc_now(&self, now_us: u64) -> Option<i128> {
        self.clock.utc_at(now_us).map(i128_from_i64)
    }

    pub(super) fn plan_options(&self) -> PlanOptions<'static> {
        let mut o = PlanOptions::default()
            .with_availability_time_offset(self.dash.use_availability_time_offset);
        if let Some(h) = self.horizon() {
            o = o.with_horizon_utc_us(h);
        }
        o
    }

    fn horizon(&self) -> Option<i128> {
        let mpd = self.mpd.as_ref()?;
        let mup = duration_us(mpd.minimum_update_period?);
        let fetch = self.live.as_ref()?.fetch_utc_us?;
        Some(fetch.saturating_add(mup))
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

    pub(super) fn note_exhausted(&mut self, now_us: u64) {
        if !self.live_open() {
            return;
        }
        let tail_us = self
            .periods
            .get(self.issue)
            .map(|p| {
                p.lanes
                    .iter()
                    .filter_map(|l| l.current())
                    .filter_map(|r| {
                        let p = r.plan.as_ref()?;
                        let last = p.len()?.checked_sub(1)?;
                        p.get(last).map(|s| s.duration_us)
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let wait = u64::try_from(tail_us.max(0)).unwrap_or(u64::MAX);
        self.force_refresh_after(now_us, wait);
    }

    // Timers ------------------------------------------------------------------

    fn refresh_due_at(&self) -> Option<u64> {
        let live = self.live.as_ref()?;
        if !self.live_open() || self.mpd_inflight || self.want_mpd || live.refresh.awaiting_retry {
            return None;
        }
        match (live.refresh.next_at, live.refresh.forced_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub(super) fn tick(&mut self, now_us: u64) {
        if self.live.is_none() {
            return;
        }
        let gate_expired = self
            .live
            .as_ref()
            .and_then(|l| l.sampler.as_ref())
            .is_some_and(|s| s.gating && s.wait_deadline_us.is_some_and(|d| now_us >= d));
        if gate_expired {
            if let Some(l) = self.live.as_mut() {
                l.sampler = None;
                l.next_resync_us = Some(now_us.saturating_add(self.dash.utc_resync_us));
            }
            self.finish_ladder(now_us);
            if self.stopped() {
                return;
            }
        }
        let resync_due = self.live.as_ref().is_some_and(|l| {
            l.ready && l.sampler.is_none() && l.next_resync_us.is_some_and(|t| now_us >= t)
        });
        if resync_due {
            if let Some(l) = self.live.as_mut() {
                l.next_resync_us = None;
            }
            self.start_sampler(now_us, false);
        }
        let mut due: Vec<ResourceId> = Vec::new();
        if let Some(l) = self.live.as_mut() {
            for (id, e) in &mut l.early {
                if e.retry_at.is_some_and(|t| now_us >= t) {
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
        if self.refresh_due_at().is_some_and(|t| now_us >= t) {
            self.want_mpd = true;
        }
    }

    pub(super) fn mpd_action(&mut self, now_us: u64) -> Option<Action> {
        if !self.want_mpd || self.mpd_inflight {
            return None;
        }
        self.want_mpd = false;
        self.mpd_inflight = true;
        let mut url = self.mpd_url.clone();
        let mut validators = MpdValidators::default();
        if let Some(live) = self.live.as_mut() {
            live.refresh.last_request_us = Some(now_us);
            live.refresh.next_at = None;
            live.refresh.forced_at = None;
            validators = live.refresh.validators.clone();
        }
        if let Some(u) = self.update_url() {
            url = u;
        }
        Some(Action::FetchMpd { url, validators })
    }

    fn update_url(&mut self) -> Option<String> {
        let live = self.live.as_ref()?;
        let mpd = self.mpd.as_ref()?;
        if let Some(loc) = mpd.locations.first() {
            let base = self.doc_url.as_ref();
            let resolved = match base {
                Some(b) => b.join(loc).ok().map(|u| u.to_string()),
                None => Url::parse(loc).ok().map(|u| u.to_string()),
            };
            if let Some(url) = resolved
                && let Ok(parsed) = Url::parse(&url)
                && matches!(parsed.scheme(), "http" | "https")
            {
                let pin = !self.dash.cross_origin_urls;
                let origin_ok = !pin
                    || self
                        .doc_url
                        .as_ref()
                        .is_none_or(|d| same_origin(d, &parsed));
                if origin_ok {
                    return Some(url);
                }
            }
        }
        live.refresh.redirect_url.clone()
    }

    pub(super) fn utc_action(&mut self, now_us: u64) -> Option<Action> {
        let dash = self.dash;
        let step = {
            let live = self.live.as_mut()?;
            let sampler = live.sampler.as_mut()?;
            sampler.poll(now_us, &mut self.next_utc_id, &dash)
        };
        match step {
            Step::Fetch(a) => Some(a),
            Step::Sample(sample) => {
                self.utc_sample(now_us, sample);
                None
            }
            Step::Exhausted => {
                self.sampler_exhausted(now_us);
                None
            }
            Step::Idle => None,
        }
    }

    // Clock ladder --------------------------------------------------------------

    pub(super) fn start_live(&mut self, now_us: u64, bytes: &[u8], meta: &ResponseMeta<'_>) {
        self.live = Some(Live::new(now_us));
        if let Some(f) = meta.final_url {
            let parsed = Url::parse(f).ok();
            let pin = !self.dash.cross_origin_urls;
            let allow = parsed.as_ref().is_some_and(|u| {
                matches!(u.scheme(), "http" | "https")
                    && (!pin || self.doc_url.as_ref().is_none_or(|d| same_origin(d, u)))
            });
            if allow {
                if let Some(l) = self.live.as_mut() {
                    l.refresh.redirect_url = Some(f.to_owned());
                }
                if let Some(u) = parsed {
                    self.doc_url = Some(u);
                }
            }
        }
        self.note_exchange(now_us, meta);
        if let Some(l) = self.live.as_mut() {
            l.refresh.body = bytes.to_vec();
            l.refresh.publish_time_us = self.mpd.as_ref().and_then(|m| m.publish_time_us);
            l.refresh.last_request_us = Some(meta.sent_us.min(meta.received_us));
        }
        self.speculative_activate(now_us);
        let want_sampler = self.begin_sampler(now_us, meta);
        if !want_sampler {
            self.finish_ladder(now_us);
        }
        self.schedule_refresh(now_us);
    }

    fn note_exchange(&mut self, now_us: u64, meta: &ResponseMeta<'_>) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        live.mpd_exchange = Exchange::new(meta.sent_us.min(meta.received_us), meta.received_us);
        if let Some(e) = meta.etag {
            live.refresh.validators.etag = Some(e.to_owned());
        }
        if let Some(m) = meta.last_modified {
            live.refresh.validators.last_modified = Some(m.to_owned());
        }
        if let Some(f) = meta.final_url {
            let parsed = Url::parse(f).ok();
            let pin = !self.dash.cross_origin_urls;
            let allow = parsed.as_ref().is_some_and(|u| {
                matches!(u.scheme(), "http" | "https")
                    && (!pin || self.doc_url.as_ref().is_none_or(|d| same_origin(d, u)))
            });
            if allow {
                live.refresh.redirect_url = Some(f.to_owned());
                if let Some(u) = parsed {
                    self.doc_url = Some(u);
                }
            }
        }
        if let Some(date) = meta.date {
            let exch = live.mpd_exchange;
            if let Ok(sample) = sample_from_date_header(date, exch, self.dash.utc_max_rtt_us) {
                self.clock.offer(
                    sample,
                    ClockSource::MpdDate,
                    now_us,
                    OfferPolicy::new(self.dash.utc_resync_us, false),
                );
            }
        }
        let mid = self.live.as_ref().map_or(now_us, |l| {
            l.mpd_exchange.sent_us + (l.mpd_exchange.received_us - l.mpd_exchange.sent_us) / 2
        });
        let fetch_utc = self.clock.utc_at(mid).map(i128_from_i64).or_else(|| {
            self.mpd
                .as_ref()
                .and_then(|m| m.publish_time_us)
                .map(i128_from_i64)
        });
        if let Some(l) = self.live.as_mut() {
            l.fetch_utc_us = fetch_utc;
        }
    }

    fn begin_sampler(&mut self, now_us: u64, meta: &ResponseMeta<'_>) -> bool {
        let _ = meta;
        let host_first = self.dash.clock_policy == DashClockPolicy::PreferHost
            && self
                .clock
                .anchor()
                .is_some_and(|a| a.source == ClockSource::Host);
        if host_first {
            return false;
        }
        self.start_sampler(now_us, true)
    }

    fn start_sampler(&mut self, now_us: u64, gating: bool) -> bool {
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
            .map_or(Exchange::new(now_us, now_us), |l| l.mpd_exchange);
        let mut accept = |value: &str| -> Option<String> {
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
            matches!(url.scheme(), "http" | "https").then(|| url.to_string())
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
            sampler.wait_deadline_us = Some(now_us.saturating_add(self.dash.utc_wait_us));
        }
        if let Some(l) = self.live.as_mut() {
            l.sampler = Some(sampler);
        }
        true
    }

    fn utc_sample(&mut self, now_us: u64, sample: UtcSample) {
        let outcome = self.clock.offer(
            sample,
            ClockSource::UtcTiming,
            now_us,
            OfferPolicy::new(self.dash.utc_resync_us, false),
        );
        let stepped = matches!(outcome, OfferOutcome::Adopted(AdoptReason::Step));
        let ready = {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            live.sampler = None;
            live.next_resync_us = Some(now_us.saturating_add(self.dash.utc_resync_us));
            live.ready
        };
        if stepped {
            self.force_refresh(now_us);
        }
        if ready {
            self.announce_clock();
        } else {
            self.finish_ladder(now_us);
        }
    }

    fn sampler_exhausted(&mut self, now_us: u64) {
        let ready = {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            live.sampler = None;
            live.next_resync_us = Some(now_us.saturating_add(self.dash.utc_resync_us));
            live.ready
        };
        if !ready {
            self.finish_ladder(now_us);
        }
    }

    fn finish_ladder(&mut self, now_us: u64) {
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
        self.on_clock_ready(now_us);
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
            uncertainty_us: anchor.uncertainty_us,
        });
    }

    fn request_resync(&mut self, now_us: u64) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !live.ready || live.sampler.is_some() {
            return;
        }
        let spaced = live
            .last_forced_resync_us
            .is_none_or(|t| now_us >= t.saturating_add(FORCED_RESYNC_MIN_GAP_US));
        if spaced {
            live.last_forced_resync_us = Some(now_us);
            live.next_resync_us = Some(now_us);
        }
    }

    // Join ----------------------------------------------------------------------

    fn delay_us(&self, plan_hint_us: i128) -> i128 {
        let Some(mpd) = self.mpd.as_ref() else {
            return 0;
        };
        let max_seg = mpd.max_segment_duration.map_or(plan_hint_us, duration_us);
        presentation_delay_us(
            mpd.suggested_presentation_delay,
            max_seg,
            mpd.time_shift_buffer_depth.map(duration_us),
            self.dash.live_delay_us.map(i128::from),
        )
    }

    fn choose_join_period(&self, utc: i128) -> Option<usize> {
        let mpd = self.mpd.as_ref()?;
        let ast = i128::from(mpd.availability_start_time_us?);
        let target = utc.saturating_sub(self.delay_us(0)).saturating_sub(ast);
        let timings = period_timings(mpd);
        let mut chosen: Option<usize> = None;
        for (i, t) in timings.iter().enumerate() {
            let Some(start) = t.start_us else { continue };
            if chosen.is_none() || start <= target {
                chosen = Some(i);
            }
        }
        chosen
    }

    fn speculative_activate(&mut self, now_us: u64) {
        let provisional = self.utc_now(now_us);
        let pick = provisional
            .and_then(|u| self.choose_join_period(u))
            .or_else(|| {
                let mpd = self.mpd.as_ref()?;
                period_timings(mpd)
                    .iter()
                    .rposition(|t| t.start_us.is_some())
            });
        if let Some(index) = pick {
            self.activate(index);
            self.issue = self.periods.len() - 1;
        }
    }

    fn on_clock_ready(&mut self, now_us: u64) {
        let Some(utc) = self.utc_now(now_us) else {
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
        now_us: u64,
    ) -> Candidate {
        let margin = i128::from(self.dash.availability_margin_us);
        for _ in 0..MAX_LANE_STEPS {
            let lane = &self.periods[pi].lanes[li];
            let Some(rep) = lane.current() else {
                return Candidate::Idle;
            };
            let Some(plan) = rep.plan.as_ref() else {
                return Candidate::Idle;
            };
            if !rep.joined {
                let hint = plan.get(0).map_or(0, |s| s.duration_us);
                let delay = self.delay_us(hint);
                match join_index(plan, utc, delay) {
                    Some(idx) => {
                        let last = idx
                            .checked_sub(1)
                            .and_then(|i| plan.get(i))
                            .map(|s| s.time_ticks);
                        let lane = &mut self.periods[pi].lanes[li];
                        let ri = lane.reps.len() - 1;
                        let rep = &mut lane.reps[ri];
                        rep.next_issue = idx;
                        rep.joined = true;
                        rep.last_time = last;
                        continue;
                    }
                    None => {
                        let first = plan
                            .get(0)
                            .and_then(|s| s.available_from_us)
                            .map(|a| a.saturating_add(margin));
                        return match first {
                            Some(a) if a > utc => Candidate::Wait(a),
                            _ => {
                                self.force_refresh(now_us);
                                self.request_resync(now_us);
                                Candidate::Idle
                            }
                        };
                    }
                }
            }
            let Some(seg) = plan.get(rep.next_issue) else {
                return Candidate::Idle;
            };
            if seg.available_until_us.is_some_and(|u| u < utc) {
                self.expire_before_request(pi, li, utc);
                continue;
            }
            let from = seg
                .available_from_us
                .unwrap_or(i128::MIN)
                .saturating_add(margin);
            if utc < from {
                return Candidate::Wait(from);
            }
            return Candidate::Ready(seg.start_us);
        }
        Candidate::Idle
    }

    fn expire_before_request(&mut self, pi: usize, li: usize, utc: i128) {
        let (oldest, next) = {
            let lane = &self.periods[pi].lanes[li];
            let Some(rep) = lane.current() else { return };
            let Some(plan) = rep.plan.as_ref() else {
                return;
            };
            (oldest_in_window(plan, utc), rep.next_issue)
        };
        self.outputs.push_back(Output::Notice(Notice::FellBehind));
        let lane = &mut self.periods[pi].lanes[li];
        lane.expired_run = lane.expired_run.saturating_add(1);
        let rejoin = lane.expired_run >= EXPIRED_REJOIN_RUN || oldest.is_none();
        match oldest {
            Some(o) if o > next && !rejoin => {
                let ri = lane.reps.len() - 1;
                lane.reps[ri].next_issue = o;
                let last = lane.reps[ri]
                    .plan
                    .as_ref()
                    .and_then(|p| o.checked_sub(1).and_then(|i| p.get(i)))
                    .map(|s| s.time_ticks);
                lane.reps[ri].last_time = last;
                push_marker(lane);
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
        now_us: u64,
        id: ResourceId,
        failure: Failure,
    ) -> Result<FailureClass> {
        let status = status_of(failure);
        if matches!(
            status,
            Some(STATUS_NOT_FOUND | STATUS_GONE | STATUS_TOO_EARLY)
        ) && let Some(utc) = self.utc_now(now_us)
        {
            let (from, until) = self
                .reqs
                .get(&id)
                .map_or((None, None), |r| (r.avail_from_us, r.avail_until_us));
            let grace = i128::from(self.dash.early_grace_us);
            let past_window = until.is_some_and(|u| utc > u);
            let refused_past_window = past_window
                && status == Some(STATUS_TOO_EARLY)
                && self
                    .clock
                    .anchor()
                    .is_some_and(|a| a.source == ClockSource::UtcTiming);
            let early = !refused_past_window
                && (status == Some(STATUS_TOO_EARLY)
                    || from.is_some_and(|f| utc < f.saturating_add(grace)));
            if early {
                return Ok(self.early_wait(now_us, id, from, utc));
            }
            if past_window {
                return Ok(self.expired_segment(now_us, id, utc));
            }
        }
        let kind = failure_kind(failure);
        if kind == FailureKind::Missing {
            let first = self
                .live
                .as_mut()
                .is_some_and(|l| l.missing_seen.insert(id.rep()));
            if first {
                self.force_refresh(now_us);
                self.request_resync(now_us);
            }
        }
        Ok(self.count_failure(now_us, id, kind, status))
    }

    fn early_wait(
        &mut self,
        now_us: u64,
        id: ResourceId,
        from: Option<i128>,
        utc: i128,
    ) -> FailureClass {
        self.inflight.remove(&id);
        let base = self.dash.early_retry_us;
        let cap = EARLY_RETRY_CAP_US.max(base);
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
            let delay = base.saturating_mul(1u64 << shift).min(cap);
            let at = now_us.saturating_add(delay);
            e.retry_at = Some(at);
            at
        };
        if from.is_some_and(|f| utc >= f) {
            self.request_resync(now_us);
        }
        FailureClass::NotYetAvailable {
            retry_at_us: retry_at,
        }
    }

    fn expired_segment(&mut self, _now_us: u64, id: ResourceId, utc: i128) -> FailureClass {
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
        if let Some((pi, li, _)) = self.rep_loc.get(&id.rep()).copied() {
            let (oldest, next) = {
                let lane = &self.periods[pi].lanes[li];
                match lane.current().and_then(|r| r.plan.as_ref().map(|p| (r, p))) {
                    Some((r, p)) => (oldest_in_window(p, utc), r.next_issue),
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
                    let last = lane.reps[ri]
                        .plan
                        .as_ref()
                        .and_then(|p| o.checked_sub(1).and_then(|i| p.get(i)))
                        .map(|s| s.time_ticks);
                    lane.reps[ri].last_time = last;
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
        status: Option<u16>,
    ) -> FailureClass {
        self.inflight.remove(&id);
        self.failures.remove(&id);
        self.mark_done(id);
        if let Some(slot) = self.slot_mut(id) {
            slot.state = SlotState::Skipped { marker: true };
        }
        self.outputs.push_back(Output::SegmentLost { id, status });
        self.drain();
        FailureClass::SegmentSkipped
    }

    pub(super) fn after_rotation(&mut self, now_us: u64) {
        if self.is_live_mpd() {
            self.force_refresh(now_us);
        }
    }

    pub(super) fn mpd_failed(&mut self, _now_us: u64) -> u32 {
        self.mpd_failures = self.mpd_failures.saturating_add(1);
        if let Some(live) = self.live.as_mut() {
            live.refresh.awaiting_retry = true;
        }
        self.mpd_failures
    }

    // Refresh ---------------------------------------------------------------------

    fn force_refresh(&mut self, now_us: u64) {
        self.force_refresh_after(now_us, 0);
    }

    fn force_refresh_after(&mut self, now_us: u64, extra_us: u64) {
        let floor = self.dash.min_mpd_refresh_us.max(extra_us);
        let live_open = self.live_open();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if !live_open {
            return;
        }
        let earliest = live
            .refresh
            .last_request_us
            .map_or(now_us, |t| t.saturating_add(floor))
            .max(now_us);
        live.refresh.forced_at = Some(live.refresh.forced_at.map_or(earliest, |f| f.min(earliest)));
    }

    fn schedule_refresh(&mut self, _now_us: u64) {
        let floor = self.dash.min_mpd_refresh_us;
        let mup = self
            .mpd
            .as_ref()
            .and_then(|m| m.minimum_update_period)
            .map(|d| u64::try_from(duration_us(d)).unwrap_or(u64::MAX));
        let Some(live) = self.live.as_mut() else {
            return;
        };
        self.mpd_failures = 0;
        live.refresh.awaiting_retry = false;
        let ex = live.mpd_exchange;
        let mid = ex
            .sent_us
            .saturating_add(ex.received_us.saturating_sub(ex.sent_us) / 2);
        live.refresh.next_at = mup.map(|m| mid.saturating_add(m.max(floor)));
    }

    pub(super) fn refresh_mpd(
        &mut self,
        now_us: u64,
        new: Mpd,
        bytes: &[u8],
        meta: &ResponseMeta<'_>,
    ) {
        if self.live.is_none() {
            return;
        }
        self.note_exchange(now_us, meta);
        let (stale, same) = {
            let live = self.live.as_ref().expect("live present");
            let stale = matches!(
                (new.publish_time_us, live.refresh.publish_time_us),
                (Some(n), Some(o)) if n < o
            );
            (stale, live.refresh.body == bytes)
        };
        if stale {
            self.schedule_refresh(now_us);
            let floor = self.dash.min_mpd_refresh_us;
            if let Some(l) = self.live.as_mut() {
                l.refresh.next_at = Some(now_us.saturating_add(floor));
            }
            return;
        }
        self.schedule_refresh(now_us);
        if same {
            return;
        }
        if let Some(l) = self.live.as_mut() {
            l.refresh.body = bytes.to_vec();
            l.refresh.publish_time_us = new.publish_time_us.or(l.refresh.publish_time_us);
        }
        self.apply_mpd(now_us, new);
        self.refresh_schedule_after_apply(now_us);
    }

    fn refresh_schedule_after_apply(&mut self, now_us: u64) {
        self.schedule_refresh(now_us);
    }

    fn apply_mpd(&mut self, now_us: u64, new: Mpd) {
        if new.periods.is_empty() {
            return;
        }
        let dynamic = new.kind == PresentationType::Dynamic;
        if dynamic && new.availability_start_time_us.is_none() {
            return;
        }
        let features = new.features;
        let reset = {
            let old = self.mpd.as_ref().expect("mpd present");
            let ids_differ = matches!((&old.id, &new.id), (Some(a), Some(b)) if a != b);
            let has_reset = new
                .essential_properties
                .iter()
                .chain(new.supplemental_properties.iter())
                .any(|d| d.scheme_id_uri == SCHEME_RESET);
            let old_dynamic = old.kind == PresentationType::Dynamic;
            (old_dynamic
                && dynamic
                && old.availability_start_time_us != new.availability_start_time_us)
                || ids_differ
                || has_reset
        };
        self.mpd = Some(new);
        if features.patch_location {
            self.notice_once(Notice::PatchIgnored);
        }
        if features.event_stream || features.inband_event_stream {
            self.notice_once(Notice::EventStreamIgnored);
        }
        if reset || !self.remap_periods() {
            self.reset_presentation(now_us);
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
                    (None, None) => timings.get(i).and_then(|t| t.start_us) == Some(run.start_us),
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
        let switch_us = {
            let lane = &self.periods[pi].lanes[li];
            lane.current()
                .and_then(|c| c.plan.as_ref().and_then(|p| p.get(c.next_issue)))
                .map(|s| s.start_us)
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
                    rep.next_issue = plan.index_at_or_before(us).unwrap_or(0);
                    rep.joined = true;
                    rep.last_time = rep
                        .next_issue
                        .checked_sub(1)
                        .and_then(|i| plan.get(i))
                        .map(|s| s.time_ticks);
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
        match planned {
            Ok(p) => {
                let rep = &mut self.periods[pi].lanes[li].reps[ri];
                if rep.joined {
                    let bound = p.plan.len().unwrap_or(u64::MAX / 2);
                    rep.next_issue = match rep.last_time {
                        Some(t) => p.plan.leading_true(bound, |s| s.time_ticks <= t),
                        None => 0,
                    };
                }
                rep.plan = Some(p.plan);
            }
            Err(e) => {
                let rep = &mut self.periods[pi].lanes[li].reps[ri];
                rep.dead = true;
                let index = self.periods[pi].index;
                self.outputs.push_back(Output::TrackDropped {
                    period: index,
                    set: at.set,
                    rep: Some(at.rep),
                    cause: DropCause::Addressing(plan_error_name(&e)),
                });
            }
        }
    }

    fn reset_presentation(&mut self, now_us: u64) {
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
        self.inits.retain(|_, e| e.state == InitState::Ready);
        if let Some(live) = self.live.as_mut() {
            live.early.clear();
            live.missing_seen.clear();
        }
        self.on_clock_ready(now_us);
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
