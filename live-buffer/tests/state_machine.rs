//! The state machine, policy, configuration and event queue, driven by hand.
//! DESIGN.md section 2.4: every state x input cell is asserted.

mod support;

use live_buffer::policy::Policy;
use live_buffer::{
    BufferController, ConfigField, ControllerConfig, Error, Event, Hint, JitterConfig,
    PolicyConfig, Reason, StallMemorySnapshot, State, WRAP_27MHZ,
};
use support::*;

const STEP_US: u64 = 35_000;
const STEP_TICKS: u64 = STEP_US * TICKS_PER_US;

/// A hand-driven caller: steady 35 ms chunks, an obedient renderer.
struct Rig {
    c: BufferController,
    now: u64,
    media: u64,
    running: bool,
    clock: u64,
    t_set: u64,
    log: Vec<Event>,
}

impl Rig {
    fn new(cfg: ControllerConfig) -> Self {
        Self {
            c: BufferController::new(cfg, None).unwrap(),
            now: 0,
            media: 27_000_000,
            running: false,
            clock: 0,
            t_set: 0,
            log: vec![],
        }
    }
    fn default() -> Self {
        Self::new(ControllerConfig::default())
    }
    fn pos(&self) -> u64 {
        if self.running {
            (self.clock + (self.now - self.t_set) * TICKS_PER_US) % WRAP_27MHZ
        } else {
            self.clock
        }
    }
    fn drain(&mut self) -> Vec<Event> {
        let mut v = vec![];
        while let Some(e) = self.c.poll_event() {
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    self.clock = media_27mhz;
                    self.t_set = self.now;
                    self.running = true;
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    self.clock = hold_at_27mhz;
                    self.running = false;
                }
                _ => {}
            }
            self.log.push(e);
            v.push(e);
        }
        v
    }
    /// `n` chunks, each 35 ms apart in arrival and media.
    fn feed(&mut self, n: u32) {
        for _ in 0..n {
            self.now += STEP_US;
            self.media += STEP_TICKS;
            self.c
                .on_chunk(self.now, 1316, Some(self.media % WRAP_27MHZ));
            self.c.on_media(self.now, self.media % WRAP_27MHZ);
        }
    }
    fn tick(&mut self) -> Vec<Event> {
        let p = (self.running || self.clock != 0).then(|| self.pos());
        self.c.on_tick(self.now, p);
        self.drain()
    }
    fn advance(&mut self, us: u64) {
        self.now += us;
    }
    fn lead_us(&self) -> i64 {
        self.c.stats().lead_us
    }

    // ---- reach each state
    fn buffering_startup() -> Self {
        let mut r = Self::default();
        r.feed(10);
        assert!(r.tick().is_empty());
        assert_eq!(r.c.state(), State::Buffering);
        r
    }
    fn playing() -> Self {
        let mut r = Self::default();
        r.feed(60);
        let ev = r.tick();
        assert!(matches!(ev[..], [Event::StartPlaying { .. }]), "{ev:?}");
        r
    }
    /// Playing then no data until the lead is gone.
    fn starved() -> Self {
        let mut r = Self::playing();
        let mut guard = 0;
        while r.c.state() == State::Playing {
            r.advance(50_000);
            r.tick();
            guard += 1;
            assert!(guard < 200);
        }
        assert_eq!(r.c.state(), State::Starved);
        r
    }
    fn buffering_discontinuity() -> Self {
        let mut r = Self::playing();
        r.c.on_hint(r.now, Hint::Reconnected);
        r.drain();
        assert_eq!(r.c.state(), State::Buffering);
        assert_eq!(r.c.stats().reason, Reason::Discontinuity);
        r
    }
}

/// Only the decisions a renderer must act on (drops TargetChanged etc.).
fn decisions(ev: &[Event]) -> Vec<Event> {
    ev.iter()
        .copied()
        .filter(|e| {
            matches!(
                e,
                Event::StartPlaying { .. } | Event::Pause { .. } | Event::Resume { .. }
            )
        })
        .collect()
}

fn kinds(ev: &[Event]) -> Vec<&'static str> {
    ev.iter().map(Event::name).collect()
}

#[test]
fn initial_target_default_is_the_owner_approved_1800_ms() {
    assert_eq!(PolicyConfig::DEFAULT.initial_us, 1_800_000);
    assert_eq!(ControllerConfig::default(), ControllerConfig::DEFAULT);
    assert_eq!(PolicyConfig::default(), PolicyConfig::DEFAULT);
    assert_eq!(JitterConfig::default(), JitterConfig::DEFAULT);
}

// ------------------------------------------------- transition table, every cell

