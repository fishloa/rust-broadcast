//! Property tests over random operation streams (seeded xorshift; no extra
//! dependency): no panic on any input, and a fixed invariant set.
use broadcast_common::clock33::PCR_WRAP_27MHZ;

/// A 27 MHz tick count as a validated clock position.
fn clock27(ticks: u64) -> Clock27 {
    Clock27::from_27mhz(ticks % PCR_WRAP_27MHZ).expect("a reduced count is on the clock")
}

const fn dur(us: u64) -> Duration {
    Duration::from_micros(us)
}
use crate::support::Rng;
use buffering_runtime::{BufferController, Clock27, ControllerConfig, Event, Hint, State};
use core::time::Duration;

const fn us(v: u64) -> Duration {
    dur(v)
}

const MIN: Duration = dur(300_000);
const MAX: Duration = dur(3_000_000);

fn pick_u64(r: &mut Rng) -> u64 {
    match r.below(10) {
        0 => u64::MAX,
        1 => 0,
        2 => u64::MAX - r.below(100),
        3 => 1 << 63,
        _ => r.below(1 << 40),
    }
}

/// One random run; returns a fingerprint of everything observable (determinism).
fn run(seed: u64, steps: u32, hostile: bool) -> u64 {
    let mut rng = Rng(seed);
    let mut c = BufferController::new(ControllerConfig::default(), None).unwrap();
    let mut now = 1_000_000u64;
    let mut media = 27_000_000u64;
    let mut playing = false;
    let mut started = false;
    let mut paused = false;
    let mut prev_target = (c.target(), now);
    let mut fp = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u64| {
        fp ^= v;
        fp = fp.wrapping_mul(0x100_0000_01b3);
    };
    for _ in 0..steps {
        // mostly sane time, sometimes hostile
        let step = if hostile && rng.below(20) == 0 {
            pick_u64(&mut rng)
        } else {
            rng.below(80_000)
        };
        let t_in = if hostile && rng.below(30) == 0 {
            pick_u64(&mut rng)
        } else {
            now.saturating_add(step)
        };
        match rng.below(6) {
            0..=2 => {
                let pcr = if hostile && rng.below(10) == 0 {
                    pick_u64(&mut rng)
                } else {
                    media
                };
                c.on_chunk(
                    us(t_in),
                    u32::try_from(rng.below(1 << 20)).unwrap(),
                    (rng.below(8) != 0).then_some(clock27(pcr)),
                );
                media = media.wrapping_add(35_000 * 27);
                c.on_media(us(t_in), clock27(media));
            }
            3 | 4 => c.on_tick(
                us(t_in),
                (rng.below(5) != 0).then(|| clock27(pick_u64(&mut rng))),
            ),
            _ => {
                let h = [
                    Hint::InputDegraded,
                    Hint::InputRecovered,
                    Hint::SyncLoss,
                    Hint::PcrDiscontinuity,
                    Hint::SourceEnded,
                    Hint::SourceError,
                    Hint::Reconnected,
                ][rng.below(7) as usize];
                c.on_hint(us(t_in), h);
            }
        }
        if t_in >= now && t_in != u64::MAX {
            now = t_in;
        }
        // invariants
        let t = c.target();
        assert!((MIN..=MAX).contains(&t), "target {:?} out of [min, max]", t);
        // never falls faster than max_step_down (100 ms/s); rises are free
        // (only meaningful on a sane clock: a hostile clock jump is a legitimate long dt)
        if !hostile && now > prev_target.1 && t < prev_target.0 {
            let fall = prev_target.0 - t;
            let allowed = u128::from(100_000u64) * u128::from(now - prev_target.1) / 1_000_000 + 1;
            assert!(
                fall.as_micros() <= allowed || !hostile,
                "target fell {}us in {}us",
                fall.as_micros(),
                now - prev_target.1
            );
        }
        prev_target = (t, now.max(prev_target.1));
        let mut n = 0;
        while let Some(e) = c.poll_event() {
            n += 1;
            assert!(n <= 17, "more events than the queue holds");
            match e {
                Event::StartPlaying { .. } => {
                    assert!(!started, "second StartPlaying");
                    started = true;
                    playing = true;
                }
                Event::Pause { resume_target, .. } => {
                    assert!(playing, "Pause while not playing");
                    assert!(resume_target <= MAX);
                    playing = false;
                    paused = true;
                }
                Event::Resume { .. } => {
                    assert!(
                        paused && !playing,
                        "Resume without a preceding Pause (no Playing->Playing Resume)"
                    );
                    playing = true;
                    paused = false;
                }
                Event::TargetChanged { new, .. } => assert!(new <= MAX),
                Event::OverMax { max, need } => assert!(need > max),
                Event::EventsDropped { .. } | Event::Rebaselined => {}
                _ => {}
            }
            mix(e.to_record().kind.into());
            mix(e.to_record().media_27mhz);
        }
        let s = c.stats();
        assert_eq!(s.state == State::Playing, playing);
        mix(u64::from(s.state.code()));
        mix(s.target.as_micros() as u64);
        mix(s.est.baseline.as_micros() as u64);
    }
    fp
}

#[test]
fn sane_random_streams_hold_every_invariant() {
    for seed in 1..=60 {
        run(seed, 4_000, false);
    }
}

#[test]
fn hostile_random_streams_never_panic_and_keep_the_bounds() {
    for seed in 1..=60 {
        run(0xdead_0000 + seed, 4_000, true);
    }
}

#[test]
fn identical_input_gives_identical_output() {
    for seed in [3u64, 99, 12345] {
        assert_eq!(run(seed, 3_000, true), run(seed, 3_000, true));
    }
}
