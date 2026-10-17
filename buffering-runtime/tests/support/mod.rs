//! Shared test harness: an obedient renderer model (the decision contract of
//! DESIGN.md section 2.4), a trace-replay driver, the real-trace loader and a
//! deterministic synthetic delivery generator.
//!
//! The renderer is ticked every `tick_us` plus once at every chunk arrival.
//! Its clock lives in the playback (PTS) domain, which for the trace replays
//! is the PCR minus a constant `offset_ticks` (the traces carry PCR only).
#![allow(dead_code)]

use buffering_runtime::{
    BufferController, ControllerConfig, Event, Hint, State, WRAP_27MHZ, pts90k_to_27mhz,
};
use std::path::Path;

pub const TICKS_PER_US: u64 = 27;
pub const SEC: u64 = 1_000_000;

#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    pub arrival_us: u64,
    pub bytes: u32,
    pub pcr: Option<u64>,
}

/// zenith-tv CSV (`arrival_ns,bytes,pcr_27mhz`), arrival rebased to the first chunk.
pub fn load_trace(path: &Path) -> Vec<Chunk> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut v = Vec::new();
    for line in text.lines().skip(1) {
        let mut f = line.split(',');
        let a: u64 = f.next().unwrap().parse().unwrap();
        let bytes: u32 = f.next().unwrap().parse().unwrap();
        let pcr = f
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap());
        v.push(Chunk {
            arrival_us: a / 1000,
            bytes,
            pcr,
        });
    }
    let t0 = v[0].arrival_us;
    for c in &mut v {
        c.arrival_us -= t0;
    }
    v
}

/// `(a + delta) mod WRAP_27MHZ` for a signed delta.
pub fn shift27(a: u64, delta: i64) -> u64 {
    (i128::from(a) + i128::from(delta)).rem_euclid(i128::from(WRAP_27MHZ)) as u64
}

#[derive(Clone)]
pub struct Opts {
    pub cfg: ControllerConfig,
    pub tick_us: u64,
    /// PCR minus PTS-domain position, in 27 MHz ticks.
    pub offset_ticks: i64,
    /// Error added to the renderer clock the caller reports (mis-mapped clock).
    pub clock_err_ticks: i64,
    /// A silence this long before a chunk is a reconnect (hints sent).
    pub reconnect_gap_us: u64,
    /// Send `Reconnected` at a detected reconnect (false: only the auto rule).
    pub send_hints: bool,
}

impl Opts {
    pub fn new(initial_us: u64) -> Self {
        let mut cfg = ControllerConfig::default();
        cfg.policy.initial_us = initial_us;
        Self {
            cfg,
            tick_us: 5_000,
            offset_ticks: 500_000 * TICKS_PER_US as i64,
            clock_err_ticks: 0,
            reconnect_gap_us: 2 * SEC,
            send_hints: true,
        }
    }
}

pub struct Run {
    pub events: Vec<(u64, Event)>,
    pub ctl: BufferController,
    /// `target_us()` after every PCR chunk.
    pub targets: Vec<u64>,
    /// Smallest `lead_us` reported by a tick while Playing.
    pub min_lead_playing: Option<i64>,
    /// Renderer clock frozen/held count of ticks where data was behind the
    /// restarted clock (always 0 for an obedient renderer).
    pub late_restarts: u32,
}

impl Run {
    pub fn start(&self) -> Option<u64> {
        self.events.iter().find_map(|(t, e)| match e {
            Event::StartPlaying { .. } => Some(*t),
            _ => None,
        })
    }
    pub fn pauses(&self) -> Vec<(u64, Option<u64>)> {
        let mut out: Vec<(u64, Option<u64>)> = Vec::new();
        for (t, e) in &self.events {
            match e {
                Event::Pause { .. } => out.push((*t, None)),
                Event::Resume { .. } => {
                    if let Some(l) = out.last_mut() {
                        l.1 = Some(*t);
                    }
                }
                _ => {}
            }
        }
        out
    }
    pub fn count(&self, f: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|(_, e)| f(e)).count()
    }
}

struct Renderer {
    running: bool,
    set: bool,
    clock: u64,
    t_set: u64,
}

