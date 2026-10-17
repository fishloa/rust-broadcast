//! Per-track delivery granularity and starvation escalation.
//!
//! Field shape (Apple TV, rai-3): MP2 audio arrives in ~0.31 s bursts while
//! chunks keep arriving every <= 0.1 s with a low PDV; video is steady. With
//! the 300 ms floor target the audio track starved between bursts, every
//! ~10 s, because nothing ever raised the target: the PDV was low and a stall
//! only fed the stall memory through the PDV.
//!
//! Synthetic, 10 ms driver steps: a PCR chunk every 20 ms (PDV 0), video end
//! `t + 0.9 s` every 40 ms, audio end advancing in `burst_us` steps every
//! `burst_us`. Each scenario is mutation-checked (see the comments).

use broadcast_common::clock33::PCR_HZ;
use live_buffer::{
    BufferController, ConfigField, ControllerConfig, Error, Event, Micros, PolicyConfig, State,
    Ticks27, WRAP_27MHZ,
};

const AUDIO: u32 = 1;
const VIDEO: u32 = 2;
const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
const BASE: Ticks27 = WRAP_27MHZ - 5 * PCR_HZ;
const STEP_US: u64 = 10_000;

fn at(offset_us: u64) -> Ticks27 {
    (BASE + offset_us * TICKS_PER_US) % WRAP_27MHZ
}

/// The rai-3 config: start at the 300 ms floor so the cadence starves it.
fn cfg() -> ControllerConfig {
    cfg_with(|_| {})
}

fn cfg_with(f: impl FnOnce(&mut PolicyConfig)) -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = p.min_us;
    f(&mut p);
    ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap()
}

struct Sim {
    c: BufferController,
    t: u64,
    clock: Ticks27,
    running: bool,
    t_set: u64,
    log: Vec<(u64, Event)>,
    /// Audio burst size; 0 = steady 20 ms steps.
    burst_us: u64,
    audio_us: u64,
    next_burst: u64,
    /// Arrival period of the bursts (defaults to the burst size).
    period_us: u64,
}

impl Sim {
    fn new(cfg: ControllerConfig, burst_us: u64) -> Self {
        let mut c = BufferController::new(cfg, None).unwrap();
        let t = 1_000_000;
        c.on_chunk(t, 1000, Some(at(0)));
        c.on_track_media(t, AUDIO, at(0));
        c.on_track_media(t, VIDEO, at(0));
        Self {
            c,
            t,
            clock: at(0),
            running: false,
            t_set: 0,
            log: vec![],
            burst_us,
            audio_us: 0,
            next_burst: t + burst_us,
            period_us: burst_us,
        }
    }
    fn pos(&self) -> Ticks27 {
        if self.running {
            (self.clock + (self.t - self.t_set) * TICKS_PER_US) % WRAP_27MHZ
        } else {
            self.clock
        }
    }
    fn drain(&mut self) {
        while let Some(e) = self.c.poll_event() {
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    self.clock = media_27mhz;
                    self.t_set = self.t;
                    self.running = true;
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    self.clock = hold_at_27mhz;
                    self.running = false;
                }
                _ => {}
            }
            self.log.push((self.t, e));
        }
    }
    fn step(&mut self) {
        self.t += STEP_US;
        let rel = self.t - 1_000_000;
        if rel % 20_000 == 0 {
            self.c.on_chunk(self.t, 1000, Some(at(rel)));
        }
        if rel % 40_000 == 0 {
            self.c.on_track_media(self.t, VIDEO, at(rel + 900_000));
        }
        if self.burst_us == 0 {
            if rel % 20_000 == 0 {
                self.c.on_track_media(self.t, AUDIO, at(rel));
            }
        } else if self.t >= self.next_burst {
            self.next_burst += self.period_us;
            self.audio_us += self.burst_us;
            self.c.on_track_media(self.t, AUDIO, at(self.audio_us));
        }
        let p = Some(self.pos());
        self.c.on_tick(self.t, p);
        self.drain();
    }
    fn run(&mut self, dur_us: u64) {
        let end = self.t + dur_us;
        while self.t < end {
            self.step();
        }
    }
    fn pauses(&self) -> Vec<u64> {
        self.log
            .iter()
            .filter_map(|(t, e)| matches!(e, Event::Pause { .. }).then_some(*t))
            .collect()
    }
    fn target(&self) -> Micros {
        self.c.target_us()
    }
}

