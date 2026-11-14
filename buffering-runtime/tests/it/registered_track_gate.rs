//! Registered-track start gate (Apple TV regression): `register_track` called
//! at pipeline creation, BEFORE any media, must behave exactly like registering
//! later. The audio of a TS mux is muxed ~1 s behind video and the caller
//! withholds audio ends until the video join point, so a registered audio
//! track legitimately has no report for ~1 s after the first (video) one. The
//! controller must not start (or resume) while a registered, not-yet-silent
//! track has not delivered, and its silence clock starts at the FIRST DELIVERY
//! on any track, never at the registration (a registration made before any
//! media is stamped with a clock that has not started, 0).
//!
//! Closed loop: a simulated renderer consumes at 1x exactly where the
//! controller says to start/resume/hold, so a pause here is the controller's
//! own decision. Wall clock is far from 0 (`T0` = 50 s) as in a real client.

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
const T0: Duration = dur(50_000_000);
const TICK_US: Duration = dur(10_000);
/// Audio is muxed this long behind video: first audio report at `T0 + AUDIO_LAG`.
const AUDIO_LAG_US: Duration = dur(1_000_000);
const AUDIO_STEP_US: Duration = dur(300_000);

fn at(offset_us: u64) -> Clock27 {
    Clock27::from_27mhz((BASE.ticks() + offset_us * TICKS_PER_US) % PCR_WRAP_27MHZ)
        .expect("a reduced count is on the clock")
}

const fn us(v: u64) -> Duration {
    dur(v)
}

fn cfg() -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial = us(300_000);
    ControllerConfig::new(d.jitter, p, d.starve_threshold, d.nodata_after).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    start_after: Option<Duration>,
    /// (instant after T0, lead) of every Pause.
    pauses: Vec<(Duration, Duration)>,
    events: Vec<(Duration, String)>,
}

/// `registration` lists the `register_track` calls made before the first
/// media, in order. `audio_delivers` false: audio never reports at all.
fn run(registration: &[u32], audio_delivers: bool, secs: u64) -> Out {
    let mut c = BufferController::new(cfg(), None).unwrap();
    for t in registration {
        c.register_track(*t);
    }
    let mut out = Out {
        start_after: None,
        pauses: Vec::new(),
        events: Vec::new(),
    };
    // video join point: zero-length first report; audio withheld until later
    c.on_chunk(T0, 1000, Some(at(0)));
    c.on_track_media(T0, VIDEO, at(0));
    let (mut clock, mut t_set, mut running) = (at(0), T0, false);
    let t0_us = T0.as_micros() as u64;
    let mut next_audio = t0_us + AUDIO_LAG_US.as_micros() as u64;
    let mut next_chunk = t0_us + 100_000;
    let mut now_us = t0_us;
    while now_us < t0_us + secs * 1_000_000 {
        now_us += TICK_US.as_micros() as u64;
        if now_us >= next_chunk {
            let i = (next_chunk - t0_us) / 100_000;
            c.on_chunk(dur(next_chunk), 1000, Some(at(i * 100_000)));
            c.on_track_media(dur(next_chunk), VIDEO, at(i * 100_000 + 900_000));
            if audio_delivers && next_chunk >= next_audio {
                // audio ends trail video by ~0.7 s, one report per 0.3 s chunk
                let end = (next_chunk - t0_us).saturating_sub(
                    AUDIO_LAG_US.as_micros() as u64 - AUDIO_STEP_US.as_micros() as u64,
                );
                c.on_track_media(dur(next_chunk), AUDIO, at(end));
                next_audio = next_chunk + AUDIO_STEP_US.as_micros() as u64;
            }
            next_chunk += 100_000;
        }
        let t = dur(now_us);
        let pos = if running {
            at((t.as_micros() - t_set.as_micros()) as u64 + clock.ticks() / TICKS_PER_US)
        } else {
            clock
        };
        c.on_tick(t, Some(pos));
        while let Some(e) = c.poll_event() {
            out.events.push((t - T0, format!("{e:?}")));
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    if out.start_after.is_none() {
                        out.start_after = Some(t - T0);
                    }
                    clock = media_27mhz;
                    t_set = t;
                    running = true;
                }
                Event::Pause {
                    hold_at_27mhz,
                    depth,
                    ..
                } => {
                    out.pauses.push((t - T0, depth));
                    clock = hold_at_27mhz;
                    running = false;
                }
                _ => {}
            }
        }
    }
    out
}

/// The regression: both registered up front, audio ~1 s behind video.
#[test]
fn registered_up_front_waits_for_late_audio() {
    for reg in [&[AUDIO, VIDEO][..], &[VIDEO, AUDIO][..]] {
        let o = run(reg, true, 30);
        let s = o.start_after.expect("started");
        assert!(
            s >= AUDIO_LAG_US,
            "{reg:?}: started at {}us before audio delivered at {}us",
            s.as_micros(),
            AUDIO_LAG_US.as_micros()
        );
        assert!(o.pauses.is_empty(), "{reg:?}: paused {:?}", o.pauses);
    }
}

/// Only audio registered (video unregistered, joins on its first report).
#[test]
fn only_audio_registered_waits_for_late_audio() {
    let o = run(&[AUDIO], true, 30);
    assert!(o.start_after.expect("started") >= AUDIO_LAG_US);
    assert!(o.pauses.is_empty(), "paused {:?}", o.pauses);
}

/// Registration order (before the first report) never changes the event
/// stream; nor does registering only the late track vs both.
#[test]
fn registration_order_is_irrelevant() {
    let a = run(&[AUDIO, VIDEO], true, 20);
    let b = run(&[VIDEO, AUDIO], true, 20);
    let c = run(&[AUDIO], true, 20);
    assert_eq!(a, b);
    assert_eq!(a, c);
}

/// A registered track that never reports blocks until the silence timeout,
/// counted from the first delivery on any track (not from the registration).
#[test]
fn never_reporting_registered_track_blocks_until_timeout() {
    let timeout = cfg().track_silence_timeout;
    let o = run(&[AUDIO, VIDEO], false, 30);
    let s = o.start_after.expect("starts once the dead track times out");
    assert!(
        s >= timeout,
        "started at {}us, before the {} timeout",
        s.as_micros(),
        timeout.as_micros()
    );
    assert!(
        s <= timeout + dur(1_000_000),
        "started late at {}us",
        s.as_micros()
    );
}
