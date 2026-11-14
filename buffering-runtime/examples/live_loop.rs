//! Skeleton of a caller loop: the pause/resume contract with a mock renderer.
//!
//! A synthetic 10 s live stream with a 1.4 s delivery gap at 4 s is fed
//! through the controller; the mock renderer obeys the contract (section 2.4
//! of the design): freeze at `Pause.hold_at`, queue whatever arrives while
//! paused, restart at exactly `Resume.media` (clock and rate in one step).
//! The initial target is lowered to 500 ms so the gap causes a visible pause.
//!
//! ```text
//! cargo run -p buffering-runtime --example live_loop
//! ```

use broadcast_common::clock33::PCR_WRAP_27MHZ;
use buffering_runtime::{
    BufferController, ControllerConfig, Event, Micros, Ticks27, pts90k_to_27mhz,
};

const TICK_US: u64 = 50_000; // recommended tick period
const CHUNK_US: u64 = 35_000; // one PCR chunk every 35 ms
const TICKS_PER_US: u64 = 27;

/// What a real wrapper owns: the renderer clock and its queue.
struct MockRenderer {
    running: bool,
    clock: Ticks27,
    set_at: Micros,
    queued_until: Ticks27,
    played_behind_clock: u32,
}

impl MockRenderer {
    fn position(&self, now: Micros) -> Ticks27 {
        if self.running {
            Ticks27(
                (self.clock.0 + now.saturating_sub(self.set_at).0 * TICKS_PER_US) % PCR_WRAP_27MHZ,
            )
        } else {
            self.clock
        }
    }
    fn apply(&mut self, ev: Event, now: Micros) {
        match ev {
            Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                // set the clock to EXACTLY this position and rate 1 in one step
                if self.queued_until < media_27mhz {
                    self.played_behind_clock += 1; // would mean data behind the clock
                }
                self.clock = media_27mhz;
                self.set_at = now;
                self.running = true;
            }
            Event::Pause { hold_at_27mhz, .. } => {
                // rate 0, freeze; keep queueing while paused
                self.clock = hold_at_27mhz;
                self.running = false;
            }
            _ => {}
        }
    }
}

fn main() {
    let mut cfg = ControllerConfig::default();
    cfg.policy.initial_us = Micros(500_000);
    let mut c = BufferController::new(cfg, None).expect("default config is valid");
    let mut r = MockRenderer {
        running: false,
        clock: Ticks27::ZERO,
        set_at: Micros::ZERO,
        queued_until: Ticks27::ZERO,
        played_behind_clock: 0,
    };
    let pts_offset_90k = 45_000; // PCR runs 500 ms ahead of PTS
    let mut next_tick = Micros::ZERO;
    let mut dead_until = 0u64;
    let mut media_us = 0u64;
    for k in 0..(10_000_000 / CHUNK_US) {
        let now = k * CHUNK_US;
        media_us += CHUNK_US;
        if now >= 4_000_000 && dead_until == 0 {
            dead_until = now + 1_400_000; // delivery gap, nothing skipped
        }
        let arrival = now.max(if dead_until != 0 && now < dead_until {
            dead_until
        } else {
            0
        });
        while next_tick.0 <= arrival {
            let p = (r.running || r.clock != Ticks27::ZERO).then(|| r.position(next_tick));
            c.on_tick(next_tick, p);
            while let Some(ev) = c.poll_event() {
                println!("{:7.3}s {ev:?}", next_tick.get() as f64 / 1e6);
                r.apply(ev, next_tick);
            }
            next_tick = Micros(next_tick.0 + TICK_US);
        }
        let pcr = Ticks27((media_us * TICKS_PER_US + 27_000_000) % PCR_WRAP_27MHZ);
        let pts = (media_us * 90 / 1000).saturating_sub(pts_offset_90k) + 90_000;
        c.on_chunk(Micros(arrival), 1316, Some(pcr));
        c.on_media(Micros(arrival), pts90k_to_27mhz(pts));
        r.queued_until = pts90k_to_27mhz(pts);
    }
    let s = c.stats();
    println!(
        "stalls={} longest_freeze={:.2}s target={}ms data-behind-clock-at-restart={}",
        s.stalls,
        s.longest_freeze_us.get() as f64 / 1e6,
        s.target_us.get() / 1000,
        r.played_behind_clock
    );
}