#[test]
fn table_buffering_startup() {
    // lead <= threshold: stay
    let mut r = Rig::default();
    r.feed(1);
    assert!(r.tick().is_empty());
    assert_eq!(r.c.state(), State::Buffering);
    // otherwise: stay
    let mut r = Rig::buffering_startup();
    assert!(r.tick().is_empty());
    // lead >= exit target: -> Playing, StartPlaying
    r.feed(45);
    let ev = r.tick();
    assert!(matches!(ev[..], [Event::StartPlaying { .. }]), "{ev:?}");
    assert_eq!(r.c.state(), State::Playing);
    // chunk: update, no state change
    let mut r = Rig::buffering_startup();
    r.feed(1);
    assert_eq!(r.c.state(), State::Buffering);
    // hints
    for h in [
        Hint::InputDegraded,
        Hint::InputRecovered,
        Hint::SourceEnded,
        Hint::SourceError,
    ] {
        let mut r = Rig::buffering_startup();
        r.c.on_hint(r.now, h);
        assert_eq!(r.c.state(), State::Buffering, "{h}");
        assert!(r.drain().is_empty(), "{h}");
    }
    for h in [Hint::Reconnected, Hint::SyncLoss, Hint::PcrDiscontinuity] {
        let mut r = Rig::buffering_startup();
        r.c.on_hint(r.now, h);
        let ev = r.drain();
        assert_eq!(
            kinds(&ev),
            ["rebaselined"],
            "{h}: restart fill, no Pause (nothing was playing)"
        );
        assert_eq!(r.c.state(), State::Buffering);
        assert_eq!(r.c.stats().reason, Reason::Startup);
        // the fill restarts: the old lead no longer counts
        r.feed(10);
        assert!(r.tick().is_empty());
        r.feed(50);
        assert!(matches!(r.tick()[..], [Event::StartPlaying { .. }]));
    }
}

#[test]
fn table_playing() {
    // lead <= th: -> Starved + Pause (reason DataArriving if chunks are still arriving)
    let mut r = Rig::playing();
    r.running = true;
    // clock overruns the media by 1 ms (still within th): chunks were arriving 5 ms ago
    r.feed(1);
    let media_end = r.media % WRAP_27MHZ;
    let ev = {
        r.c.on_tick(r.now + 5_000, Some(media_end - 10_000 * TICKS_PER_US));
        r.drain()
    };
    match ev[..] {
        [
            Event::Pause {
                reason,
                hold_at_27mhz,
                ..
            },
        ] => {
            assert_eq!(reason, Reason::DataArriving);
            assert_eq!(hold_at_27mhz, media_end - 10_000 * TICKS_PER_US);
        }
        _ => panic!("{ev:?}"),
    }
    assert_eq!(r.c.state(), State::Starved);
    // silent for >= nodata_after: NoData
    let mut r = Rig::playing();
    r.advance(300_000);
    let p = r.media % WRAP_27MHZ;
    r.c.on_tick(r.now, Some(p));
    match r.drain()[..] {
        [Event::Pause { reason, .. }] => assert_eq!(reason, Reason::NoData),
        ref e => panic!("{e:?}"),
    }
    // the clock ran AHEAD of the data: hold_at is clamped to the data
    let mut r = Rig::playing();
    r.advance(400_000);
    let ahead = (r.media + 5_000_000 * TICKS_PER_US) % WRAP_27MHZ;
    r.c.on_tick(r.now, Some(ahead));
    match r.drain()[..] {
        [
            Event::Pause {
                hold_at_27mhz,
                depth_us,
                ..
            },
        ] => {
            assert_eq!(
                hold_at_27mhz,
                r.media % WRAP_27MHZ,
                "never ahead of the data"
            );
            assert_eq!(depth_us, 0);
        }
        ref e => panic!("{e:?}"),
    }
    // lead >= target / otherwise: stay, no event; no Resume while Playing
    let mut r = Rig::playing();
    for _ in 0..5 {
        r.feed(2);
        assert!(r.tick().is_empty());
        assert_eq!(r.c.state(), State::Playing);
    }
    // chunks: no state change
    let mut r = Rig::playing();
    r.feed(3);
    assert_eq!(r.c.state(), State::Playing);
    // non-authoritative hints
    for h in [
        Hint::InputDegraded,
        Hint::InputRecovered,
        Hint::SourceEnded,
        Hint::SourceError,
    ] {
        let mut r = Rig::playing();
        r.c.on_hint(r.now, h);
        assert_eq!(r.c.state(), State::Playing, "{h} must not change state");
        assert!(r.drain().is_empty(), "{h}");
        assert!(r.tick().is_empty());
    }
    // authoritative: -> Buffering(Discontinuity) with Pause at hold_at
    for h in [Hint::Reconnected, Hint::SyncLoss, Hint::PcrDiscontinuity] {
        let mut r = Rig::playing();
        r.advance(100_000);
        let clock_now = r.pos();
        r.tick();
        r.c.on_hint(r.now, h);
        let ev = r.drain();
        assert_eq!(kinds(&ev), ["pause", "rebaselined"], "{h}");
        match ev[0] {
            Event::Pause {
                reason,
                hold_at_27mhz,
                ..
            } => {
                assert_eq!(reason, Reason::Discontinuity);
                assert_eq!(hold_at_27mhz, clock_now);
            }
            _ => unreachable!(),
        }
        assert_eq!(r.c.state(), State::Buffering);
    }
}

