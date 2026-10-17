//! The buffer controller: estimator + policy + state machine + events.
//!
//! **Two clock domains, kept apart.** The jitter estimator is fed the 27 MHz
//! *PCR* through [`BufferController::on_chunk`] (when a chunk carries one).
//! The *lead* is measured in the *playback (PTS) domain*: the caller reports
//! the end of the last demuxed sample with [`BufferController::on_media`] and
//! the renderer clock with [`BufferController::on_tick`], both PTS-derived
//! (`PTS90k x 300`, see [`crate::pts90k_to_27mhz`]). PCR typically runs ahead
//! of PTS by the mux/decoder delay, so a lead taken from PCR would overstate
//! what can play; every position in an [`Event`] is in the PTS domain.
//!
//! **PCR-less fallback.** If no chunk with a PCR has ever been fed, the
//! estimator is fed from `on_media` (its `media_end` is the media time). The
//! first PCR seen switches the estimator to PCR for good (its baseline is
//! reset once because the two clocks differ); from then on `on_media` never
//! feeds the estimator.

use crate::config::{ControllerConfig, Micros, REARM_DEN, REARM_NUM, TICKS_PER_US, Ticks27};
use crate::error::Error;
use crate::events::{Event, EventQueue, Hint, Reason, State, Stats};
use crate::jitter::{DeliveryJitter, SampleOutcome};
use crate::policy::Policy;
use crate::stall_memory::StallMemorySnapshot;
use broadcast_common::clock33::pcr_signed_distance;

const TICKS_PER_US_I: i64 = TICKS_PER_US as i64;

/// Signed lead in microseconds of `media_end` over `from` (PTS domain).
fn lead_between(from: Ticks27, media_end: Ticks27) -> i64 {
    pcr_signed_distance(from, media_end) / TICKS_PER_US_I
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v.max(0)).unwrap_or(0)
}

/// The decision engine. Sans-IO: the caller passes every clock reading.
#[derive(Clone, Debug)]
pub struct BufferController {
    cfg: ControllerConfig,
    jitter: DeliveryJitter,
    policy: Policy,
    events: EventQueue,
    state: State,
    reason: Reason,
    started: bool,
    neutral_pause: bool,
    last_tick: u64,
    latest: u64,
    last_chunk_at: Option<u64>,
    media_end: Option<Ticks27>,
    queue_start: Option<Ticks27>,
    last_playback: Option<Ticks27>,
    fill_from: Option<Ticks27>,
    pause_at: u64,
    source_down: bool,
    degraded: bool,
    pcr_seen: bool,
    fed_media: bool,
    below_since: Option<u64>,
    over_armed: bool,
    reported_target: Micros,
    lead_us: i64,
    stalls: u32,
    reprimes: u32,
    last_stall_depth: Micros,
    longest_freeze: Micros,
    regressions: u32,
    discontinuities: u32,
    hints: u32,
}

impl BufferController {
    /// Build a controller, optionally seeded with a persisted stall memory
    /// (age it with [`StallMemorySnapshot::aged`] first).
    ///
    /// # Errors
    /// Any [`ControllerConfig::validate`] failure.
    pub fn new(cfg: ControllerConfig, seed: Option<StallMemorySnapshot>) -> Result<Self, Error> {
        cfg.validate()?;
        let mut c = Self {
            jitter: DeliveryJitter::new(cfg.jitter)?,
            policy: Policy::new(cfg.policy),
            events: EventQueue::new(),
            state: State::Buffering,
            reason: Reason::Startup,
            started: false,
            neutral_pause: false,
            last_tick: 0,
            latest: 0,
            last_chunk_at: None,
            media_end: None,
            queue_start: None,
            last_playback: None,
            fill_from: None,
            pause_at: 0,
            source_down: false,
            degraded: false,
            pcr_seen: false,
            fed_media: false,
            below_since: None,
            over_armed: true,
            reported_target: cfg.policy.min_us,
            lead_us: 0,
            stalls: 0,
            reprimes: 0,
            last_stall_depth: 0,
            longest_freeze: 0,
            regressions: 0,
            discontinuities: 0,
            hints: 0,
            cfg,
        };
        if let Some(s) = seed {
            c.restore_memory(s);
        }
        Ok(c)
    }