impl Renderer {
    fn pos(&self, now: u64) -> u64 {
        if self.running {
            (self.clock + (now - self.t_set) * TICKS_PER_US) % WRAP_27MHZ
        } else {
            self.clock
        }
    }
    fn apply(&mut self, e: &Event, now: u64) {
        match *e {
            Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                self.clock = media_27mhz;
                self.t_set = now;
                self.running = true;
                self.set = true;
            }
            Event::Pause { hold_at_27mhz, .. } => {
                self.clock = hold_at_27mhz;
                self.t_set = now;
                self.running = false;
                self.set = true;
            }
            _ => {}
        }
    }
}

fn drain(c: &mut BufferController, r: &mut Renderer, now: u64, log: &mut Vec<(u64, Event)>) {
    while let Some(e) = c.poll_event() {
        r.apply(&e, now);
        log.push((now, e));
    }
}

/// One controller with its obedient renderer, steppable chunk by chunk so
/// several sessions can be interleaved.
pub struct Session {
    pub c: BufferController,
    r: Renderer,
    o: Opts,
    pub log: Vec<(u64, Event)>,
    pub targets: Vec<u64>,
    pub min_lead: Option<i64>,
    next_tick: u64,
    last_arrival: u64,
    seen: bool,
}

impl Session {
    pub fn new(o: &Opts) -> Self {
        Self {
            c: BufferController::new(o.cfg, None).unwrap(),
            r: Renderer {
                running: false,
                set: false,
                clock: 0,
                t_set: 0,
            },
            o: o.clone(),
            log: Vec::new(),
            targets: Vec::new(),
            min_lead: None,
            next_tick: 0,
            last_arrival: 0,
            seen: false,
        }
    }

    fn tick(&mut self, now: u64) {
        let reported = self
            .r
            .set
            .then(|| shift27(self.r.pos(now), self.o.clock_err_ticks));
        self.c.on_tick(now, reported);
        drain(&mut self.c, &mut self.r, now, &mut self.log);
        if self.c.state() == State::Playing {
            let l = self.c.stats().lead_us;
            self.min_lead = Some(self.min_lead.map_or(l, |m| m.min(l)));
        }
    }

    /// Deliver one chunk (ticking the renderer up to its arrival first).
    pub fn feed(&mut self, ch: &Chunk) {
        let a = ch.arrival_us;
        if self.seen && a - self.last_arrival >= self.o.reconnect_gap_us && self.o.send_hints {
            self.c.on_hint(self.last_arrival, Hint::SourceEnded);
            drain(&mut self.c, &mut self.r, self.last_arrival, &mut self.log);
            self.c.on_hint(a, Hint::Reconnected);
            drain(&mut self.c, &mut self.r, a, &mut self.log);
        }
        while self.next_tick <= a {
            self.tick(self.next_tick);
            self.next_tick += self.o.tick_us;
        }
        self.c.on_chunk(a, ch.bytes, ch.pcr);
        drain(&mut self.c, &mut self.r, a, &mut self.log);
        if let Some(pcr) = ch.pcr {
            self.targets.push(self.c.target_us());
            self.c.on_media(a, shift27(pcr, -self.o.offset_ticks));
        }
        self.tick(a);
        self.last_arrival = a;
        self.seen = true;
    }

    pub fn finish(self) -> Run {
        Run {
            events: self.log,
            ctl: self.c,
            targets: self.targets,
            min_lead_playing: self.min_lead,
            late_restarts: 0,
        }
    }
}

/// Replay `chunks` through a controller with an obedient renderer.
pub fn replay(chunks: &[Chunk], o: &Opts) -> Run {
    let mut s = Session::new(o);
    for ch in chunks {
        s.feed(ch);
    }
    s.finish()
}

// ---------------------------------------------------------------- synthetic