#[test]
fn table_starved() {
    // lead <= th: stay Starved, no repeat Pause
    let mut r = Rig::starved();
    for _ in 0..5 {
        r.advance(50_000);
        assert!(r.tick().is_empty());
        assert_eq!(r.c.state(), State::Starved);
    }
    // lead between th and the exit bar: stay (hysteresis, see below)
    // lead >= exit bar: -> Playing, Resume at the held position. The silence that
    // starved it was ~1.8 s, so the stall memory asks for 1.5 x that (clamped at the 3 s cap).
    let mut r = Rig::starved();
    let hold = r.clock;
    r.feed(95);
    let ev = decisions(&r.tick());
    match ev[..] {
        [
            Event::Resume {
                media_27mhz,
                lead_us,
            },
        ] => {
            assert_eq!(
                media_27mhz, hold,
                "Resume carries the media time to restart from"
            );
            assert!(lead_us >= 2_700_000);
        }
        _ => panic!("{ev:?}"),
    }
    assert_eq!(r.c.state(), State::Playing);
    // chunk: no state change
    let mut r = Rig::starved();
    r.feed(1);
    assert_eq!(r.c.state(), State::Starved);
    // non-authoritative hints
    for h in [
        Hint::InputDegraded,
        Hint::InputRecovered,
        Hint::SourceEnded,
        Hint::SourceError,
    ] {
        let mut r = Rig::starved();
        r.c.on_hint(r.now, h);
        assert_eq!(r.c.state(), State::Starved, "{h}");
        assert!(r.drain().is_empty(), "{h}");
    }
    // authoritative: -> Buffering(Discontinuity), already paused so no second Pause
    for h in [Hint::Reconnected, Hint::SyncLoss, Hint::PcrDiscontinuity] {
        let mut r = Rig::starved();
        r.c.on_hint(r.now, h);
        assert_eq!(kinds(&r.drain()), ["rebaselined"], "{h}");
        assert_eq!(r.c.state(), State::Buffering);
        assert_eq!(r.c.stats().reason, Reason::Discontinuity);
    }
}

#[test]
fn table_buffering_discontinuity() {
    let mut r = Rig::buffering_discontinuity();
    // no data on the new timeline yet: stay
    r.advance(50_000);
    assert!(r.tick().is_empty());
    // more hints: stay, another Rebaselined only
    r.c.on_hint(r.now, Hint::SyncLoss);
    assert_eq!(kinds(&r.drain()), ["rebaselined"]);
    // refill at the TARGET (not after_rebuffer): Resume at the new timeline's first media
    r.media += 99_000_000 * TICKS_PER_US; // new timeline, far away
    let first = r.media % WRAP_27MHZ;
    r.feed(1);
    let first_end = r.media % WRAP_27MHZ;
    r.feed(10);
    let ev = r.tick();
    match ev[..] {
        [Event::Resume { media_27mhz, .. }] => {
            assert_eq!(
                media_27mhz, first_end,
                "Resume at the oldest media of the new timeline"
            );
            assert_ne!(media_27mhz, first);
        }
        _ => panic!("{ev:?}"),
    }
}

// ------------------------------------------------------------------ behaviour

#[test]
fn hysteresis_exit_only_at_the_after_rebuffer_bar() {
    // a 1.4 s gap makes the stall peak ~1.4 s: the exit bar is 1.5 x that
    let mut r = Rig::new({
        let mut c = ControllerConfig::default();
        c.policy.initial_us = 500_000;
        c
    });
    r.feed(30);
    assert!(matches!(r.tick()[..], [Event::StartPlaying { .. }]));
    r.feed(10);
    r.tick();
    r.advance(1_400_000); // the gap; the clock runs on
    r.tick();
    assert_eq!(r.c.state(), State::Starved);
    r.feed(1); // first post-gap chunk reveals the depth
    let peak = r.c.stats().est.stall_peak_us;
    assert!(peak >= 1_300_000);
    let bar = peak * 384 / 256;
    let mut resumed_at = None;
    for i in 0..200 {
        r.feed(1);
        let ev = r.tick();
        let lead = r.lead_us();
        if let Some(Event::Resume { .. }) = ev.first() {
            resumed_at = Some((i, lead));
            break;
        }
        assert_eq!(
            r.c.state(),
            State::Starved,
            "lead {lead} below the bar {bar}: no exit"
        );
        assert!(lead < bar as i64);
    }
    let (_, lead) = resumed_at.expect("resumes");
    assert!(lead >= bar as i64, "resumed at {lead} below the bar {bar}");
    // and the lead hovering around the ENTER threshold cannot flap a Playing state
    // back out: once Starved, a lead of 100 ms (> 20 ms) is not an exit
}

