//! A host that declares `[video]` first and `[video, audio]` a few ms later (the tvOS player on
//! live tf-1, 2026-11-08: it handles the video `TrackAdded` before its audio pick succeeds) must
//! not get a `Start` judged on video alone: the audio that was being waited for keeps holding the
//! start until it has delivered, and it is never unregistered and re-registered.
//!
//! The shape of the field log: an opening burst (1 MiB at 200 us per chunk) puts ~1 s of video
//! ahead before the tracks are announced, and audio is muxed 1.5 s behind video (shifted here), so
//! no audio past the join point has arrived while the two declarations are made.

use crate::support::replay::{bitrate_bps, open_live};
use crate::support::ts_shift::{Shift, shift_pes};

use broadcast_client::{BufferEvent, BufferInput, Config, Event, Session, TrackId};
use broadcast_common::stage::Timestamp;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const TF1: &str = "http/live-channels/tf1-live.ts";
const CHUNK: usize = 188 * 64;
const TICK_US: u64 = 2_000;
const FIRST_DECLARATION_AFTER_US: u64 = 10_000;
const SECOND_DECLARATION_AFTER_US: u64 = 6_000;
const MICROS: u64 = 1_000_000;
const AUDIO_BEHIND_90KHZ: i64 = 135_000;
const BURST_BYTES: usize = 1024 * 1024;
const BURST_CHUNK_US: u64 = 200;
const RUN_US: u64 = 8 * MICROS;
const AUDIO_NEVER_90KHZ: i64 = 900_000;
const STAMP_OFFSET_US: u64 = 1_234;

struct Host {
    video: Option<TrackId>,
    audio: Option<TrackId>,
    tracks_at: Option<u64>,
    first_at: Option<u64>,
    second_done: bool,
}

struct Run {
    session: Session,
    inputs: Vec<BufferInput>,
    start_input: Option<usize>,
    start_at: Option<u64>,
    dropped_at: u64,
    video: TrackId,
    audio: TrackId,
}

fn drain(s: &mut Session, host: &mut Host, now: u64, run: &mut Run) {
    while let Some(e) = s.next_event() {
        match e {
            Event::Tracks(specs) if host.tracks_at.is_none() => {
                for spec in &specs {
                    if spec.config.is_video() {
                        host.video.get_or_insert(spec.track_id);
                    } else if spec.config.audio_sample_rate().is_some()
                        && host.audio.is_none_or(|a| spec.track_id < a)
                    {
                        host.audio = Some(spec.track_id);
                    }
                }
                host.tracks_at = Some(now);
            }
            Event::Buffer(BufferEvent::Start { .. }) if run.start_input.is_none() => {
                run.inputs.extend(s.drain_buffer_inputs());
                run.start_input = Some(run.inputs.len());
                run.start_at = Some(now);
            }
            _ => {}
        }
    }
    run.inputs.extend(s.drain_buffer_inputs());
}

fn run(bytes: &[u8], redeclare: bool, mut config: Config) -> Run {
    let bps = bitrate_bps(bytes);
    config.record_buffer_inputs = true;
    let (mut s, conn) = open_live(config);
    let mut host = Host {
        video: None,
        audio: None,
        tracks_at: None,
        first_at: None,
        second_done: false,
    };
    let mut out = Run {
        session: open_live(Config::default()).0,
        inputs: Vec::new(),
        start_input: None,
        start_at: None,
        dropped_at: 0,
        video: TrackId::new(0),
        audio: TrackId::new(0),
    };
    let mut chunks = bytes.chunks(CHUNK).peekable();
    let (mut next_chunk, mut next_tick) = (0u64, 0u64);
    let mut burst = BURST_BYTES;
    loop {
        let now = if chunks.peek().is_some() && next_chunk <= next_tick {
            let part = chunks.next().unwrap();
            s.on_data(conn, part, ts(next_chunk));
            let at = next_chunk;
            burst = burst.saturating_sub(part.len());
            next_chunk += if burst > 0 {
                BURST_CHUNK_US
            } else {
                part.len() as u64 * 8 * MICROS / bps
            };
            at
        } else {
            s.on_playback_position(ts(next_tick), None);
            let at = next_tick;
            next_tick += TICK_US;
            at
        };
        drain(&mut s, &mut host, now, &mut out);
        if host.first_at.is_none()
            && host
                .tracks_at
                .is_some_and(|t| now >= t + FIRST_DECLARATION_AFTER_US)
        {
            s.set_rendered_tracks(ts(now), &[host.video.expect("tf-1 has video")]);
            host.first_at = Some(now);
            drain(&mut s, &mut host, now, &mut out);
        }
        if redeclare
            && !host.second_done
            && host
                .first_at
                .is_some_and(|f| now >= f + SECOND_DECLARATION_AFTER_US)
        {
            let both = [host.video.unwrap(), host.audio.expect("tf-1 has audio")];
            s.set_rendered_tracks(ts(now), &both);
            host.second_done = true;
            drain(&mut s, &mut host, now, &mut out);
        }
        if out.start_input.is_some() || now >= RUN_US {
            break;
        }
    }
    out.video = host.video.expect("tracks announced");
    out.audio = host.audio.expect("tracks announced");
    out.dropped_at = host.first_at.expect("declared");
    out.session = s;
    out
}

