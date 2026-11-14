//! The delivery instant of the granularity floor is the chunk boundary: a
//! caller reporting a track end once per delivered chunk and one reporting it
//! once per demuxed sample get the same floor (rai-3: one MP2 PES of ~0.3 s,
//! 12 frames, per chunk; the first integration reported per frame and the floor
//! never engaged).

const fn dur(us: u64) -> Duration {
    Duration::from_micros(us)
}
use broadcast_common::clock33::PCR_WRAP_27MHZ;
use broadcast_common::clock33::{PCR_EXT_MODULUS, PCR_HZ};
use buffering_runtime::{BufferController, Clock27, ControllerConfig, Event, PolicyConfig};
use core::time::Duration;

const AUDIO: u32 = 1;
const VIDEO: u32 = 2;
const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
/// A reduced 27 MHz tick count as a validated clock position.
const fn clock27(ticks: u64) -> Clock27 {
    match Clock27::from_parts(ticks / PCR_EXT_MODULUS, (ticks % PCR_EXT_MODULUS) as u16) {
        Ok(c) => c,
        Err(_) => unreachable!(),
    }
}

const BASE: Clock27 = clock27(PCR_WRAP_27MHZ - 5 * 27_000_000);

fn at(offset_us: u64) -> Clock27 {
    Clock27::from_27mhz((BASE.ticks() + offset_us * TICKS_PER_US) % PCR_WRAP_27MHZ)
        .expect("a reduced count is on the clock")
}

fn cfg() -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial = p.min;
    ControllerConfig::new(d.jitter, p, d.starve_threshold, d.nodata_after).unwrap()
}

type Run = (Vec<(Duration, Event)>, Duration, Vec<Duration>);

/// A PCR chunk every 100 ms, video end `t + 0.9 s` per chunk, one audio PES
/// (300 ms of media) in every third chunk, reported as ONE end per delivered
/// chunk or as 12 per-frame calls of 25 ms starting at the chunk's arrival
/// time and `between_us` apart.
fn pes_run(per_frame: bool, between_us: u64) -> Run {
    let mut c = BufferController::new(cfg(), None).unwrap();
    let mut log = Vec::new();
    let mut targets = Vec::new();
    let t0 = dur(1_000_000);
    c.on_chunk(t0, 1000, Some(at(0)));
    c.on_track_media(t0, AUDIO, at(0));
    c.on_track_media(t0, VIDEO, at(0));
    let (mut audio, mut clock, mut t_set, mut running) = (0u64, at(0), t0, false);
    for i in 1..=600u64 {
        let t = t0 + dur(i * 100_000);
        c.on_chunk(t, 1000, Some(at(i * 100_000)));
        c.on_track_media(t, VIDEO, at(i * 100_000 + 900_000));
        if i % 3 == 0 {
            if per_frame {
                for f in 1..=12u64 {
                    c.on_track_media(t + dur((f - 1) * between_us), AUDIO, at(audio + f * 25_000));
                }
            } else {
                c.on_track_media(t, AUDIO, at(audio + 300_000));
            }
            audio += 300_000;
        }
        let pos = if running {
            clock27(
                (clock.ticks() + (t - t_set).as_micros() as u64 * TICKS_PER_US) % PCR_WRAP_27MHZ,
            )
        } else {
            clock
        };
        c.on_tick(t, Some(pos));
        while let Some(e) = c.poll_event() {
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    clock = media_27mhz;
                    t_set = t;
                    running = true;
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    clock = hold_at_27mhz;
                    running = false;
                }
                _ => {}
            }
            log.push((t, e));
        }
        targets.push(c.target());
    }
    (log, c.target(), targets)
}

/// Per-PES and per-frame (at one instant, or microseconds apart) reports give
/// identical events and targets, and the floor engages (target >= the 300 ms
/// step + margin). Mutation: making every call its own window
/// (`same_window = false` in `tracks.rs`) leaves the per-frame callers with no
/// floor and the equality fails.
#[test]
fn per_chunk_and_per_frame_reports_give_the_same_floor() {
    let margin = cfg().policy.margin;
    let a = pes_run(false, 0);
    let b = pes_run(true, 0);
    let c = pes_run(true, 50);
    assert!(
        a.1.as_micros() >= 300_000 + margin.as_micros(),
        "floor must engage: {:?}",
        a.1
    );
    assert_eq!(a, b, "per-frame at one instant");
    assert_eq!(a, c, "per-frame microseconds apart");
}

/// With no `on_chunk` at all, calls within 1 ms are one delivery and calls
/// further apart are distinct ones. Mutation: dropping the time fallback
/// (windows then never close, no step is seen, `coarse` stays at 300 ms).
#[test]
fn chunkless_callers_coalesce_within_a_millisecond() {
    let run = |frame_gap_us: u64, floor: bool| {
        // PDV term at its minimum so only the floor can lift the target
        let d = ControllerConfig::default();
        let mut p = PolicyConfig::default();
        p.k1_q8 = 1;
        let cfg = ControllerConfig::new(d.jitter, p, d.starve_threshold, d.nodata_after)
            .unwrap()
            .with_track_granularity(floor);
        let mut c = BufferController::new(cfg, None).unwrap();
        let mut t = dur(1_000_000);
        c.on_track_media(t, AUDIO, at(0));
        for k in 0..40u64 {
            t += dur(300_000);
            for f in 1..=12u64 {
                c.on_track_media(
                    t + dur((f - 1) * frame_gap_us),
                    AUDIO,
                    at(k * 300_000 + f * 25_000),
                );
            }
        }
        c.target()
    };
    // 12 frames inside 1 ms: one 300 ms step, the floor adds to the PDV term
    assert!(run(10, false).as_micros() < 300_000 + cfg().policy.margin.as_micros());
    assert!(run(10, true).as_micros() >= 300_000 + cfg().policy.margin.as_micros());
    // frames 25 ms apart in arrival time are fine-grained: the floor is idle
    assert_eq!(run(25_000, true), run(25_000, false));
}