#[test]
fn no_flapping_between_the_thresholds() {
    let mut r = Rig::starved();
    let hold = r.clock;
    // lead 100 ms: above the 20 ms enter threshold, below the exit bar (1.5 x the stall that starved it)
    r.feed(3);
    assert!(decisions(&r.tick()).is_empty());
    assert_eq!(r.c.state(), State::Starved);
    assert_eq!(r.clock, hold);
    // still no Pause/Resume pair from repeated ticks at that lead
    for _ in 0..10 {
        r.advance(1_000);
        assert!(decisions(&r.tick()).is_empty());
    }
}

#[test]
fn startup_suppression_gap_during_the_first_fill() {
    // servustv shape: a 1.33 s gap 0.49 s into the stream, while still filling
    let mut r = Rig::default();
    r.feed(14);
    r.advance(1_330_000);
    let mut lead = None;
    for _ in 0..120 {
        r.feed(1);
        for e in r.tick() {
            if let Event::StartPlaying { lead_us, .. } = e {
                lead = Some(lead_us);
            }
        }
        if lead.is_some() {
            break;
        }
    }
    let lead = lead.expect("started");
    let s = r.c.stats();
    assert_eq!(s.stalls, 0, "the first fill is never a stall");
    assert_eq!(s.longest_freeze_us, 0);
    // never the after_rebuffer escalation (1.5 x 1.33 s = 2.0 s): the start
    // threshold is max(initial, target) = 1.8 s here
    assert!(lead < 2_000_000, "start lead {lead}");
    // ...but the gap fed the stall memory: the target rose
    assert!(s.target_us >= 1_550_000, "target {}", s.target_us);
    // a lower initial shows the memory raising the START target
    let mut cfg = ControllerConfig::default();
    cfg.policy.initial_us = 500_000;
    let mut r = Rig::new(cfg);
    r.feed(14);
    r.advance(1_330_000);
    for _ in 0..120 {
        r.feed(1);
        if let Some(Event::StartPlaying { lead_us, .. }) = r.tick().first().copied() {
            assert!(
                lead_us >= 1_550_000,
                "start target raised by the startup gap: {lead_us}"
            );
            return;
        }
    }
    panic!("never started");
}

#[test]
fn a_gap_while_paused_leaves_no_permanent_starvation() {
    let mut cfg = ControllerConfig::default();
    cfg.policy.initial_us = 500_000;
    let mut s = Synth::new(30 * SEC);
    // first gap stalls the player; the second 1.5 s gap hits while it refills
    s.gaps = vec![(8 * SEC, 1_400_000), (11 * SEC, 1_500_000)];
    let mut o = Opts::new(500_000);
    o.cfg = cfg;
    let r = replay(&s.generate(), &o);
    let p = r.pauses();
    assert_eq!(p.len(), 1, "exactly one pause: {p:?}");
    assert!(p[0].1.is_some(), "resumed");
    assert_eq!(r.ctl.state(), State::Playing);
    // after the resume the lead is at least the resume bar's worth and never starves again
    let freeze = p[0].1.unwrap() - p[0].0;
    assert!(
        freeze > 2_500_000,
        "refill waited out the second gap too: {freeze}"
    );
    assert!(r.min_lead_playing.unwrap() > 20_000);
}

#[test]
fn resume_carries_hold_position_for_a_stall_and_the_new_media_for_a_discontinuity() {
    let mut s = Synth::new(40 * SEC);
    s.gaps = vec![(8 * SEC, 1_400_000)];
    s.reconnect = Some((25 * SEC, 3_400_000, 1_300_000));
    let r = replay(&s.generate(), &Opts::new(500_000));
    let pauses: Vec<&Event> = r.events.iter().map(|(_, e)| e).collect();
    let mut holds = vec![];
    let mut resumes = vec![];
    for e in pauses {
        match e {
            Event::Pause {
                hold_at_27mhz,
                reason,
                ..
            } => holds.push((*hold_at_27mhz, *reason)),
            Event::Resume { media_27mhz, .. } => resumes.push(*media_27mhz),
            _ => {}
        }
    }
    assert!(
        holds.len() >= 2 && resumes.len() == holds.len(),
        "{holds:?} {resumes:?}"
    );
    assert_eq!(resumes[0], holds[0].0, "stall: Resume == hold_at");
    // the reconnect: the last resume is on the NEW timeline, ahead of the old hold
    let last_hold = holds.last().unwrap().0;
    let last_resume = *resumes.last().unwrap();
    assert_ne!(
        last_resume, last_hold,
        "discontinuity: restart from the new media"
    );
}