/// xorshift64*: deterministic, no dependency.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[derive(Clone, Debug)]
pub struct Synth {
    pub dur_us: u64,
    pub step_us: u64,
    pub jitter_us: u64,
    pub seed: u64,
    /// First PCR value (set near the wrap to cross it).
    pub pcr0: u64,
    /// `(at_media_us, length_us)` delivery gaps.
    pub gaps: Vec<(u64, u64)>,
    /// After a gap deliver the backlog at 10x instead of "no catch-up".
    pub catch_up: bool,
    /// Jitter amplitude grows linearly from `jitter_us` to this by the end.
    pub ramp_to_us: Option<u64>,
    /// Arrival clock drift versus media, parts per million (signed).
    pub drift_ppm: i64,
    /// `(at_media_us, dead_us, ahead_us)`: connection drops for `dead`, the
    /// new one joins `ahead` beyond the live edge (media jumps `dead + ahead`).
    pub reconnect: Option<(u64, u64, u64)>,
    /// `(at_media_us, jump_us)`: an instantaneous PCR jump with no dead time.
    pub pcr_jump: Option<(u64, i64)>,
}

impl Synth {
    pub fn new(dur_us: u64) -> Self {
        Self {
            dur_us,
            step_us: 35_000,
            jitter_us: 15_000,
            seed: 12345,
            pcr0: 27_000_000,
            gaps: vec![],
            catch_up: false,
            ramp_to_us: None,
            drift_ppm: 0,
            reconnect: None,
            pcr_jump: None,
        }
    }

    pub fn generate(&self) -> Vec<Chunk> {
        let mut rng = Rng(self.seed);
        let mut out: Vec<Chunk> = Vec::new();
        let (mut shift, mut media_off) = (0i64, 0i64);
        let mut gap_i = 0usize;
        let mut last = 0u64;
        let mut burst: Option<u64> = None;
        let n = self.dur_us / self.step_us;
        let mut reconnect_done = false;
        let mut jump_done = false;
        for k in 0..n {
            let m = k * self.step_us;
            if let Some((at, dead, ahead)) = self.reconnect
                && !reconnect_done
                && m >= at
            {
                reconnect_done = true;
                shift += dead as i64;
                media_off += (dead + ahead) as i64;
            }
            if let Some((at, jump)) = self.pcr_jump
                && !jump_done
                && m >= at
            {
                jump_done = true;
                media_off += jump;
            }
            if !self.catch_up {
                while gap_i < self.gaps.len() && m >= self.gaps[gap_i].0 {
                    shift += self.gaps[gap_i].1 as i64;
                    gap_i += 1;
                }
            }
            let amp = match self.ramp_to_us {
                Some(to) => self.jitter_us + (to - self.jitter_us.min(to)) * m / self.dur_us.max(1),
                None => self.jitter_us,
            };
            let jit = rng.below(amp + 1) as i64;
            let drift = (m as i64) * self.drift_ppm / 1_000_000;
            let ideal_extra = shift + drift + jit;
            let ideal = (m as i64 + ideal_extra).max(0) as u64;
            let mut arrival = ideal;
            if self.catch_up {
                if burst.is_none() && gap_i < self.gaps.len() && ideal >= self.gaps[gap_i].0 {
                    burst = Some(self.gaps[gap_i].0 + self.gaps[gap_i].1);
                    gap_i += 1;
                }
                if let Some(b) = burst {
                    if b > ideal {
                        arrival = b;
                        burst = Some(b + self.step_us / 10);
                    } else {
                        burst = None;
                    }
                }
            }
            arrival = arrival.max(last);
            last = arrival;
            let media_ticks = (m as i64 + media_off) * TICKS_PER_US as i64 + self.pcr0 as i64;
            out.push(Chunk {
                arrival_us: arrival,
                bytes: 1316,
                pcr: Some(shift27(0, media_ticks)),
            });
        }
        out
    }
}

/// Delivery-stall lengths of the synthetic suite: 1x/2x/4x, the pattern of a
/// transport that retransmits with exponential backoff (RFC 6298); the cause of
/// the real ~1.4 s stalls is undetermined, only their size is measured.
pub const GAP_1S: u64 = 1_000_000;
pub const GAP_2S: u64 = 2_000_000;
pub const GAP_4S: u64 = 4_000_000;

/// PTS-domain helper for tests: 90 kHz PTS to 27 MHz.
pub fn pts(p: u64) -> u64 {
    pts90k_to_27mhz(p)
}
