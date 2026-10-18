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

use broadcast_common::clock33::PCR_HZ;
use buffering_runtime::{
    BufferController, ControllerConfig, Event, Micros, PolicyConfig, Ticks27, WRAP_27MHZ,
};

const AUDIO: u32 = 1;
const VIDEO: u32 = 2;
const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
const BASE: Ticks27 = WRAP_27MHZ - 5 * PCR_HZ;
const T0: Micros = 1_000_000;
const TICK_US: Micros = 10_000;

fn at(offset_us: u64) -> Ticks27 {
    (BASE + offset_us * TICKS_PER_US) % WRAP_27MHZ
}

fn cfg(initial_us: Micros) -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = initial_us;
    ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap()
}

struct Out {
    /// First `StartPlaying`, microseconds after the first delivery.
    start_after: Option<Micros>,
    /// `Pause` instants, microseconds after the first delivery.
    pauses: Vec<Micros>,
}

/// `audio_period` gives the interval to the n-th audio delivery (microseconds);
/// `register` announces both tracks before the first delivery.
fn run(initial_us: Micros, gaps: &[Micros], register: bool, secs: u64) -> Out {
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
    let (mut audio, mut next_audio, mut gi) = (0u64, T0 + gaps[0], 0usize);
    let mut next_chunk = T0 + 100_000;
    let mut t = T0;
    while t < T0 + secs * 1_000_000 {
        t += TICK_US;
        if t >= next_chunk {
            let i = (next_chunk - T0) / 100_000;
            c.on_chunk(next_chunk, 1000, Some(at(i * 100_000)));
            c.on_track_media(next_chunk, VIDEO, at(i * 100_000 + 900_000));
            next_chunk += 100_000;
        }
        if t >= next_audio {
            audio += 300_000;
            c.on_track_media(next_audio, AUDIO, at(audio));
            gi += 1;
            next_audio += gaps[gi % gaps.len()];
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

const JITTERED: [Micros; 2] = [250_000, 350_000];
const STEADY: [Micros; 1] = [100_000];

/// A start fill below the audio PES step, jittered audio: the gate waits until
/// the cadence is known and leaves with `G + margin`, so nothing stalls.
#[test]
fn small_start_fill_with_coarse_audio_does_not_stall() {
    let o = run(300_000, &JITTERED, true, 30);
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
    for (initial_us, jittered, steady) in
        [(500_000, 600_000, 300_000), (1_800_000, 1_800_000, 900_000)]
    {
        for register in [true, false] {
            let j = run(initial_us, &JITTERED, register, 30);
            assert_eq!(
                j.start_after,
                Some(jittered),
                "jittered, initial {initial_us}"
            );
            assert!(j.pauses.is_empty(), "stalled at {:?}", j.pauses);
            let s = run(initial_us, &STEADY, register, 30);
            assert_eq!(s.start_after, Some(steady), "steady, initial {initial_us}");
            assert!(s.pauses.is_empty(), "stalled at {:?}", s.pauses);
        }
    }
}

/// Fine-grained tracks (audio every chunk, 100 ms) learn their cadence at once
/// and the gate costs (almost) nothing: the start is at most one gate window
/// after the plain start-fill would have been met.
#[test]
fn fine_grained_tracks_start_without_extra_wait() {
    let o = run(300_000, &STEADY, true, 10);
    assert!(o.pauses.is_empty());
    let s = o.start_after.unwrap();
    assert!(s <= 450_000, "start after {s}");
}