#[test]
fn reconnect_keeps_the_stall_memory_and_resumes_at_the_target() {
    let mut s = Synth::new(60 * SEC);
    s.gaps = vec![(8 * SEC, 1_400_000)];
    s.reconnect = Some((30 * SEC, 3_400_000, 1_300_000));
    let r = replay(&s.generate(), &Opts::new(1_800_000));
    assert_eq!(r.count(|e| matches!(e, Event::Rebaselined)), 1);
    let st = r.ctl.stats();
    // the 1.4 s stall memory is alive after the reconnect, the 3.4 s dead time did not replace it
    assert!(
        st.est.stall_peak_us >= 1_350_000 && st.est.stall_peak_us < 1_600_000,
        "{}",
        st.est.stall_peak_us
    );
    // the resume after the reconnect used the target, not 1.5 x depth
    let resume_lead = r.events.iter().rev().find_map(|(_, e)| match e {
        Event::Resume { lead_us, .. } => Some(*lead_us),
        _ => None,
    });
    let l = resume_lead.expect("resume after reconnect");
    assert!(
        l < 2_000_000,
        "resume lead {l} must be the target, not the 2.1 s stall bar"
    );
    assert_eq!(st.state, State::Playing);
}

#[test]
fn reconnect_without_the_hint_is_still_caught_by_the_forward_jump_rule() {
    let mut s = Synth::new(60 * SEC);
    s.reconnect = Some((30 * SEC, 3_400_000, 1_300_000));
    let mut o = Opts::new(1_800_000);
    o.send_hints = false;
    let r = replay(&s.generate(), &o);
    assert_eq!(r.count(|e| matches!(e, Event::Rebaselined)), 1);
    assert_eq!(r.ctl.stats().discontinuities, 1);
}

#[test]
fn input_degraded_is_recorded_but_never_authoritative() {
    let mut r = Rig::playing();
    r.c.on_hint(r.now, Hint::InputDegraded);
    assert!(r.c.stats().input_degraded);
    r.feed(2);
    assert!(r.tick().is_empty());
    assert_eq!(r.c.state(), State::Playing);
    r.c.on_hint(r.now, Hint::InputRecovered);
    assert!(!r.c.stats().input_degraded);
    assert_eq!(r.c.stats().hints_seen, 2);
}

#[test]
fn source_ended_turns_a_later_pause_into_no_data() {
    let mut r = Rig::playing();
    r.c.on_hint(r.now, Hint::SourceEnded);
    // chunks keep arriving (reason would be DataArriving) but the clock overruns
    r.feed(1);
    let p = r.media % WRAP_27MHZ;
    r.c.on_tick(r.now + 1_000, Some(p));
    match r.drain()[..] {
        [Event::Pause { reason, .. }] => assert_eq!(reason, Reason::NoData),
        ref e => panic!("{e:?}"),
    }
}

#[test]
fn voluntary_reprime_is_off_by_default_and_works_when_enabled() {
    let mut r = Rig::playing();
    for _ in 0..40 {
        r.advance(15_000);
        r.feed(1); // lead shrinks 15 ms per 50 ms of wall time
        assert!(r.tick().iter().all(|e| !matches!(
            e,
            Event::Pause {
                reason: Reason::Reprime,
                ..
            }
        )));
    }
    let mut cfg = ControllerConfig::default();
    cfg.policy.reprime_floor_us = 1_000_000;
    let mut r = Rig::new(cfg);
    r.feed(60);
    r.tick();
    let mut saw = false;
    for _ in 0..200 {
        r.advance(15_000);
        r.feed(1);
        for e in r.tick() {
            if let Event::Pause {
                reason: Reason::Reprime,
                ..
            } = e
            {
                saw = true;
            }
        }
        if saw {
            break;
        }
    }
    assert!(saw, "re-prime fired below the floor after the hold");
    assert_eq!(r.c.stats().reprimes, 1);
    assert_eq!(r.c.stats().stalls, 0, "a re-prime is not a stall");
}

// ---------------------------------------------------------------------- policy

