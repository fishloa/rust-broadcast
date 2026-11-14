//! The buffer controller: estimator + policy + state machine + events.
//!
//! **Two clock domains, kept apart.** The jitter estimator is fed the 27 MHz
//! *PCR* through [`BufferController::on_chunk`] (when a chunk carries one).
//! The *lead* is measured in the *playback (PTS) domain*: the caller reports
//! the end of the last demuxed sample with [`BufferController::on_media`] and
//! the renderer clock with [`BufferController::on_tick`], both PTS-derived
//! (`PTS90k x 300`, see [`crate::pts90k_to_clock27`]). PCR typically runs ahead
//! of PTS by the mux/decoder delay, so a lead taken from PCR would overstate
//! what can play; every position in an [`Event`] is in the PTS domain.
//!
//! **PCR-less fallback.** If no chunk with a PCR has ever been fed, the
//! estimator is fed from `on_media` (its `media_end` is the media time), one
//! track's own series: the first track to deliver, until it is removed or
//! silent. Never the minimum over tracks, which jumps whenever a track joins
//! behind or the limiting track changes. The first PCR seen switches the estimator to PCR for good (its baseline is
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

use core::time::Duration;

use crate::config::{ControllerConfig, REARM_DEN, REARM_NUM, TICKS_PER_US};
use crate::error::Error;
use crate::events::{Event, EventQueue, Hint, Reason, State, Stats};
use crate::jitter::{DeliveryJitter, SampleOutcome};
use crate::policy::Policy;
use crate::stall_memory::StallMemorySnapshot;
use crate::time::{Clock27, duration_between, duration_micros, micros_as_duration};
use crate::tracks::{DEFAULT_TRACK, Effective, StepRules, TrackSet};

fn dur(us: u64) -> Duration {
    Duration::from_micros(us)
}
use broadcast_common::clock33::pcr_signed_distance;

const TICKS_PER_US_I: i64 = TICKS_PER_US as i64;

