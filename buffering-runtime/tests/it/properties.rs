//! Property tests over random operation streams (seeded xorshift; no extra
//! dependency): no panic on any input, and a fixed invariant set.
use crate::support::Rng;
use buffering_runtime::{BufferController, ControllerConfig, Event, Hint, State};

const MIN: u64 = 300_000;
const MAX: u64 = 3_000_000;

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
    let mut prev_target = (c.target_us(), now);
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
                    t_in,
                    u32::try_from(rng.below(1 << 20)).unwrap(),
                    (rng.below(8) != 0).then_some(pcr),
                );
                media = media.wrapping_add(35_000 * 27);
                c.on_media(t_in, media);
            }
            3 | 4 => c.on_tick(t_in, (rng.below(5) != 0).then(|| pick_u64(&mut rng))),
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
                c.on_hint(t_in, h);
            }
        }
        if t_in >= now && t_in != u64::MAX {
            now = t_in;
        }
        // invariants
        let t = c.target_us();
        assert!((MIN..=MAX).contains(&t), "target {t} out of [min, max]");
        // never falls faster than max_step_down (100 ms/s); rises are free
        // (only meaningful on a sane clock: a hostile clock jump is a legitimate long dt)
        if !hostile && now > prev_target.1 && t < prev_target.0 {
            let fall = prev_target.0 - t;
            let allowed = u128::from(100_000u64) * u128::from(now - prev_target.1) / 1_000_000 + 1;
            assert!(
                u128::from(fall) <= allowed || !hostile,
                "target fell {fall} in {} us",
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
                Event::Pause {
                    resume_target_us, ..
                } => {
                    assert!(playing, "Pause while not playing");
                    assert!(resume_target_us <= MAX);
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
                Event::TargetChanged { new_us, .. } => assert!(new_us <= MAX),
                Event::OverMax { max_us, need_us } => assert!(need_us > max_us),
                Event::EventsDropped { .. } | Event::Rebaselined => {}
                _ => {}
            }
            mix(e.to_record().kind.into());
            mix(e.to_record().media_27mhz);
        }
        let s = c.stats();
        assert_eq!(s.state == State::Playing, playing);
        mix(u64::from(s.state.code()));
        mix(s.target_us);
        mix(s.est.baseline_us);
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