#[test]
fn policy_follower_matches_a_brute_force_reference_and_never_falls_faster_than_the_slew() {
    let cfg = PolicyConfig::DEFAULT;
    let mut p = Policy::new(cfg);
    let mut rng = Rng(11);
    let mut target = i128::from(cfg.min_us);
    let (mut now, mut prev_t): (u64, Option<u64>) = (0, None);
    let mut binding = false;
    for _ in 0..20_000 {
        now += rng.below(2_000_000);
        let need = if rng.below(4) == 0 {
            rng.below(6_000_000)
        } else {
            rng.below(500_000)
        };
        let raw = i128::from(need.clamp(cfg.min_us, cfg.max_us));
        let got = p.follow(need, now);
        // reference: rise at once, fall at most `step` per second (no fall on the first call)
        let fall = prev_t.map_or(0, |t| {
            i128::from(cfg.max_step_down_us_per_s) * i128::from(now - t) / 1_000_000
        });
        let want = raw.max(target - fall);
        assert_eq!(i128::from(got), want);
        binding |= want > raw;
        target = want;
        prev_t = Some(now);
        assert!((cfg.min_us..=cfg.max_us).contains(&got));
    }
    assert!(binding, "the slew limit bound at least once");
}

#[test]
fn policy_need_start_and_resume_formulas() {
    let c = PolicyConfig::DEFAULT;
    let p = Policy::new(c);
    // need = max(k1*base + margin, k2*peak) in Q8
    assert_eq!(p.need_us(20_000, 0), 40_000 + 100_000);
    assert_eq!(p.need_us(0, 1_447_000), 1_447_000 * 307 / 256);
    assert_eq!(p.need_us(1_000_000, 0), 2_100_000);
    assert_eq!(p.start_us(), 1_800_000);
    assert_eq!(p.resume_us(1_000_000), 1_500_000);
    assert_eq!(p.resume_us(0), c.min_us);
}

#[test]
fn max_us_is_a_hard_cap_everywhere() {
    let c = PolicyConfig::DEFAULT;
    let mut p = Policy::new(c);
    for v in [0, 1, c.max_us, c.max_us + 1, u64::MAX / 2, u64::MAX] {
        assert!(p.clamp(v) <= c.max_us);
        assert!(p.resume_us(v) <= c.max_us);
        let t = p.follow(p.need_us(v, v), v / 1000);
        assert!(t <= c.max_us && t >= c.min_us);
        p.raise(v);
        assert!(p.target_us() <= c.max_us);
        assert!(p.start_us() <= c.max_us);
    }
    // through the controller: a huge seeded stall memory and a 10 s gap
    let seed = StallMemorySnapshot::new(u64::MAX, 0);
    let mut ctl = BufferController::new(ControllerConfig::default(), Some(seed)).unwrap();
    assert_eq!(ctl.target_us(), c.max_us);
    ctl.restore_memory(StallMemorySnapshot::new(u64::MAX, 0));
    assert!(ctl.target_us() <= c.max_us);
}

#[test]
fn over_max_is_reported_once_per_excursion_and_the_target_stays_capped() {
    let mut s = Synth::new(60 * SEC);
    s.gaps = vec![(8 * SEC, 10_000_000)];
    let mut o = Opts::new(500_000);
    o.send_hints = false; // a long silence here is a stall, not a reconnect
    let r = replay(&s.generate(), &o);
    let max = PolicyConfig::DEFAULT.max_us;
    assert!(r.targets.iter().all(|&t| t <= max));
    let overs: Vec<_> = r
        .events
        .iter()
        .filter_map(|(_, e)| match e {
            Event::OverMax { need_us, max_us } => Some((*need_us, *max_us)),
            _ => None,
        })
        .collect();
    assert_eq!(overs.len(), 1, "once per excursion: {overs:?}");
    assert!(overs[0].0 > max && overs[0].1 == max);
    for (_, e) in &r.events {
        match e {
            Event::TargetChanged { new_us, .. } => assert!(*new_us <= max),
            Event::Pause {
                resume_target_us, ..
            } => assert!(*resume_target_us <= max),
            Event::Resume { lead_us, .. } => {
                assert!(*lead_us <= max + 40_000, "asked to hold {lead_us} > cap");
            }
            _ => {}
        }
    }
}

// ----------------------------------------------------------- config validation