    /// The validated configuration in force.
    #[must_use]
    pub fn config(&self) -> &ControllerConfig {
        &self.cfg
    }

    /// One chunk arrived. `pcr_27mhz` is the chunk's last PCR (raw, wrapped)
    /// if it has one. Call this BEFORE [`on_media`](Self::on_media) for the
    /// same chunk.
    pub fn on_chunk(&mut self, arrival_us: Micros, bytes: u32, pcr_27mhz: Option<Ticks27>) {
        self.latest = self.latest.max(arrival_us);
        self.last_chunk_at = Some(self.last_chunk_at.map_or(arrival_us, |t| t.max(arrival_us)));
        if let Some(pcr) = pcr_27mhz {
            if !self.pcr_seen {
                self.pcr_seen = true;
                if self.fed_media {
                    self.jitter.reset_baseline();
                    self.fed_media = false;
                }
            }
            self.feed(arrival_us, pcr, bytes);
        }
    }

    /// The demuxer's media position advanced: `media_end_27mhz` is the end of
    /// the last demuxed sample in the **playback (PTS) domain** (27 MHz ticks;
    /// `PTS90k x 300`). The first value after (re)start is taken as the oldest
    /// buffered position; report the first sample's *start* PTS as a
    /// zero-length first report so that sample is not skipped.
    pub fn on_media(&mut self, now_us: Micros, media_end_27mhz: Ticks27) {
        self.latest = self.latest.max(now_us);
        self.jitter.anchor(now_us);
        if !self.pcr_seen {
            self.fed_media = true;
            self.feed(now_us, media_end_27mhz, 0);
        }
        self.media_end = Some(media_end_27mhz);
        self.queue_start.get_or_insert(media_end_27mhz);
    }

    fn feed(&mut self, arrival_us: Micros, media: Ticks27, bytes: u32) {
        match self.jitter.on_chunk(arrival_us, media, bytes) {
            SampleOutcome::Accepted => self.after_sample(arrival_us),
            SampleOutcome::Discontinuity => {
                self.rebaseline(arrival_us, false);
                self.after_sample(arrival_us);
            }
            SampleOutcome::NonMonotonicArrival => {
                self.regressions = self.regressions.saturating_add(1);
            }
            SampleOutcome::Duplicate => {}
        }
    }

    fn after_sample(&mut self, now: u64) {
        let base = self.jitter.baseline_us();
        let peak = self.jitter.stall_peak_at(now);
        let need = self.policy.need_us(base, peak);
        let max = self.cfg.policy.max_us;
        if need > max {
            if self.over_armed {
                self.over_armed = false;
                self.events.push(Event::OverMax {
                    need_us: need,
                    max_us: max,
                });
            }
        } else if u128::from(need) * u128::from(REARM_DEN)
            <= u128::from(max) * u128::from(REARM_NUM)
        {
            self.over_armed = true;
        }
        let new = self.policy.follow(need, now);
        let old = self.reported_target;
        if new != old {
            let bound = new == self.cfg.policy.min_us || new == max;
            let moved = new.max(old) - new.min(old);
            if moved >= u64::from(self.cfg.jitter.bucket_us) || bound {
                self.reported_target = new;
                self.events.push(Event::TargetChanged {
                    old_us: old,
                    new_us: new,
                });
            }
        }
    }

