//! Re-registration and the start gate (tf-1 on tvOS, 2026-11-08,
//! `private/fixtures/logs/buffering/tf1-start-2026-11-08-*`, see its PROVENANCE.md).
//!
//! A re-registered track is a fresh registration: it holds the start until it delivers or the
//! silence rule excludes it, exactly like a track registered once. `unregister_track` releases a
//! start that was waiting on the track at the next tick, as documented. The field failure was a
//! caller that unregistered audio and ticked before registering it again (broadcast-client let a
//! transient `[video]` declaration through); the controller's start on video alone was correct
//! for the inputs it was given.

use buffering_runtime::input_log::{Input, InputLog};
use buffering_runtime::{BufferController, ControllerConfig, Event, Micros, PolicyConfig, Ticks27};

const VIDEO: u32 = 1;
const AUDIO: u32 = 2;
const TICKS_PER_US: u64 = 27;
const BASE: Ticks27 = 1_000_000 * TICKS_PER_US;
const T0: Micros = 50_000_000;
const CHUNK_US: Micros = 20_000;
const VIDEO_AHEAD_US: Micros = 1_000_000;
const AUDIO_FIRST_AT_US: Micros = 400_000;
const RUN_US: Micros = 2_000_000;

fn at(offset_us: u64) -> Ticks27 {
    BASE + offset_us * TICKS_PER_US
}

fn controller() -> BufferController {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = 500_000;
    let cfg = ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap();
    BufferController::new(cfg, None).unwrap()
}

/// Video is ~1 s ahead from the first chunk, audio (past the join point) only from
/// `AUDIO_FIRST_AT_US`. `churn` runs once video is well ahead: `Some(false)` unregisters and
/// re-registers audio back to back, `Some(true)` ticks in between. Returns the first `Start`
/// instant (after `T0`) and its lead.
fn run(churn: Option<bool>) -> Option<(Micros, u64)> {
    let mut c = controller();
    c.register_track(VIDEO);
    c.register_track(AUDIO);
    let mut churned = false;
    let mut t = 0;
    while t < RUN_US {
        let now = T0 + t;
        c.on_chunk(now, 1000, None);
        if t == 0 {
            c.on_track_media(now, VIDEO, at(0));
        }
        c.on_track_media(now, VIDEO, at(t + VIDEO_AHEAD_US));
        if t >= AUDIO_FIRST_AT_US {
            c.on_track_media(now, AUDIO, at(t - AUDIO_FIRST_AT_US / 2));
        }
        if let Some(tick_between) = churn
            && !churned
            && t >= AUDIO_FIRST_AT_US / 2
        {
            churned = true;
            c.unregister_track(AUDIO);
            if tick_between {
                c.on_tick(now, None);
            }
            c.register_track(AUDIO);
        }
        c.on_tick(now, None);
        while let Some(e) = c.poll_event() {
            if let Event::StartPlaying { lead_us, .. } = e {
                return Some((t, lead_us));
            }
        }
        t += CHUNK_US;
    }
    None
}

#[test]
fn a_re_registered_track_holds_the_start_until_it_delivers() {
    let once = run(None).expect("starts once audio delivers");
    assert!(once.0 >= AUDIO_FIRST_AT_US, "started at {once:?}");
    assert_eq!(
        run(Some(false)),
        Some(once),
        "unregister + register back to back is a fresh registration that holds like the first"
    );
}

#[test]
fn a_tick_between_unregister_and_register_releases_the_start_as_documented() {
    let (t, lead) = run(Some(true)).expect("released");
    assert!(
        t < AUDIO_FIRST_AT_US,
        "the tick in between starts on video alone, at {t}"
    );
    assert!(lead >= VIDEO_AHEAD_US - CHUNK_US);
}

fn log(name: &str) -> Option<InputLog> {
    let path = test_fixtures::require(
        &format!("logs/buffering/tf1-start-2026-11-08-{name}-apple-tv.lb.csv"),
        "buffering-runtime reregistered_track_start",
    )?;
    let mut log = InputLog::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    log.rebase();
    Some(log)
}

/// Index of the input whose call produced the first `StartPlaying`, and the event's lead.
fn first_start(log: &InputLog) -> (usize, Micros, u64) {
    let mut c = log.controller().unwrap();
    for (i, input) in log.inputs.iter().enumerate() {
        input.apply(&mut c);
        while let Some(e) = c.poll_event() {
            if let Event::StartPlaying { lead_us, .. } = e {
                return (i, input.now_us(), lead_us);
            }
        }
    }
    panic!("the log never starts");
}

fn is_audio_unregister(i: &Input) -> bool {
    matches!(i, Input::Unregister { track, .. } if *track == AUDIO)
}

fn is_audio_register(i: &Input) -> bool {
    matches!(i, Input::Register { track, .. } if *track == AUDIO)
}

/// The audio unregister row and the re-register row that follows it.
fn churn_rows(log: &InputLog) -> (usize, usize) {
    let unregister = log.inputs.iter().position(is_audio_unregister).unwrap();
    let register = unregister
        + 1
        + log.inputs[unregister + 1..]
            .iter()
            .position(is_audio_register)
            .unwrap();
    (unregister, register)
}

#[test]
fn the_field_start_came_from_a_tick_while_audio_was_unregistered() {
    let Some(log) = log("fail") else {
        return;
    };
    let (unregister, register) = churn_rows(&log);
    let (start, _, lead) = first_start(&log);
    assert!(
        unregister < start && start < register,
        "Start at input {start}, audio unregistered at {unregister} and re-registered at {register}"
    );
    assert!(matches!(log.inputs[start], Input::Tick { .. }));
    assert_eq!(lead, 1_000_000);
}

#[test]
fn without_the_churn_the_field_log_starts_only_once_audio_covers_the_lead() {
    let Some(mut log) = log("fail") else {
        return;
    };
    let (unregister, register) = churn_rows(&log);
    log.inputs.remove(register);
    log.inputs.remove(unregister);
    let first_audio = log
        .inputs
        .iter()
        .position(|i| matches!(i, Input::TrackMedia { track, .. } if *track == AUDIO))
        .unwrap();
    let (start, now, lead) = first_start(&log);
    assert!(
        start > first_audio,
        "Start at input {start} before audio's first media at {first_audio}"
    );
    let audio_end = log.inputs[..start]
        .iter()
        .filter_map(|i| match i {
            Input::TrackMedia { track, end, .. } if *track == AUDIO => Some(*end),
            _ => None,
        })
        .max()
        .unwrap();
    let origin = log
        .inputs
        .iter()
        .find_map(|i| match i {
            Input::TrackMedia { end, .. } => Some(*end),
            _ => None,
        })
        .unwrap();
    assert!(
        lead * TICKS_PER_US <= audio_end - origin,
        "lead {lead} us is not covered by audio, which ends {} us past the origin",
        (audio_end - origin) / TICKS_PER_US
    );
    assert_eq!((now, lead), (681_132, 687_995));
}

#[test]
fn the_pass_log_is_unchanged() {
    let Some(log) = log("pass") else {
        return;
    };
    let (_, now, lead) = first_start(&log);
    assert_eq!((now, lead), (1_734_275, 543_995));
}