/// Signed lead in microseconds of `media_end` over `from` (PTS domain).
fn lead_between(from: Clock27, media_end: Clock27) -> i64 {
    pcr_signed_distance(from.ticks(), media_end.ticks()) / TICKS_PER_US_I
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
    last_tick: Duration,
    latest: Duration,
    last_chunk_at: Option<Duration>,
    gap_end_at: Option<Duration>,
    chunks: u64,
    tracks: TrackSet,
    queue_start: Option<Clock27>,
    last_playback: Option<Clock27>,
    fill_from: Option<Clock27>,
    pause_at: Duration,
    source_down: bool,
    degraded: bool,
    pcr_seen: bool,
    fed_media: bool,
    media_ref: Option<u32>,
    below_since: Option<Duration>,
    over_armed: bool,
    reported_target: u64,
    lead: i64,
    stalls: u32,
    reprimes: u32,
    last_stall_depth: Duration,
    longest_freeze: Duration,
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
            last_tick: Duration::ZERO,
            latest: Duration::ZERO,
            last_chunk_at: None,
            gap_end_at: None,
            chunks: 0,
            tracks: TrackSet::new(cfg.granularity_hold, cfg.granularity_decay),
            queue_start: None,
            last_playback: None,
            fill_from: None,
            pause_at: Duration::ZERO,
            source_down: false,
            degraded: false,
            pcr_seen: false,
            fed_media: false,
            media_ref: None,
            below_since: None,
            over_armed: true,
            reported_target: duration_micros(cfg.policy.min),
            lead: 0,
            stalls: 0,
            reprimes: 0,
            last_stall_depth: Duration::ZERO,
            longest_freeze: Duration::ZERO,
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
    /// `on_chunk` gets calls less than [`COALESCE`](crate::COALESCE)
    /// apart coalesced instead. The step is folded in (and the target
    /// re-evaluated) at the next `on_chunk`.
    pub fn on_chunk(&mut self, arrival: Duration, bytes: u32, pcr_27mhz: Option<Clock27>) {
        self.latest = self.latest.max(arrival);
        // the previous chunk's deliveries are complete: fold their steps before
        // this chunk's arrival can mark a gap
        self.tracks.close_windows(self.step_rules());
        if self.cfg.track_granularity && self.granularity_need(arrival) > self.policy.target() {
            self.after_sample(arrival);
        }
        self.chunks = self.chunks.wrapping_add(1).max(1);
        if self
            .last_chunk_at
            .is_some_and(|t| duration_between(t, arrival) >= self.cfg.nodata_after)
        {
            self.gap_end_at = Some(arrival);
        }
        self.last_chunk_at = Some(self.last_chunk_at.map_or(arrival, |t| t.max(arrival)));
        if let Some(pcr) = pcr_27mhz {
            if !self.pcr_seen {
                self.pcr_seen = true;
                if self.fed_media {
                    self.jitter.reset_baseline();
                    self.fed_media = false;
                }
            }
            self.feed(arrival, pcr, bytes);
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
    /// `on_track_media(now, DEFAULT_TRACK, media_end)` (track 0), so
    /// it participates in the same per-track minimum as any explicit track.
    pub fn on_media(&mut self, now: Duration, media_end: Clock27) {
        self.on_track_media(now, DEFAULT_TRACK, media_end);
    }

    /// Announce that `track` will be rendered, so a start or resume waits for
    /// it before its first delivery. A registered track that never delivers is
    /// excluded once other tracks have delivered for
    /// [`ControllerConfig::track_silence_timeout`] without it (never after a
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
    /// [`ControllerConfig::track_silence_timeout`] before the most recent
    /// delivery on any track. A global delivery gap (nothing delivered
    /// anywhere) excludes nobody: the minimum of the last ends is kept, so a
    /// reconnect does not make the controller resume on stale data. A `Pause` for starvation names the
    /// limiting track in [`Event::Pause::track`] and [`Stats::limiting_track`].
    ///
    /// A `now` earlier than the latest instant seen on any call is clamped to it, as
    /// [`on_hint`](Self::on_hint) does, so a stepped-back clock cannot stamp a delivery in the past.
    pub fn on_track_media(&mut self, now: Duration, track: u32, media_end: Clock27) {
        let now = now.max(self.latest);
        self.latest = now;
        self.jitter.anchor(now);
        let rules = self.step_rules();
        let Some(first) = self.tracks.deliver(track, now, media_end, rules) else {
            return;
        };
        if self.granularity_need(now) > self.policy.target() {
            // a coarser delivery cadence than the target covers: raise it now,
            // not at the next PCR chunk
            self.after_sample(now);
        }
        if !self.pcr_seen && self.effective().is_some() && self.feeds_estimator(track) {
            self.fed_media = true;
            self.feed(now, media_end, 0);
        }
        match self.queue_start {
            None => self.queue_start = Some(media_end),
            Some(q) if first && pcr_signed_distance(q.ticks(), media_end.ticks()) < 0 => {
                self.queue_start = Some(media_end);
            }
            Some(_) => {}
        }
    }

    /// A track taking over the PCR-less feed starts a fresh baseline: its
    /// media clock is offset from the previous track's.
    fn feeds_estimator(&mut self, track: u32) -> bool {
        if self.media_ref == Some(track) {
            return true;
        }
        if let Some(r) = self.media_ref {
            if self
                .tracks
                .is_live(r, self.latest, self.cfg.track_silence_timeout)
            {
                return false;
            }
            if self.fed_media {
                self.jitter.reset_baseline();
            }
        }
        self.media_ref = Some(track);
        true
    }

    fn step_rules(&self) -> StepRules {
        StepRules {
            max_interval: self.cfg.track_silence_timeout,
            max_excess: self.cfg.nodata_after,
            gap_end_at: self.gap_end_at,
            epoch: self.chunks,
        }
    }

    fn effective(&self) -> Option<Effective> {
        self.tracks.effective(
            self.latest,
            self.cfg.track_silence_timeout,
            self.state != State::Playing,
        )
    }

    /// The limiting track, only meaningful with two or more tracks known.
    fn limiting(&self) -> Option<u32> {
        if self.tracks.len() < 2 {
            return None;
        }
        self.effective().map(|e| e.limiting)
    }

    /// Delivery-granularity floor (`step + margin`), 0 when disabled or unknown.
    fn granularity_need(&self, now: Duration) -> Duration {
        if !self.cfg.track_granularity {
            return Duration::ZERO;
        }
        let g = self.tracks.granularity(now, self.cfg.track_silence_timeout);
        micros_as_duration(self.policy.granularity_need_us(duration_micros(g)))
    }

    fn feed(&mut self, arrival: Duration, media: Clock27, bytes: u32) {
        match self.jitter.on_chunk(arrival, media, bytes) {
            SampleOutcome::Accepted => self.after_sample(arrival),
            SampleOutcome::Discontinuity => {
                self.rebaseline(arrival, false);
                self.after_sample(arrival);
            }
            SampleOutcome::NonMonotonicArrival => {
                self.regressions = self.regressions.saturating_add(1);
            }
            SampleOutcome::Duplicate => {}
        }
    }

    fn after_sample(&mut self, now: Duration) {
        let base = duration_micros(self.jitter.baseline());
        let peak = duration_micros(self.jitter.stall_peak_at(now));
        let gran = if self.cfg.track_granularity {
            duration_micros(self.tracks.granularity(now, self.cfg.track_silence_timeout))
        } else {
            0
        };
        let need = self.policy.need_with_granularity_us(base, peak, gran);
        let max = duration_micros(self.cfg.policy.max);
        if need > max {
            if self.over_armed {
                self.over_armed = false;
                self.events.push(Event::OverMax {
                    need: dur(need),
                    max: dur(max),
                });
            }
        } else if u128::from(need) * u128::from(REARM_DEN)
            <= u128::from(max) * u128::from(REARM_NUM)
        {
            self.over_armed = true;
        }
        let new = self.policy.follow(need, duration_micros(now));
        let old = self.reported_target;
        if new != old {
            let bound = new == duration_micros(self.cfg.policy.min) || new == max;
            let moved = new.max(old) - new.min(old);
            if moved >= duration_micros(self.cfg.jitter.bucket) || bound {
                self.reported_target = new;
                self.events.push(Event::TargetChanged {
                    old: dur(old),
                    new: dur(new),
                });
            }
        }
    }

    /// Periodic tick (recommended 50 ms). `playback_27mhz` is the renderer
    /// clock in the PTS domain, `None` while it is not running; it is only
    /// read while `Playing` (while paused the controller uses the position it
    /// told the caller to hold).
    pub fn on_tick(&mut self, now: Duration, playback_27mhz: Option<Clock27>) {
        let now = if now < self.last_tick {
            self.regressions = self.regressions.saturating_add(1);
            self.last_tick
        } else {
            self.last_tick = now;
            now
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
            self.lead = lead;
            if lead <= i64::try_from(duration_micros(self.cfg.starve_threshold)).unwrap_or(i64::MAX)
            {
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
        self.lead = lead;
        let need = duration_micros(self.exit_target(now));
        if lead >= i64::try_from(need).unwrap_or(i64::MAX) && self.granularity_ready(now) {
            self.begin_playing(now, origin, lead);
        }
    }

    /// Start/resume gate on the delivery granularity (see the module docs of
    /// [`crate::tracks`]): every delivering track must have shown enough
    /// deliveries to know its cadence, bounded by the start fill so a slow
    /// track cannot block forever.
    fn granularity_ready(&self, now: Duration) -> bool {
        !self.cfg.track_granularity
            || self.tracks.granularity_known(
                now,
                self.cfg.track_silence_timeout,
                self.policy.start(),
            )
    }

    fn exit_target(&self, now: Duration) -> Duration {
        if !self.started {
            self.policy.start()
        } else if self.neutral_pause {
            self.policy.target()
        } else {
            self.policy.resume(self.jitter.stall_peak_at(now))
        }
    }

    fn begin_playing(&mut self, now: Duration, origin: Clock27, lead: i64) {
        let lead = dur(to_u64(lead));
        if self.started {
            let freeze = now.saturating_sub(self.pause_at);
            self.longest_freeze = self.longest_freeze.max(freeze);
            if !self.neutral_pause {
                self.last_stall_depth = self.jitter.stall_peak_at(now);
            }
            self.events.push(Event::Resume {
                media_27mhz: origin,
                lead,
            });
        } else {
            self.started = true;
            self.events.push(Event::StartPlaying {
                media_27mhz: origin,
                lead,
            });
        }
        self.state = State::Playing;
        self.fill_from = None;
        self.neutral_pause = false;
        self.below_since = None;
    }

    fn enter_pause(
        &mut self,
        now: Duration,
        hold_at: Clock27,
        reason: Reason,
        neutral: bool,
        lead: i64,
        track: Option<u32>,
    ) {
        self.reason = reason;
        self.neutral_pause = neutral;
        self.pause_at = now;
        self.below_since = None;
        let resume_target = if neutral {
            self.policy.target()
        } else {
            self.policy.resume(self.jitter.stall_peak_at(now))
        };
        self.events.push(Event::Pause {
            hold_at_27mhz: hold_at,
            reason,
            depth: dur(to_u64(lead)),
            resume_target,
            track,
        });
    }

    fn pause_starved(&mut self, now: Duration, playback: Clock27, media_end: Clock27, lead: i64) {
        let hold_at = if lead < 0 { media_end } else { playback };
        let silent = self
            .last_chunk_at
            .is_none_or(|t| duration_between(t, now) >= self.cfg.nodata_after);
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
            self.jitter.note_starvation(now, self.policy.target());
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

    fn maybe_reprime(&mut self, now: Duration, playback: Clock27, media_end: Clock27, lead: i64) {
        let floor = self.cfg.policy.reprime_floor;
        if floor == Duration::ZERO {
            return;
        }
        if lead < i64::try_from(duration_micros(floor)).unwrap_or(i64::MAX) {
            let since = *self.below_since.get_or_insert(now);
            if duration_between(since, now) >= self.cfg.policy.reprime_hold {
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
    pub fn on_hint(&mut self, now: Duration, hint: Hint) {
        let now = now.max(self.latest);
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

    fn rebaseline(&mut self, now: Duration, reset_jitter: bool) {
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
                    (None, None) => Clock27::from_27mhz(0).expect("zero is on the clock"),
                };
                let lead = self.lead;
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

    /// Current desired lead; always within `[min, max]`.
    #[must_use]
    pub fn target(&self) -> Duration {
        self.policy.target()
    }

    /// Statistics snapshot.
    #[must_use]
    pub fn stats(&self) -> Stats {
        let lead = dur(to_u64(self.lead));
        Stats {
            state: self.state,
            reason: self.reason,
            lead,
            target: self.policy.target(),
            stalls: self.stalls,
            reprimes: self.reprimes,
            last_stall_depth: self.last_stall_depth,
            longest_freeze: self.longest_freeze,
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
        let peak = duration_micros(self.jitter.stall_peak_at(self.latest));
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
        self.media_ref = None;
        self.below_since = None;
        self.over_armed = true;
        self.lead = 0;
        self.reported_target = duration_micros(self.cfg.policy.min);
        if let Some(s) = kept {
            self.restore_memory(s);
        }
    }
}
