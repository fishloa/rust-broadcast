use crate::support;
use crate::support::video_run::VideoRun;

use broadcast_client::{Action, Config, Event, Session, TrackId};
use broadcast_common::stage::Timestamp;
use support::replay::{TS_PACKET, bitrate_bps, open_live, replay_paced};

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const CHUNK: usize = 4096;
const SETTLE_US: u64 = 500_000;
const FAR_US: u64 = 60_000_000;
const TF1: &str = "http/live-channels/tf1-live.ts";
const TF1_LAST_AUDIO: u16 = 0x84;
const TF1_LATE_DATA: u16 = 0x97;
const TF1_EARLY_DATA: u16 = 0x96;
const FRANCE2: &str = "ts/france2.ts";
const FRANCE2_SILENT: u16 = 142;
const PID_MASK: u16 = 0x1fff;
const PAYLOAD_START: usize = 4;
const GARBAGE: u8 = 0xa5;
const STEP_US: u64 = 20_000;
const MAX_WAKES: usize = 16;

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1]) << 8 | u16::from(p[2])) & PID_MASK
}

fn without_pid(bytes: &[u8], pid: u16) -> Vec<u8> {
    bytes
        .chunks(TS_PACKET)
        .filter(|p| pid_of(p) != pid)
        .flatten()
        .copied()
        .collect()
}

fn garbage(bytes: &[u8], pid: u16) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for p in out.chunks_mut(TS_PACKET) {
        if pid_of(p) == pid {
            p[PAYLOAD_START..].fill(GARBAGE);
        }
    }
    out
}

fn config(wall: u64, media: u64) -> Config {
    {
        let mut new_config = Config::default();
        new_config.tracks_hold_max_wall_us = wall;
        new_config.tracks_hold_max_media_us = media;
        new_config
    }
}

fn sets(r: &support::replay::Replay) -> Vec<Vec<TrackId>> {
    r.events
        .iter()
        .filter_map(|t| match &t.event {
            Event::Tracks(v) => Some(v.iter().map(|s| s.track_id).collect()),
            _ => None,
        })
        .collect()
}

fn early_release_case(label: &str, bytes: &[u8]) {
    let bps = bitrate_bps(bytes);
    let free = replay_paced(bytes, config(0, 0), CHUNK, bps);
    let last_resolution = *free.tracks_times().last().unwrap();
    let final_set: Vec<_> = sets(&free).pop().unwrap();

    let held = replay_paced(bytes, config(FAR_US, FAR_US), CHUNK, bps);
    let held_sets = sets(&held);
    assert_eq!(
        held_sets.len(),
        1,
        "{label}: one Tracks only: {held_sets:?}"
    );
    let mut got = held_sets[0].clone();
    got.sort_unstable();
    let mut want = final_set.clone();
    want.sort_unstable();
    assert_eq!(got, want, "{label}: every stream that resolved is listed");
    let at = held.tracks_times()[0];
    assert!(
        at >= last_resolution,
        "{label}: released at {at} before the last stream resolved at {last_resolution}"
    );
    assert!(
        at <= last_resolution + SETTLE_US,
        "{label}: released at {at}, later than last resolution {last_resolution} + settle"
    );
    assert!(at < FAR_US / 10, "{label}: nowhere near the wall limit");
    assert_eq!(
        replay_paced(bytes, config(0, 0), CHUNK, bps).samples(),
        held.samples(),
        "{label}: samples unchanged"
    );
}

#[test]
fn a_declared_pid_that_never_sends_does_not_hold_the_tracks() {
    let Some(b) = test_fixtures::require_bytes(TF1, "hold_early_release") else {
        return;
    };
    early_release_case("silent data pid", &without_pid(&b, TF1_LATE_DATA));
    early_release_case(
        "two silent data pids",
        &without_pid(&without_pid(&b, TF1_LATE_DATA), TF1_EARLY_DATA),
    );
    let bytes = without_pid(
        &without_pid(&without_pid(&b, TF1_LAST_AUDIO), TF1_LATE_DATA),
        TF1_EARLY_DATA,
    );
    early_release_case("silent audio pid", &bytes);
    let bps = bitrate_bps(&bytes);
    let held = replay_paced(&bytes, config(FAR_US, FAR_US), CHUNK, bps);
    let full = replay_paced(&b, config(FAR_US, FAR_US), CHUNK, bitrate_bps(&b));
    assert!(sets(&held)[0].len() < sets(&full)[0].len());
}

#[test]
fn a_stream_that_keeps_sending_but_never_resolves_is_bounded_by_the_wall_limit() {
    let Some(b) = test_fixtures::require_bytes(TF1, "hold_early_release") else {
        return;
    };
    let bytes = garbage(&b, TF1_LAST_AUDIO);
    let bps = bitrate_bps(&bytes);
    let free = replay_paced(&bytes, config(0, 0), CHUNK, bps);
    let first_media = free.first_sample_us().unwrap();
    let wall = 2 * SETTLE_US;
    let held = replay_paced(
        &bytes,
        {
            let mut new_config = config(wall, FAR_US);
            new_config.tracks_hold_quiet_input_bytes = bytes.len() / 2;
            new_config.tracks_hold_max_input_bytes = bytes.len();
            new_config
        },
        CHUNK,
        bps,
    );
    let at = held.tracks_times()[0];
    assert!(
        at >= first_media + wall,
        "released at {at}, before first media {first_media} + wall {wall}: the unresolved PID must be waited for"
    );
    assert!(at <= first_media + wall + 2 * CHUNK as u64 * 8 * 1_000_000 / bps);
}

fn drive(s: &mut Session, now: u64, wake: &mut Option<u64>, events: &mut Vec<Event>) {
    while let Some(a) = s.poll(ts(now)) {
        if let Action::WakeAt(t) = a {
            *wake = Some(t.as_micros());
        }
    }
    while let Some(e) = s.next_event() {
        events.push(e);
    }
}

