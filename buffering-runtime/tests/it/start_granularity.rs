//! Start gate: a start must not be granted before the controller knows each
//! track's delivery granularity, and must leave with at least one granularity
//! step of cushion (`G + margin`) even when the configured start fill is
//! smaller. Closed loop: a simulated renderer consumes at 1x exactly where the
//! controller says to start/resume/hold, so a stall here is the controller's
//! own decision, not an open-loop replay artefact.
//!
//! The source delivers video steadily (one chunk per 100 ms, 0.9 s ahead) and
//! audio as one 300 ms PES per chunk, arriving alternately 250 ms and 350 ms
//! apart (media advances 300 ms per delivery: average real time, jittered
//! like rai-3's 0.3-0.4 s bursts).

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
const T0: Duration = dur(1_000_000);
const TICK_US: Duration = dur(10_000);

fn at(offset_us: u64) -> Clock27 {
    Clock27::from_27mhz((BASE.ticks() + offset_us * TICKS_PER_US) % PCR_WRAP_27MHZ)
        .expect("a reduced count is on the clock")
}

const fn us(v: u64) -> Duration {
    dur(v)
}

fn cfg(initial_us: Duration) -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial = initial_us;
    ControllerConfig::new(d.jitter, p, d.starve_threshold, d.nodata_after).unwrap()
}

struct Out {
    /// First `StartPlaying`, microseconds after the first delivery.
    start_after: Option<Duration>,
    /// `Pause` instants, microseconds after the first delivery.
    pauses: Vec<Duration>,
}

/// `audio_period` gives the interval to the n-th audio delivery (microseconds);
/// `register` announces both tracks before the first delivery.
fn run(initial_us: Duration, gaps: &[Duration], register: bool, secs: u64) -> Out {
    let mut c = BufferController::new(cfg(initial_us), None).unwrap();
    if register {
        c.register_track(AUDIO);
        c.register_track(VIDEO);
    }
    let mut out = Out {
        start_after: None,
        pauses: Vec::new(),
    };
    // first reports (zero-length) at the join point
    c.on_chunk(T0, 1000, Some(at(0)));
    c.on_track_media(T0, AUDIO, at(0));
    c.on_track_media(T0, VIDEO, at(0));
    let (mut clock, mut t_set, mut running) = (at(0), T0, false);
    let mut now_us = T0.as_micros() as u64;
    let (mut audio, mut next_audio, mut gi): (u64, u64, usize) =
        (0, now_us + gaps[0].as_micros() as u64, 0);
    let mut next_chunk = now_us + 100_000;
    let t0_us = now_us;
    while now_us < t0_us + secs * 1_000_000 {
        now_us += TICK_US.as_micros() as u64;
        if now_us >= next_chunk {
            let i = (next_chunk - t0_us) / 100_000;
            c.on_chunk(dur(next_chunk), 1000, Some(at(i * 100_000)));
            c.on_track_media(dur(next_chunk), VIDEO, at(i * 100_000 + 900_000));
            next_chunk += 100_000;
        }
        if now_us >= next_audio {
            audio += 300_000;
            c.on_track_media(dur(next_audio), AUDIO, at(audio));
            gi += 1;
            next_audio += gaps[gi % gaps.len()].as_micros() as u64;
        }
        let t = dur(now_us);
        let pos = if running {
            at((t.as_micros() - t_set.as_micros()) as u64 + clock.ticks() / TICKS_PER_US)
        } else {
            clock
        };
        c.on_tick(t, Some(pos));
        while let Some(e) = c.poll_event() {
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    if out.start_after.is_none() {
                        out.start_after = Some(t - T0);
                    }
                    clock = media_27mhz;
                    t_set = t;
                    running = true;
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    out.pauses.push(t - T0);
                    clock = hold_at_27mhz;
                    running = false;
                }
                _ => {}
            }
        }
    }
    out
}

const JITTERED: [Duration; 2] = [us(250_000), us(350_000)];
const STEADY: [Duration; 1] = [us(100_000)];

/// A start fill below the audio PES step, jittered audio: the gate waits until
/// the cadence is known and leaves with `G + margin`, so nothing stalls.
#[test]
fn small_start_fill_with_coarse_audio_does_not_stall() {
    let o = run(us(300_000), &JITTERED, true, 30);
    eprintln!("start {:?} pauses {:?}", o.start_after, o.pauses);
    assert!(o.start_after.is_some());
    assert!(o.pauses.is_empty(), "stalled at {:?}", o.pauses);
}

/// At a start fill of 500 ms and above the gate costs nothing: the start is
/// bound by the audio lead (one 300 ms PES per delivery) and by the gate's own
/// bound (`start_us()` after the track's first delivery), never by waiting for
/// three closed windows (which would be ~1.05 s here). Measured with the gate
/// bypassed, the same simulation starts at the same instants. Zero stalls.
#[test]
fn gate_adds_no_start_delay_at_500ms_and_above() {
    for (initial_us, jittered, steady) in [
        (us(500_000), us(600_000), us(300_000)),
        (us(1_800_000), us(1_800_000), us(900_000)),
    ] {
        for register in [true, false] {
            let j = run(initial_us, &JITTERED, register, 30);
            assert_eq!(
                j.start_after,
                Some(jittered),
                "jittered, initial {}us",
                initial_us.as_micros()
            );
            assert!(j.pauses.is_empty(), "stalled at {:?}", j.pauses);
            let s = run(initial_us, &STEADY, register, 30);
            assert_eq!(
                s.start_after,
                Some(steady),
                "steady, initial {}us",
                initial_us.as_micros()
            );
            assert!(s.pauses.is_empty(), "stalled at {:?}", s.pauses);
        }
    }
}

/// Fine-grained tracks (audio every chunk, 100 ms) learn their cadence at once
/// and the gate costs (almost) nothing: the start is at most one gate window
/// after the plain start-fill would have been met.
#[test]
fn fine_grained_tracks_start_without_extra_wait() {
    let o = run(us(300_000), &STEADY, true, 10);
    assert!(o.pauses.is_empty());
    let s = o.start_after.unwrap();
    assert!(s <= us(450_000), "start after {}us", s.as_micros());
}