#[test]
fn config_validation_names_every_bad_field() {
    let ok = ControllerConfig::DEFAULT;
    assert!(ok.validate().is_ok());
    let bad = |f: &dyn Fn(&mut ControllerConfig)| {
        let mut c = ok;
        f(&mut c);
        c.validate()
    };
    use ConfigField::*;
    assert_eq!(
        bad(&|c| c.jitter.min_window_us = 0),
        Err(Error::ZeroLengthWindow)
    );
    assert_eq!(
        bad(&|c| c.jitter.interval_us = 0),
        Err(Error::InvalidConfig(Interval))
    );
    assert_eq!(
        bad(&|c| c.jitter.min_window_us = 999_999),
        Err(Error::InvalidConfig(MinWindow))
    );
    assert_eq!(
        bad(&|c| c.jitter.bucket_us = 0),
        Err(Error::InvalidConfig(Bucket))
    );
    assert_eq!(
        bad(&|c| c.jitter.buckets = 0),
        Err(Error::InvalidConfig(Bucket))
    );
    assert_eq!(
        bad(&|c| c.jitter.buckets = 100),
        Err(Error::InvalidConfig(Bucket)),
        "max_us beyond the histogram range"
    );
    assert_eq!(
        bad(&|c| c.jitter.forget_q15 = 32768),
        Err(Error::InvalidConfig(Forget))
    );
    assert_eq!(
        bad(&|c| c.jitter.quantile_permille = 0),
        Err(Error::InvalidConfig(Quantile))
    );
    assert_eq!(
        bad(&|c| c.jitter.quantile_permille = 1001),
        Err(Error::InvalidConfig(Quantile))
    );
    assert_eq!(
        bad(&|c| c.jitter.outlier_us = 0),
        Err(Error::InvalidConfig(Outlier))
    );
    assert_eq!(
        bad(&|c| c.jitter.decay_us = 0),
        Err(Error::InvalidConfig(Decay))
    );
    assert_eq!(
        bad(&|c| c.jitter.max_samples = 7),
        Err(Error::InvalidConfig(Capacity))
    );
    assert_eq!(
        bad(&|c| c.jitter.discontinuity_us = 0),
        Err(Error::InvalidConfig(Discontinuity))
    );
    assert_eq!(
        bad(&|c| c.policy.min_us = 0),
        Err(Error::InvalidConfig(Order))
    );
    assert_eq!(
        bad(&|c| c.policy.initial_us = 100),
        Err(Error::InvalidConfig(Order))
    );
    assert_eq!(
        bad(&|c| c.policy.max_us = 1_000_000),
        Err(Error::InvalidConfig(Order))
    );
    assert_eq!(
        bad(&|c| c.policy.k1_q8 = 0),
        Err(Error::InvalidConfig(Multiplier))
    );
    assert_eq!(
        bad(&|c| c.policy.k2_q8 = 0),
        Err(Error::InvalidConfig(Multiplier))
    );
    assert_eq!(
        bad(&|c| c.policy.after_rebuffer_q8 = 0),
        Err(Error::InvalidConfig(Multiplier))
    );
    assert_eq!(
        bad(&|c| c.policy.max_step_down_us_per_s = 0),
        Err(Error::InvalidConfig(Slew))
    );
    assert_eq!(
        bad(&|c| c.starve_threshold_us = 300_000),
        Err(Error::InvalidConfig(Threshold))
    );
    assert_eq!(
        bad(&|c| c.nodata_after_us = 0),
        Err(Error::InvalidConfig(Threshold))
    );
    // a bad config can never build a controller, and the error renders
    let mut c = ok;
    c.jitter.interval_us = 0;
    let e = BufferController::new(c, None).unwrap_err();
    assert_eq!(e.to_string(), "invalid configuration: interval");
    assert!(ControllerConfig::new(ok.jitter, ok.policy, 20_000, 200_000).is_ok());
    assert!(ControllerConfig::new(ok.jitter, ok.policy, 20_000, 0).is_err());
}

#[test]
fn abi_surface_codes_are_stable() {
    assert_eq!(live_buffer::ABI_VERSION, 2);
    for (h, code) in [
        (Hint::InputDegraded, 1),
        (Hint::InputRecovered, 2),
        (Hint::SyncLoss, 3),
        (Hint::PcrDiscontinuity, 4),
        (Hint::SourceEnded, 5),
        (Hint::SourceError, 6),
        (Hint::Reconnected, 7),
    ] {
        assert_eq!(h.code(), code);
        assert_eq!(Hint::from_code(code), Some(h));
    }
    assert_eq!(Hint::from_code(0), None);
    assert_eq!(Hint::from_code(8), None);
    assert_eq!(
        [
            State::Buffering.code(),
            State::Playing.code(),
            State::Starved.code()
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            Reason::Startup.code(),
            Reason::NoData.code(),
            Reason::DataArriving.code(),
            Reason::Discontinuity.code(),
            Reason::Reprime.code()
        ],
        [1, 2, 3, 4, 5]
    );
    let rec = Event::Pause {
        hold_at_27mhz: 9,
        reason: Reason::NoData,
        depth_us: 3,
        resume_target_us: 4,
        track: None,
    }
    .to_record();
    assert_eq!(
        (rec.kind, rec.reason, rec.media_27mhz, rec.a_us, rec.b_us),
        (2, 2, 9, 3, 4)
    );
    let rec = Event::OverMax {
        need_us: 5,
        max_us: 6,
    }
    .to_record();
    assert_eq!((rec.kind, rec.a_us, rec.b_us), (5, 5, 6));
    let rec = Event::TargetChanged {
        old_us: 1,
        new_us: 2,
    }
    .to_record();
    assert_eq!((rec.kind, rec.a_us, rec.b_us), (4, 1, 2));
    assert_eq!(
        Event::StartPlaying {
            media_27mhz: 7,
            lead_us: 8
        }
        .to_record()
        .kind,
        1
    );
    assert_eq!(
        Event::Resume {
            media_27mhz: 7,
            lead_us: 8
        }
        .to_record()
        .kind,
        3
    );
    assert_eq!(Event::Rebaselined.to_record().kind, 6);
    assert_eq!(Event::EventsDropped { count: 3 }.to_record().a_us, 3);
    assert_eq!(live_buffer::pts90k_to_27mhz(90_000), 27_000_000);
    // one PTS wrap is exactly one PCR wrap
    assert_eq!(live_buffer::pts90k_to_27mhz(1 << 33), 0);
    assert_eq!(
        live_buffer::pts90k_to_27mhz((1 << 33) - 1),
        WRAP_27MHZ - 300
    );
    // config() accessor
    let c = BufferController::new(ControllerConfig::DEFAULT, None).unwrap();
    assert_eq!(*c.config(), ControllerConfig::DEFAULT);
}