    /// Periodic tick (recommended 50 ms). `playback_27mhz` is the renderer
    /// clock in the PTS domain, `None` while it is not running; it is only
    /// read while `Playing` (while paused the controller uses the position it
    /// told the caller to hold).
    pub fn on_tick(&mut self, now_us: Micros, playback_27mhz: Option<Ticks27>) {
        let now = if now_us < self.last_tick {
            self.regressions = self.regressions.saturating_add(1);
            self.last_tick
        } else {
            self.last_tick = now_us;
            now_us
        };
        self.latest = self.latest.max(now);
        self.jitter.anchor(now);
        if playback_27mhz.is_some() {
            self.last_playback = playback_27mhz;
        }
        let Some(media_end) = self.media_end else {
            return;
        };
        if self.state == State::Playing {
            let Some(p) = playback_27mhz else { return };
            let lead = lead_between(p, media_end);
            self.lead_us = lead;
            if lead <= i64::try_from(self.cfg.starve_threshold_us).unwrap_or(i64::MAX) {
                self.pause_starved(now, p, media_end, lead);
            } else {
                self.maybe_reprime(now, p, media_end, lead);
            }
            return;
        }
        let Some(origin) = self.fill_from.or(self.queue_start) else {
            return;
        };
        let lead = lead_between(origin, media_end);
        self.lead_us = lead;
        let need = self.exit_target(now);
        if lead >= i64::try_from(need).unwrap_or(i64::MAX) {
            self.begin_playing(now, origin, lead);
        }
    }

    fn exit_target(&self, now: u64) -> Micros {
        if !self.started {
            self.policy.start_us()
        } else if self.neutral_pause {
            self.policy.target_us()
        } else {
            self.policy.resume_us(self.jitter.stall_peak_at(now))
        }
    }

    fn begin_playing(&mut self, now: u64, origin: Ticks27, lead: i64) {
        let lead_us = to_u64(lead);
        if self.started {
            let freeze = now.saturating_sub(self.pause_at);
            self.longest_freeze = self.longest_freeze.max(freeze);
            if !self.neutral_pause {
                self.last_stall_depth = self.jitter.stall_peak_at(now);
            }
            self.events.push(Event::Resume {
                media_27mhz: origin,
                lead_us,
            });
        } else {
            self.started = true;
            self.events.push(Event::StartPlaying {
                media_27mhz: origin,
                lead_us,
            });
        }
        self.state = State::Playing;
        self.fill_from = None;
        self.neutral_pause = false;
        self.below_since = None;
    }

    fn enter_pause(
        &mut self,
        now: u64,
        hold_at: Ticks27,
        reason: Reason,
        neutral: bool,
        lead: i64,
    ) {
        self.reason = reason;
        self.neutral_pause = neutral;
        self.pause_at = now;
        self.below_since = None;
        let resume_target_us = if neutral {
            self.policy.target_us()
        } else {
            self.policy.resume_us(self.jitter.stall_peak_at(now))
        };
        self.events.push(Event::Pause {
            hold_at_27mhz: hold_at,
            reason,
            depth_us: to_u64(lead),
            resume_target_us,
        });
    }

    fn pause_starved(&mut self, now: u64, playback: Ticks27, media_end: Ticks27, lead: i64) {
        let hold_at = if lead < 0 { media_end } else { playback };
        let silent = self
            .last_chunk_at
            .is_none_or(|t| now.saturating_sub(t) >= self.cfg.nodata_after_us);
        let reason = if self.source_down || silent {
            Reason::NoData
        } else {
            Reason::DataArriving
        };
        self.stalls = self.stalls.saturating_add(1);
        self.enter_pause(now, hold_at, reason, false, lead);
        self.state = State::Starved;
        self.fill_from = Some(hold_at);
    }

    fn maybe_reprime(&mut self, now: u64, playback: Ticks27, media_end: Ticks27, lead: i64) {
        let floor = self.cfg.policy.reprime_floor_us;
        if floor == 0 {
            return;
        }
        if lead < i64::try_from(floor).unwrap_or(i64::MAX) {
            let since = *self.below_since.get_or_insert(now);
            if now.saturating_sub(since) >= self.cfg.policy.reprime_hold_us {
                let hold_at = if lead < 0 { media_end } else { playback };
                self.reprimes = self.reprimes.saturating_add(1);
                self.enter_pause(now, hold_at, Reason::Reprime, true, lead);
                self.state = State::Starved;
                self.fill_from = Some(hold_at);
            }
        } else {
            self.below_since = None;
        }
    }

