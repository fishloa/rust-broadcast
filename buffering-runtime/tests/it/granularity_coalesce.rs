//! The delivery instant of the granularity floor is the chunk boundary: a
//! caller reporting a track end once per delivered chunk and one reporting it
//! once per demuxed sample get the same floor (rai-3: one MP2 PES of ~0.3 s,
//! 12 frames, per chunk; the first integration reported per frame and the floor
//! never engaged).

use broadcast_common::clock33::PCR_HZ;
use buffering_runtime::{
    BufferController, ControllerConfig, Event, Micros, PolicyConfig, Ticks27, WRAP_27MHZ,
};

const AUDIO: u32 = 1;
const VIDEO: u32 = 2;
const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
const BASE: Ticks27 = WRAP_27MHZ - 5 * PCR_HZ;

fn at(offset_us: u64) -> Ticks27 {
    (BASE + offset_us * TICKS_PER_US) % WRAP_27MHZ
}

fn cfg() -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = p.min_us;
    ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap()
}

type Run = (Vec<(u64, Event)>, Micros, Vec<Micros>);

/// A PCR chunk every 100 ms, video end `t + 0.9 s` per chunk, one audio PES
/// (300 ms of media) in every third chunk, reported as ONE end per delivered
/// chunk or as 12 per-frame calls of 25 ms starting at the chunk's arrival
/// time and `between_us` apart.
fn pes_run(per_frame: bool, between_us: u64) -> Run {
    let mut c = BufferController::new(cfg(), None).unwrap();
    let mut log = Vec::new();
    let mut targets = Vec::new();
    let t0 = 1_000_000u64;
    c.on_chunk(t0, 1000, Some(at(0)));
    c.on_track_media(t0, AUDIO, at(0));
    c.on_track_media(t0, VIDEO, at(0));
    let (mut audio, mut clock, mut t_set, mut running) = (0u64, at(0), t0, false);
    for i in 1..=600u64 {
        let t = t0 + i * 100_000;
        c.on_chunk(t, 1000, Some(at(i * 100_000)));
        c.on_track_media(t, VIDEO, at(i * 100_000 + 900_000));
        if i % 3 == 0 {
            if per_frame {
                for f in 1..=12u64 {
                    c.on_track_media(t + (f - 1) * between_us, AUDIO, at(audio + f * 25_000));
                }
            } else {
                c.on_track_media(t, AUDIO, at(audio + 300_000));
            }
            audio += 300_000;
        }
        let pos = if running {
            (clock + (t - t_set) * TICKS_PER_US) % WRAP_27MHZ
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
        targets.push(c.target_us());
    }
    (log, c.target_us(), targets)
}

/// Per-PES and per-frame (at one instant, or microseconds apart) reports give
/// identical events and targets, and the floor engages (target >= the 300 ms
/// step + margin). Mutation: making every call its own window
/// (`same_window = false` in `tracks.rs`) leaves the per-frame callers with no
/// floor and the equality fails.
#[test]
fn per_chunk_and_per_frame_reports_give_the_same_floor() {
    let margin = cfg().policy.margin_us;
    let a = pes_run(false, 0);
    let b = pes_run(true, 0);
    let c = pes_run(true, 50);
    assert!(a.1 >= 300_000 + margin, "floor must engage: {}", a.1);
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
        let cfg = ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us)
            .unwrap()
            .with_track_granularity(floor);
        let mut c = BufferController::new(cfg, None).unwrap();
        let mut t = 1_000_000u64;
        c.on_track_media(t, AUDIO, at(0));
        for k in 0..40u64 {
            t += 300_000;
            for f in 1..=12u64 {
                c.on_track_media(
                    t + (f - 1) * frame_gap_us,
                    AUDIO,
                    at(k * 300_000 + f * 25_000),
                );
            }
        }
        c.target_us()
    };
    // 12 frames inside 1 ms: one 300 ms step, the floor adds to the PDV term
    assert!(run(10, false) < 300_000 + cfg().policy.margin_us);
    assert!(run(10, true) >= 300_000 + cfg().policy.margin_us);
    // frames 25 ms apart in arrival time are fine-grained: the floor is idle
    assert_eq!(run(25_000, true), run(25_000, false));
}