#[test]
fn event_queue_drops_non_critical_events_first_and_reports_the_loss() {
    // a ramp makes many TargetChanged events; nobody polls
    let mut s = Synth::new(120 * SEC);
    s.jitter_us = 1_000;
    s.ramp_to_us = Some(1_000_000);
    let chunks = s.generate();
    let mut c = BufferController::new(ControllerConfig::DEFAULT, None).unwrap();
    for ch in &chunks {
        c.on_chunk(ch.arrival_us, ch.bytes, ch.pcr);
        c.on_media(ch.arrival_us, ch.pcr.unwrap());
        c.on_tick(ch.arrival_us, None);
    }
    let dropped = c.stats().events_dropped;
    assert!(dropped > 0, "the ramp overflowed the 16-entry queue");
    let first = c.poll_event().unwrap();
    assert_eq!(first, Event::EventsDropped { count: dropped });
    let mut rest = vec![];
    while let Some(e) = c.poll_event() {
        rest.push(e);
    }
    assert!(rest.len() <= 16);
    assert!(
        rest.iter().any(|e| matches!(e, Event::StartPlaying { .. })),
        "StartPlaying is critical and survives: {rest:?}"
    );
}

#[test]
fn reset_keeps_or_drops_the_stall_memory() {
    let mut s = Synth::new(30 * SEC);
    s.gaps = vec![(8 * SEC, 1_400_000)];
    let mut r = replay(&s.generate(), &Opts::new(1_800_000));
    let peak = r.ctl.stats().est.stall_peak_us;
    assert!(peak > 1_300_000);
    r.ctl.reset(true);
    assert_eq!(r.ctl.state(), State::Buffering);
    assert!(
        r.ctl.target_us() >= 1_550_000,
        "kept memory lifts the target: {}",
        r.ctl.target_us()
    );
    r.ctl.reset(false);
    assert_eq!(r.ctl.target_us(), PolicyConfig::DEFAULT.min_us);
    assert_eq!(r.ctl.snapshot_memory(), StallMemorySnapshot::default());
}

#[test]
fn seeded_memory_raises_the_start_target_for_the_hold_and_decay() {
    let snap = StallMemorySnapshot::new(1_450_000, 0);
    let mut cfg = ControllerConfig::default();
    cfg.policy.initial_us = 500_000;
    let c = BufferController::new(cfg, Some(snap)).unwrap();
    assert!(c.target_us() >= 1_700_000 && c.target_us() <= 1_740_000);
    // aged 30 s: still held; aged 200 s: gone
    let c = BufferController::new(cfg, Some(snap.aged(30_000_000))).unwrap();
    assert!(c.target_us() >= 1_700_000);
    let c = BufferController::new(cfg, Some(snap.aged(200_000_000))).unwrap();
    assert_eq!(c.target_us(), cfg.policy.min_us);
    // snapshot round trip through a live controller
    let mut c = BufferController::new(cfg, Some(snap)).unwrap();
    c.on_chunk(1_000, 1316, Some(1));
    let s2 = c.snapshot_memory();
    assert_eq!(s2.peak_us, 1_450_000);
}

/// `StallMemorySnapshot::new` is `const` (FFI wrappers build statics with it;
/// the struct is `#[non_exhaustive]`, so no literal outside the crate).
#[test]
fn snapshot_new_is_const_and_matches_fields() {
    const S: StallMemorySnapshot = StallMemorySnapshot::new(7, 9);
    assert_eq!((S.peak_us, S.age_us), (7, 9));
    assert_eq!(
        StallMemorySnapshot::new(0, 0),
        StallMemorySnapshot::default()
    );
}