/// The ~0.31 s audio-burst shape with the floor ON: the first bursts teach
/// the step, the target rises to step + margin BEFORE start, and nothing ever
/// stalls. Mutations: dropping the floor from `need` (stalls every cycle);
/// not re-evaluating on delivery (start still fires at the 300 ms floor).
#[test]
fn rai3_burst_audio_floor_prevents_every_stall() {
    let mut s = Sim::new(cfg(), 310_000);
    s.run(60_000_000);
    assert!(s.pauses().len() <= 1, "pauses {:?}", s.pauses());
    let margin = cfg().policy.margin_us;
    assert!(s.target() >= 310_000 + margin, "target {}", s.target());
    let late: Vec<_> = s.pauses().into_iter().filter(|t| *t > 6_000_000).collect();
    assert!(late.is_empty(), "recurrence {late:?}");
    assert_eq!(s.c.state(), State::Playing);
    assert_eq!(s.c.stats().stalls as usize, s.pauses().len());
}

/// Escalation alone (floor OFF): the first DataArriving starvation is charged
/// to the stall memory, the target and resume bar ratchet, and the cadence
/// no longer starves. Mutation: remove `note_starvation` and the stalls
/// recur every burst with the target stuck at 300 ms.
#[test]
fn escalation_alone_ratchets_after_one_data_arriving_stall() {
    let mut s = Sim::new(cfg().with_track_granularity(false), 310_000);
    s.run(60_000_000);
    let p = s.pauses();
    assert_eq!(p.len(), 1, "exactly one stall then none: {p:?}");
    assert!(s.target() > 300_000, "target {}", s.target());
    // the Pause carried the post-charge resume bar (1.5 x the failed target)
    let resume = s.log.iter().find_map(|(_, e)| match e {
        Event::Pause {
            resume_target_us, ..
        } => Some(*resume_target_us),
        _ => None,
    });
    assert_eq!(resume, Some(450_000));
    assert!(s.c.stats().est.stall_peak_us >= 300_000);
}

/// Repeated stalls keep ratcheting (a cadence one step cannot cover), each
/// charged to the memory, bounded by the hard cap.
#[test]
fn escalation_ratchets_repeatedly_and_stays_under_the_cap() {
    let mut s = Sim::new(
        cfg_with(|p| p.max_us = 1_000_000).with_track_granularity(false),
        900_000,
    );
    // 900 ms of media every 920 ms: starves on every cycle until the
    // ratcheted resume bar covers the deficit
    s.period_us = 920_000;
    s.run(120_000_000);
    assert!(s.pauses().len() >= 2, "{:?}", s.pauses());
    assert!(s.target() <= 1_000_000);
    let targets: Vec<u64> = s
        .log
        .iter()
        .filter_map(|(_, e)| match e {
            Event::TargetChanged { new_us, .. } => Some(*new_us),
            _ => None,
        })
        .collect();
    assert!(targets.windows(2).any(|w| w[1] > w[0]), "{targets:?}");
}

/// The granularity decays back once the bursts stop: held for the hold, then
/// linear decay, then the 100 ms/s slew brings the target to the floor.
/// Mutation: never decaying the memory (target stays raised forever).
#[test]
fn granularity_decays_back_after_the_bursts_stop() {
    let mut s = Sim::new(cfg(), 310_000);
    s.run(20_000_000);
    assert!(s.target() >= 410_000);
    s.burst_us = 0; // audio now steady 20 ms
    s.run(10_000_000);
    assert!(s.target() >= 410_000, "held during the hold");
    s.run(80_000_000); // hold 30 s + decay 30 s + slew
    assert_eq!(s.target(), cfg().policy.min_us);
}

