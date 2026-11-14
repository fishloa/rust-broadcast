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

use core::time::Duration;

use broadcast_common::clock33::PCR_WRAP_27MHZ;
use buffering_runtime::{
    BufferController, Clock27, ControllerConfig, Event, micros_as_duration, pts90k_to_clock27,
};

const TICK: Duration = Duration::from_millis(50); // recommended tick period
const CHUNK_US: u64 = 35_000; // one PCR chunk every 35 ms
const TICKS_PER_US: u64 = 27;

/// What a real wrapper owns: the renderer clock and its queue.
struct MockRenderer {
    running: bool,
    clock: Clock27,
    set_at: Duration,
    queued_until: Clock27,
    played_behind_clock: u32,
}

impl MockRenderer {
    fn position(&self, now: Duration) -> Clock27 {
        if self.running {
            let elapsed_us = now.saturating_sub(self.set_at).as_micros() as u64;
            Clock27::from_27mhz((self.clock.ticks() + elapsed_us * TICKS_PER_US) % PCR_WRAP_27MHZ)
                .expect("a reduced 27 MHz value is on the clock")
        } else {
            self.clock
        }
    }
    fn apply(&mut self, ev: Event, now: Duration) {
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
    cfg.policy.initial = Duration::from_millis(500);
    let mut c = BufferController::new(cfg, None).expect("default config is valid");
    let mut r = MockRenderer {
        running: false,
        clock: Clock27::from_27mhz(0).expect("zero is on the clock"),
        set_at: Duration::ZERO,
        queued_until: Clock27::from_27mhz(0).expect("zero is on the clock"),
        played_behind_clock: 0,
    };
    let pts_offset_90k = 45_000; // PCR runs 500 ms ahead of PTS
    let mut next_tick = Duration::ZERO;
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
        while next_tick.as_micros() as u64 <= arrival {
            let started = Clock27::from_27mhz(0).expect("zero is on the clock");
            let p = (r.running || r.clock != started).then(|| r.position(next_tick));
            c.on_tick(next_tick, p);
            while let Some(ev) = c.poll_event() {
                println!("{:7.3}s {ev:?}", next_tick.as_secs_f64());
                r.apply(ev, next_tick);
            }
            next_tick += TICK;
        }
        let pcr = Clock27::from_27mhz((media_us * TICKS_PER_US + 27_000_000) % PCR_WRAP_27MHZ)
            .expect("a reduced 27 MHz value is on the clock");
        let pts = (media_us * 90 / 1000).saturating_sub(pts_offset_90k) + 90_000;
        let arrival = micros_as_duration(arrival);
        c.on_chunk(arrival, 1316, Some(pcr));
        let end = pts90k_to_clock27(pts).expect("a 33-bit PTS is on the 27 MHz clock");
        c.on_media(arrival, end);
        r.queued_until = end;
    }
    let s = c.stats();
    println!(
        "stalls={} longest_freeze={:.2}s target={}ms data-behind-clock-at-restart={}",
        s.stalls,
        s.longest_freeze.as_secs_f64(),
        s.target.as_millis(),
        r.played_behind_clock
    );
}