#[test]
fn a_video_first_declaration_does_not_start_before_the_audio_delivers() {
    let Some(bytes) = test_fixtures::require_bytes(TF1, "rendered_redeclare_start") else {
        return;
    };
    let r = run(
        &shift_pes(&bytes, 0, -AUDIO_BEHIND_90KHZ, Shift::Audio),
        true,
        Config::default(),
    );
    let audio = r.audio.value();
    let start = r.start_input.expect("the run reaches Start");
    let before = &r.inputs[..start];
    assert!(
        before
            .iter()
            .any(|i| matches!(i, BufferInput::TrackMedia { track, .. } if *track == audio)),
        "Start was decided before audio {audio} reported any media"
    );
    let churn: Vec<&BufferInput> = r
        .inputs
        .iter()
        .filter(|i| matches!(i, BufferInput::Unregister { track, .. } if *track == audio))
        .collect();
    assert!(
        churn.is_empty(),
        "audio {audio} stayed rendered but was unregistered: {churn:?}"
    );
}

#[test]
fn a_track_dropped_before_start_is_released_at_the_start() {
    let Some(bytes) = test_fixtures::require_bytes(TF1, "rendered_redeclare_start") else {
        return;
    };
    let mut config = Config::default();
    config.start_pending_track_wait_us = RUN_US;
    let r = run(
        &shift_pes(&bytes, 0, -AUDIO_BEHIND_90KHZ, Shift::Audio),
        false,
        config,
    );
    let audio = r.audio.value();
    let start = r.start_input.expect("the run reaches Start");
    let unregister = r
        .inputs
        .iter()
        .position(|i| matches!(i, BufferInput::Unregister { track, .. } if *track == audio));
    assert!(
        r.inputs[..start]
            .iter()
            .any(|i| matches!(i, BufferInput::TrackMedia { track, .. } if *track == audio)),
        "the dropped audio {audio} must still hold the first Start"
    );
    let decided = r.inputs[..start]
        .iter()
        .rposition(|i| matches!(i, BufferInput::Tick { .. }))
        .unwrap();
    assert!(
        unregister.is_some_and(|u| u > decided),
        "audio {audio} is released after the tick that started, not before: {unregister:?} vs {decided}"
    );
}

#[test]
fn a_dropped_track_that_never_delivers_holds_the_start_only_for_the_pending_wait() {
    let Some(bytes) = test_fixtures::require_bytes(TF1, "rendered_redeclare_start") else {
        return;
    };
    let config = Config::default();
    let wait = config.start_pending_track_wait_us;
    let silence = config.controller_config().track_silence_timeout_us;
    let r = run(
        &shift_pes(&bytes, 0, -AUDIO_NEVER_90KHZ, Shift::Audio),
        false,
        Config::default(),
    );
    let audio = r.audio.value();
    let start = r.start_input.expect("the run reaches Start");
    assert!(
        !r.inputs[..start]
            .iter()
            .any(|i| matches!(i, BufferInput::TrackMedia { track, .. } if *track == audio)),
        "audio {audio} must not deliver in this run"
    );
    let after_drop = r.start_at.unwrap() - r.dropped_at;
    assert!(
        (wait..wait + 2 * TICK_US).contains(&after_drop),
        "Start {after_drop} us after the drop, want the {wait} us pending wait"
    );
    assert!(
        after_drop < silence.0,
        "not the {silence} us silence timeout"
    );
}

#[test]
fn rendered_set_rows_carry_the_session_time_of_the_call() {
    let Some(bytes) = test_fixtures::require_bytes(TF1, "rendered_redeclare_start") else {
        return;
    };
    let mut r = run(
        &shift_pes(&bytes, 0, -AUDIO_BEHIND_90KHZ, Shift::Audio),
        true,
        Config::default(),
    );
    let s = &mut r.session;
    s.drain_buffer_inputs().for_each(drop);
    let t = r.start_at.expect("started") + STAMP_OFFSET_US;
    s.set_rendered_tracks(ts(t), &[r.video]);
    s.set_rendered_tracks(ts(t), &[r.video, r.audio]);
    let rows: Vec<BufferInput> = s
        .drain_buffer_inputs()
        .filter(|i| {
            matches!(
                i,
                BufferInput::Register { .. } | BufferInput::Unregister { .. }
            )
        })
        .collect();
    let audio = r.audio.value();
    assert_eq!(
        rows,
        vec![
            BufferInput::Unregister {
                now_us: buffering_runtime::Micros(t),
                track: audio
            },
            BufferInput::Register {
                now_us: buffering_runtime::Micros(t),
                track: audio
            },
        ]
    );
}
