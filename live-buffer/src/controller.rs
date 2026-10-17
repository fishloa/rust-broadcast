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
//!
//! **Several tracks.** A TS mux sends video well ahead of play time and audio
//! close to real time, so the media that can be *played* ends at the least
//! advanced rendered track. The lead is therefore measured against the
//! **minimum media end over the rendered tracks**: either the caller reduces
//! them and calls [`BufferController::on_media`] with that minimum, or it
//! reports each track with [`BufferController::on_track_media`] and the
//! controller does it (see [`crate::tracks`] for the active/silent rules).

use crate::config::{ControllerConfig, Micros, REARM_DEN, REARM_NUM, TICKS_PER_US, Ticks27};
use crate::error::Error;
use crate::events::{Event, EventQueue, Hint, Reason, State, Stats};
use crate::jitter::{DeliveryJitter, SampleOutcome};
use crate::policy::Policy;
use crate::stall_memory::StallMemorySnapshot;
use crate::tracks::{DEFAULT_TRACK, Effective, StepRules, TrackSet};
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
    gap_end_at: Option<u64>,
    chunks: u64,
    tracks: TrackSet,
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
            gap_end_at: None,
            chunks: 0,
            tracks: TrackSet::new(cfg.granularity_hold_us, cfg.granularity_decay_us),
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
    ///
    /// **Delivery instant.** A chunk boundary is the delivery instant of the
    /// per-track granularity floor: every
    /// [`on_track_media`](Self::on_track_media) call for a track between two
    /// `on_chunk` calls is ONE delivery (step = end after the last call - end
    /// before the first). Reporting once per delivered chunk or once per
    /// demuxed sample gives the same floor. A caller that never calls
    /// `on_chunk` gets calls less than [`COALESCE_US`](crate::COALESCE_US)
    /// apart coalesced instead. The step is folded in (and the target
    /// re-evaluated) at the next `on_chunk`.
    pub fn on_chunk(&mut self, arrival_us: Micros, bytes: u32, pcr_27mhz: Option<Ticks27>) {
        self.latest = self.latest.max(arrival_us);
        // the previous chunk's deliveries are complete: fold their steps before
        // this chunk's arrival can mark a gap
        self.tracks.close_windows(self.step_rules());
        if self.cfg.track_granularity && self.granularity_need(arrival_us) > self.policy.target_us()
        {
            self.after_sample(arrival_us);
        }
        self.chunks = self.chunks.wrapping_add(1).max(1);
        if self
            .last_chunk_at
            .is_some_and(|t| arrival_us.saturating_sub(t) >= self.cfg.nodata_after_us)
        {
            self.gap_end_at = Some(arrival_us);
        }
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
    ///
    /// **Caller contract (join point).** The first end reported is the start
    /// origin (with several tracks, the earliest first report). Report the
    /// first end only from the join point (the first video random-access
    /// point) and never report an end for media before it: audio reported
    /// before the join point (normal in a TS mux, audio is ~1 s behind video)
    /// anchors the origin that much early and the start loses that much audio
    /// cushion. The controller cannot see RAPs, so the caller must withhold
    /// ends before the join point.
    ///
    /// `media_end_27mhz` means **the end of the media playable on ALL rendered
    /// tracks**: when audio and video are both rendered the caller must pass
    /// the *minimum* of their ends (video is typically ~1 s ahead of audio in a
    /// TS mux; judging the lead on video alone makes audio arrive after its
    /// play time). Or use [`on_track_media`](Self::on_track_media) and let the
    /// controller take the minimum.
    ///
    /// Mixing: this is shorthand for
    /// `on_track_media(now_us, DEFAULT_TRACK, media_end_27mhz)` (track 0), so
    /// it participates in the same per-track minimum as any explicit track.
    pub fn on_media(&mut self, now_us: Micros, media_end_27mhz: Ticks27) {
        self.on_track_media(now_us, DEFAULT_TRACK, media_end_27mhz);
    }

    /// Announce that `track` will be rendered, so a start or resume waits for
    /// it before its first delivery. A registered track that never delivers is
    /// excluded once other tracks have delivered for
    /// [`ControllerConfig::track_silence_timeout_us`] without it (never after a
    /// global delivery gap). Optional:
    /// an unregistered track becomes active on its first delivery (join-point
    /// contract: see [`on_media`](Self::on_media), report only from the join
    /// point). At most
    /// [`MAX_TRACKS`](crate::MAX_TRACKS) distinct tracks are kept.
    pub fn register_track(&mut self, track: u32) {
        self.tracks.register(track, self.latest);
    }

    /// Remove `track` (a replaced or dead audio/video track) so it can no
    /// longer hold back the lead: its end and any registration are forgotten
    /// at once, so the minimum is taken over the remaining tracks and a
    /// start/resume that was waiting on it is released on the next tick.
    /// [`Stats::limiting_track`] and the track named by a later `Pause` follow
    /// the remaining set. An unknown `track` is a no-op. If no track is left
    /// there is no effective end (ticks do nothing, as before the first
    /// delivery) until a track delivers or is registered again. The track may
    /// be reported again later; it then rejoins as a new one.
    pub fn unregister_track(&mut self, track: u32) {
        self.tracks.unregister(track);
    }

    /// Per-track media position: `media_end_27mhz` is the end of the last
    /// demuxed sample of `track` in the playback (PTS) domain (same units and
    /// first-report rule and join-point caller contract as
    /// [`on_media`](Self::on_media)). The controller keeps
    /// every track's end and measures the lead against the **minimum over the
    /// active tracks**. A track is excluded only when it is silent *relative to
    /// the other tracks*: its last delivery is at least
    /// [`ControllerConfig::track_silence_timeout_us`] before the most recent
    /// delivery on any track. A global delivery gap (nothing delivered
    /// anywhere) excludes nobody: the minimum of the last ends is kept, so a
    /// reconnect does not make the controller resume on stale data. A `Pause` for starvation names the
    /// limiting track in [`Event::Pause::track`] and [`Stats::limiting_track`].
    pub fn on_track_media(&mut self, now_us: Micros, track: u32, media_end_27mhz: Ticks27) {
        self.latest = self.latest.max(now_us);
        self.jitter.anchor(now_us);
        let rules = self.step_rules();
        let Some(first) = self.tracks.deliver(track, now_us, media_end_27mhz, rules) else {
            return;
        };
        if self.granularity_need(now_us) > self.policy.target_us() {
            // a coarser delivery cadence than the target covers: raise it now,
            // not at the next PCR chunk
            self.after_sample(now_us);
        }
        if !self.pcr_seen
            && let Some(e) = self.effective()
        {
            self.fed_media = true;
            self.feed(now_us, e.end, 0);
        }
        match self.queue_start {
            None => self.queue_start = Some(media_end_27mhz),
            Some(q) if first && pcr_signed_distance(q, media_end_27mhz) < 0 => {
                self.queue_start = Some(media_end_27mhz);
            }
            Some(_) => {}
        }
    }

    fn step_rules(&self) -> StepRules {
        StepRules {
            max_interval_us: self.cfg.track_silence_timeout_us,
            max_excess_us: self.cfg.nodata_after_us,
            gap_end_at: self.gap_end_at,
            epoch: self.chunks,
        }
    }

    fn effective(&self) -> Option<Effective> {
        self.tracks.effective(self.cfg.track_silence_timeout_us)
    }

    /// The limiting track, only meaningful with two or more tracks known.
    fn limiting(&self) -> Option<u32> {
        if self.tracks.len() < 2 {
            return None;
        }
        self.effective().map(|e| e.limiting)
    }

    /// Delivery-granularity floor (`step + margin`), 0 when disabled or unknown.
    fn granularity_need(&self, now: u64) -> Micros {
        if !self.cfg.track_granularity {
            return 0;
        }
        let g = self
            .tracks
            .granularity_us(now, self.cfg.track_silence_timeout_us);
        self.policy.granularity_need_us(g)
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
        let gran = if self.cfg.track_granularity {
            self.tracks
                .granularity_us(now, self.cfg.track_silence_timeout_us)
        } else {
            0
        };
        let need = self.policy.need_with_granularity_us(base, peak, gran);
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
        let Some(media_end) = self.effective().map(|e| e.end) else {
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
        track: Option<u32>,
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
            track,
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
        if reason == Reason::DataArriving {
            // Starved with data flowing (track-limited delivery, no gap): the
            // PDV never saw it, so charge the failed target to the stall memory
            // (a repeat then ratchets the target and the resume bar).
            self.jitter.note_starvation(now, self.policy.target_us());
        }
        let track = self.limiting();
        self.enter_pause(now, hold_at, reason, false, lead, track);
        if reason == Reason::DataArriving {
            // after the Pause event: the target follows the new stall memory now
            self.after_sample(now);
        }
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
                let track = self.limiting();
                self.enter_pause(now, hold_at, Reason::Reprime, true, lead, track);
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
        let old_end = self.effective().map(|e| e.end);
        self.tracks.rebaseline(now);
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
                self.enter_pause(now, hold_at, Reason::Discontinuity, true, lead, None);
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
            limiting_track: self.limiting(),
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
        self.gap_end_at = None;
        self.tracks.clear();
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