fn last_resolution_chunk(bytes: &[u8], chunk: usize) -> usize {
    let free = replay_paced(bytes, config(0, 0), chunk, bitrate_bps(bytes));
    let t = *free.tracks_times().last().unwrap();
    let bps = bitrate_bps(bytes);
    let mut at = 0u64;
    for (i, part) in bytes.chunks(chunk).enumerate() {
        at += part.len() as u64 * 8 * 1_000_000 / bps;
        if at >= t {
            return i;
        }
    }
    0
}

#[test]
fn the_settle_is_armed_on_the_session_clock_when_the_source_goes_quiet() {
    let bytes = without_pid(&test_fixtures::read(FRANCE2), FRANCE2_SILENT);
    let chunk = 64 * 1024;
    let last_chunk = last_resolution_chunk(&bytes, chunk);
    let (mut s, c) = open_live(config(FAR_US, FAR_US));
    let mut now = 0;
    let mut wake = None;
    let mut events = Vec::new();
    let mut resolved_at = 0;
    for (i, part) in bytes.chunks(chunk).enumerate().take(last_chunk + 1) {
        now += STEP_US;
        s.on_data(c, part, ts(now));
        drive(&mut s, now, &mut wake, &mut events);
        if i == last_chunk {
            resolved_at = now;
        }
    }
    assert!(
        !events.iter().any(|e| matches!(e, Event::Tracks(_))),
        "the prefix ends at the last resolution: nothing to release yet"
    );
    let release_at = resolved_at + SETTLE_US;
    for _ in 0..MAX_WAKES {
        let due = wake.take().expect("a wake-up stays armed while holding");
        assert!(
            due <= release_at,
            "{due} is past the settle deadline {release_at}"
        );
        drive(&mut s, due, &mut wake, &mut events);
        let released = events.iter().any(|e| matches!(e, Event::Tracks(_)));
        assert_eq!(
            released,
            due == release_at,
            "wake at {due}, settle deadline {release_at}"
        );
        if released {
            assert!(events.iter().any(|e| matches!(e, Event::Sample { .. })));
            return;
        }
    }
    panic!("never released by {release_at}");
}

/// The track hold released by a wake-up (`poll`, not `on_data`) hands samples to the host; the
/// controller must learn their ends in that same `poll`, not at the next chunk.
#[test]
fn samples_released_by_a_wake_up_reach_the_controller_in_the_same_poll() {
    use broadcast_client::BufferInput;
    use std::collections::BTreeMap;

    let bytes = without_pid(&test_fixtures::read(FRANCE2), FRANCE2_SILENT);
    let chunk = 64 * 1024;
    let mut cfg = config(FAR_US, FAR_US);
    cfg.record_buffer_inputs = true;
    let (mut s, c) = open_live(cfg);
    let mut now = 0;
    let mut wake = None;
    let mut timescale: BTreeMap<u32, u32> = BTreeMap::new();
    let mut video: BTreeMap<u32, VideoRun> = BTreeMap::new();
    let mut is_video: BTreeMap<u32, bool> = BTreeMap::new();
    let mut host_end: BTreeMap<u32, u64> = BTreeMap::new();
    let mut reported: BTreeMap<u32, u64> = BTreeMap::new();
    let mut released = false;
    let mut collect = |s: &mut Session, now: u64, wake: &mut Option<u64>| {
        let mut got_tracks = false;
        while let Some(a) = s.poll(ts(now)) {
            if let Action::WakeAt(t) = a {
                *wake = Some(t.as_micros());
            }
        }
        while let Some(e) = s.next_event() {
            match e {
                Event::Tracks(specs) => {
                    got_tracks = true;
                    for t in specs {
                        timescale.insert(t.track_id.value(), t.timescale);
                        is_video.insert(t.track_id.value(), t.config.is_video());
                    }
                }
                Event::Sample { track, sample, .. } => {
                    let ts = u64::from(timescale[&track.value()]);
                    let pts = sample.pts.unwrap();
                    let dur = i64::from(sample.duration.unwrap_or(0));
                    let end = if is_video[&track.value()] {
                        video
                            .entry(track.value())
                            .or_insert_with(VideoRun::new)
                            .push(pts, sample.dts.unwrap_or(pts), dur)
                    } else {
                        pts + dur
                    };
                    let end = u64::try_from(end).unwrap();
                    let slot = host_end.entry(track.value()).or_insert(0);
                    *slot = (*slot).max(end * 27_000_000 / ts);
                }
                _ => {}
            }
        }
        for i in s.drain_buffer_inputs() {
            if let BufferInput::TrackMedia { track, end, .. } = i {
                let slot = reported.entry(track).or_insert(0);
                *slot = (*slot).max(end);
            }
        }
        got_tracks
    };
    for part in bytes
        .chunks(chunk)
        .take(last_resolution_chunk(&bytes, chunk) + 1)
    {
        now += STEP_US;
        s.on_data(c, part, ts(now));
        assert!(
            !collect(&mut s, now, &mut wake),
            "precondition: the hold is released by a wake-up, not by data"
        );
    }
    for _ in 0..MAX_WAKES {
        let due = wake.take().expect("a wake-up stays armed while holding");
        if collect(&mut s, due, &mut wake) {
            released = true;
            break;
        }
    }
    assert!(released, "precondition: the hold released on a wake-up");
    assert!(
        !reported.is_empty(),
        "the wake-up handed samples to the host but reported no end to the controller"
    );
    for (track, rep) in &reported {
        assert!(
            host_end[track] <= *rep,
            "track {track}: host holds media to {} but the controller was told {rep}",
            host_end[track]
        );
    }
}
