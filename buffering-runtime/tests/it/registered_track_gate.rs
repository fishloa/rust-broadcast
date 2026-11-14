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

use broadcast_common::clock33::PCR_HZ;
use broadcast_common::clock33::PCR_WRAP_27MHZ;
use buffering_runtime::{BufferController, ControllerConfig, Event, Micros, PolicyConfig, Ticks27};

const AUDIO: u32 = 1;
const VIDEO: u32 = 2;
const TICKS_PER_US: u64 = PCR_HZ / 1_000_000;
const BASE: Ticks27 = Ticks27(PCR_WRAP_27MHZ - 5 * PCR_HZ);
const T0: Micros = Micros(50_000_000);
const TICK_US: Micros = Micros(10_000);
/// Audio is muxed this long behind video: first audio report at `T0 + AUDIO_LAG`.
const AUDIO_LAG_US: Micros = Micros(1_000_000);
const AUDIO_STEP_US: Micros = Micros(300_000);

fn at(offset_us: u64) -> Ticks27 {
    Ticks27((BASE.0 + offset_us * TICKS_PER_US) % PCR_WRAP_27MHZ)
}

const fn us(v: u64) -> Micros {
    Micros(v)
}

fn cfg() -> ControllerConfig {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = us(300_000);
    ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    start_after: Option<Micros>,
    /// (instant after T0, lead_us) of every Pause.
    pauses: Vec<(Micros, Micros)>,
    events: Vec<(Micros, String)>,
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
    let mut next_audio = Micros(T0.0 + AUDIO_LAG_US.0);
    let mut next_chunk = Micros(T0.0 + 100_000);
    let mut t = T0;
    while t < Micros(T0.0 + secs * 1_000_000) {
        t = Micros(t.0 + TICK_US.0);
        if t >= next_chunk {
            let i = (next_chunk.0 - T0.0) / 100_000;
            c.on_chunk(next_chunk, 1000, Some(at(i * 100_000)));
            c.on_track_media(next_chunk, VIDEO, at(i * 100_000 + 900_000));
            if audio_delivers && next_chunk >= next_audio {
                // audio ends trail video by ~0.7 s, one report per 0.3 s chunk
                let end = (next_chunk.0 - T0.0).saturating_sub(AUDIO_LAG_US.0 - AUDIO_STEP_US.0);
                c.on_track_media(next_chunk, AUDIO, at(end));
                next_audio = Micros(next_chunk.0 + AUDIO_STEP_US.0);
            }
            next_chunk = Micros(next_chunk.0 + 100_000);
        }
        let pos = if running {
            Ticks27((clock.0 + (t.0 - t_set.0) * TICKS_PER_US) % PCR_WRAP_27MHZ)
        } else {
            clock
        };
        c.on_tick(t, Some(pos));
        while let Some(e) = c.poll_event() {
            out.events.push((Micros(t.0 - T0.0), format!("{e:?}")));
            match e {
                Event::StartPlaying { media_27mhz, .. } | Event::Resume { media_27mhz, .. } => {
                    if out.start_after.is_none() {
                        out.start_after = Some(Micros(t.0 - T0.0));
                    }
                    clock = media_27mhz;
                    t_set = t;
                    running = true;
                }
                Event::Pause {
                    hold_at_27mhz,
                    depth_us,
                    ..
                } => {
                    out.pauses.push((Micros(t.0 - T0.0), depth_us));
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
            s.0 >= AUDIO_LAG_US.0,
            "{reg:?}: started at {s} before audio delivered at {AUDIO_LAG_US}"
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
    let timeout = cfg().track_silence_timeout_us;
    let o = run(&[AUDIO, VIDEO], false, 30);
    let s = o.start_after.expect("starts once the dead track times out");
    assert!(s >= timeout, "started at {s}, before the {timeout} timeout");
    assert!(s <= Micros(timeout.0 + 1_000_000), "started late at {s}");
}
