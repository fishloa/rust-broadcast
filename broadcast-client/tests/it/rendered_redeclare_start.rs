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

const TF1: &str = "http/live-channels/tf1-live.ts";
const CHUNK: usize = 188 * 64;
const TICK_US: u64 = 2_000;
const FIRST_DECLARATION_AFTER_US: u64 = 10_000;
const SECOND_DECLARATION_AFTER_US: u64 = 6_000;
const MICROS: u64 = 1_000_000;
const AUDIO_BEHIND_90KHZ: i64 = 135_000;
const BURST_BYTES: usize = 1024 * 1024;
const BURST_CHUNK_US: u64 = 200;
const RUN_US: u64 = 5 * MICROS;

struct Host {
    video: Option<TrackId>,
    audio: Option<TrackId>,
    tracks_at: Option<u64>,
    first_at: Option<u64>,
    second_done: bool,
}

struct Run {
    inputs: Vec<BufferInput>,
    start_input: Option<usize>,
    audio: u32,
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
            }
            _ => {}
        }
    }
    run.inputs.extend(s.drain_buffer_inputs());
}

fn run(bytes: &[u8]) -> Run {
    let bps = bitrate_bps(bytes);
    let mut config = Config::default();
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
        inputs: Vec::new(),
        start_input: None,
        audio: 0,
    };
    let mut chunks = bytes.chunks(CHUNK).peekable();
    let (mut next_chunk, mut next_tick) = (0u64, 0u64);
    let mut burst = BURST_BYTES;
    loop {
        let now = if chunks.peek().is_some() && next_chunk <= next_tick {
            let part = chunks.next().unwrap();
            s.on_data(conn, part, next_chunk);
            let at = next_chunk;
            burst = burst.saturating_sub(part.len());
            next_chunk += if burst > 0 {
                BURST_CHUNK_US
            } else {
                part.len() as u64 * 8 * MICROS / bps
            };
            at
        } else {
            s.on_playback_position(next_tick, None);
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
            s.set_rendered_tracks(&[host.video.expect("tf-1 has video")]);
            host.first_at = Some(now);
            drain(&mut s, &mut host, now, &mut out);
        }
        if !host.second_done
            && host
                .first_at
                .is_some_and(|f| now >= f + SECOND_DECLARATION_AFTER_US)
        {
            let both = [host.video.unwrap(), host.audio.expect("tf-1 has audio")];
            s.set_rendered_tracks(&both);
            host.second_done = true;
            drain(&mut s, &mut host, now, &mut out);
        }
        if out.start_input.is_some() || now >= RUN_US {
            break;
        }
    }
    out.audio = host.audio.expect("tracks announced").value();
    out
}

#[test]
fn a_video_first_declaration_does_not_start_before_the_audio_delivers() {
    let Some(bytes) = test_fixtures::require_bytes(TF1, "rendered_redeclare_start") else {
        return;
    };
    let r = run(&shift_pes(&bytes, 0, -AUDIO_BEHIND_90KHZ, Shift::Audio));
    let audio = r.audio;
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