/// `max_us` is a hard cap whatever the cadence.
#[test]
fn max_us_cap_still_wins() {
    let mut s = Sim::new(cfg_with(|p| p.max_us = 700_000), 800_000);
    let mut at_cap = false;
    for _ in 0..3_000 {
        s.step();
        assert!(s.c.target_us() <= 700_000);
        at_cap |= s.c.target_us() == 700_000;
    }
    assert!(at_cap, "the floor should have driven the target to the cap");
    assert!(
        s.log
            .iter()
            .any(|(_, e)| matches!(e, Event::OverMax { .. }))
    );
}

/// Steady fine-grained tracks: identical to the floor switched off.
#[test]
fn steady_tracks_are_unchanged_by_the_floor() {
    let mut on = Sim::new(ControllerConfig::default(), 0);
    let mut off = Sim::new(ControllerConfig::default().with_track_granularity(false), 0);
    on.run(40_000_000);
    off.run(40_000_000);
    assert_eq!(on.log, off.log);
    assert!(on.pauses().is_empty());
    assert_eq!(on.c.target_us(), 300_000);
    assert_eq!(off.c.target_us(), 300_000);
}

/// A global data gap is PDV, not granularity: the catch-up step after a
/// 1.2 s silence must not raise the floor, while the same step with chunks
/// flowing is a cadence. Mutation: dropping the gap check lifts the first
/// target to 1.3 s.
#[test]
fn a_global_data_gap_is_not_delivery_granularity() {
    let mut c = BufferController::new(ControllerConfig::default(), None).unwrap();
    let mut t = 1_000_000u64;
    let mut rel = 0u64;
    c.on_chunk(t, 1, Some(at(0)));
    c.on_track_media(t, AUDIO, at(0));
    for _ in 0..200 {
        t += 20_000;
        rel += 20_000;
        c.on_chunk(t, 1, Some(at(rel)));
        c.on_track_media(t, AUDIO, at(rel));
    }
    assert_eq!(c.target_us(), 300_000);
    t += 1_200_000;
    rel += 1_200_000;
    c.on_chunk(t, 1, Some(at(rel)));
    c.on_track_media(t, AUDIO, at(rel));
    assert!(c.target_us() < 1_000_000, "target {}", c.target_us());

    let mut c2 = BufferController::new(ControllerConfig::default(), None).unwrap();
    let mut t2 = 1_000_000u64;
    c2.on_chunk(t2, 1, Some(at(0)));
    c2.on_track_media(t2, AUDIO, at(0));
    for i in 1..=12u64 {
        t2 += 100_000;
        c2.on_chunk(t2, 1, Some(at(i * 100_000)));
    }
    c2.on_track_media(t2, AUDIO, at(1_200_000));
    assert!(c2.target_us() >= 1_300_000, "target {}", c2.target_us());
}

/// A forward jump far beyond the arrival interval is not granularity.
/// Mutation: dropping the `max_excess` check.
#[test]
fn a_forward_jump_is_not_delivery_granularity() {
    let mut c = BufferController::new(ControllerConfig::default(), None).unwrap();
    let mut t = 1_000_000u64;
    c.on_chunk(t, 1, Some(at(0)));
    c.on_track_media(t, AUDIO, at(0));
    for i in 1..=10u64 {
        t += 20_000;
        c.on_chunk(t, 1, Some(at(i * 20_000)));
        c.on_track_media(t, AUDIO, at(i * 20_000));
    }
    t += 20_000;
    c.on_chunk(t, 1, Some(at(220_000)));
    c.on_track_media(t, AUDIO, at(220_000 + 2_500_000));
    assert_eq!(c.target_us(), 300_000);
}

/// The config knobs validate.
#[test]
fn granularity_config_validates() {
    let base = ControllerConfig::default();
    assert!(base.track_granularity);
    assert_eq!(base.granularity_hold_us, 30_000_000);
    assert_eq!(base.granularity_decay_us, 30_000_000);
    assert_eq!(
        base.with_granularity_memory_us(1, 0),
        Err(Error::InvalidConfig(ConfigField::GranularityDecay))
    );
    assert!(base.with_granularity_memory_us(0, 1).is_ok());
}