    /// Report an out-of-band hint. `Reconnected` must be sent before the
    /// first chunk of the new connection is fed.
    pub fn on_hint(&mut self, now_us: Micros, hint: Hint) {
        let now = now_us.max(self.latest);
        self.latest = now;
        self.jitter.anchor(now);
        self.hints = self.hints.saturating_add(1);
        match hint {
            Hint::InputDegraded => self.degraded = true,
            Hint::InputRecovered => self.degraded = false,
            Hint::SourceEnded | Hint::SourceError => self.source_down = true,
            Hint::Reconnected => {
                self.source_down = false;
                self.rebaseline(now, true);
            }
            Hint::SyncLoss | Hint::PcrDiscontinuity => self.rebaseline(now, true),
        }
    }

    fn rebaseline(&mut self, now: u64, reset_jitter: bool) {
        if reset_jitter {
            self.jitter.reset_baseline();
        }
        self.discontinuities = self.discontinuities.saturating_add(1);
        let old_end = self.media_end.take();
        self.queue_start = None;
        match self.state {
            State::Playing => {
                let hold_at = match (self.last_playback, old_end) {
                    (Some(p), Some(e)) => {
                        if lead_between(p, e) >= 0 {
                            p
                        } else {
                            e
                        }
                    }
                    (None, Some(e)) => e,
                    (Some(p), None) => p,
                    (None, None) => 0,
                };
                let lead = self.lead_us;
                self.enter_pause(now, hold_at, Reason::Discontinuity, true, lead);
                self.state = State::Buffering;
                self.fill_from = None;
            }
            State::Starved => {
                self.state = State::Buffering;
                self.reason = Reason::Discontinuity;
                self.neutral_pause = true;
                self.fill_from = None;
            }
            State::Buffering => self.fill_from = None,
        }
        self.events.push(Event::Rebaselined);
    }

    /// Next pending event, oldest first. An `EventsDropped` precedes the
    /// survivors after an overflow.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.poll()
    }

    /// Current state.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// Current desired lead; always within `[min_us, max_us]`.
    #[must_use]
    pub fn target_us(&self) -> Micros {
        self.policy.target_us()
    }

    /// Statistics snapshot.
    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            state: self.state,
            reason: self.reason,
            lead_us: self.lead_us,
            target_us: self.policy.target_us(),
            stalls: self.stalls,
            reprimes: self.reprimes,
            last_stall_depth_us: self.last_stall_depth,
            longest_freeze_us: self.longest_freeze,
            est: self.jitter.estimate(),
            clock_regressions: self.regressions,
            discontinuities: self.discontinuities,
            hints_seen: self.hints,
            events_dropped: self.events.dropped_total(),
            input_degraded: self.degraded,
            window_overflows: self.jitter.window_overflows(),
        }
    }

    /// The stall memory, for persistence (age it on restore with
    /// [`StallMemorySnapshot::aged`]).
    #[must_use]
    pub fn snapshot_memory(&self) -> StallMemorySnapshot {
        self.jitter.snapshot_at(self.latest)
    }

    /// Install a stall memory and lift the target to what it implies.
    pub fn restore_memory(&mut self, snapshot: StallMemorySnapshot) {
        self.jitter.restore(snapshot);
        self.reseed();
    }

    fn reseed(&mut self) {
        let peak = self.jitter.stall_peak_at(self.latest);
        self.policy.raise(self.policy.need_us(0, peak));
        self.reported_target = self.policy.target_us();
    }

    /// New channel (`keep_memory = false`) or reconnect-style restart
    /// (`keep_memory = true`): back to the first fill; counters are kept.
    pub fn reset(&mut self, keep_memory: bool) {
        let kept = keep_memory.then(|| self.jitter.snapshot_at(self.latest));
        self.jitter.reset();
        self.policy.reset();
        self.events.clear();
        self.state = State::Buffering;
        self.reason = Reason::Startup;
        self.started = false;
        self.neutral_pause = false;
        self.last_chunk_at = None;
        self.media_end = None;
        self.queue_start = None;
        self.last_playback = None;
        self.fill_from = None;
        self.source_down = false;
        self.degraded = false;
        self.pcr_seen = false;
        self.fed_media = false;
        self.below_since = None;
        self.over_armed = true;
        self.lead_us = 0;
        self.reported_target = self.cfg.policy.min_us;
        if let Some(s) = kept {
            self.restore_memory(s);
        }
    }
}
