mod support;

use broadcast_client::{Action, Config, Event, Session, TrackId};
use support::replay::{TS_PACKET, bitrate_bps, open_live, replay_paced};

const CHUNK: usize = 4096;
const SETTLE_US: u64 = 500_000;
const FAR_US: u64 = 60_000_000;
const TF1: &str = "zenith-http/tf1-live.ts";
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
    while let Some(a) = s.poll(now) {
        if let Action::WakeAt(t) = a {
            *wake = Some(t);
        }
    }
    while let Some(e) = s.next_event() {
        events.push(e);
    }
}

#[test]
fn the_settle_is_armed_on_the_session_clock_when_the_source_goes_quiet() {
    let bytes = without_pid(&test_fixtures::read(FRANCE2), FRANCE2_SILENT);
    let chunk = 64 * 1024;
    let free = replay_paced(&bytes, config(0, 0), chunk, bitrate_bps(&bytes));
    let last_chunk = {
        let t = *free.tracks_times().last().unwrap();
        let bps = bitrate_bps(&bytes);
        let mut at = 0u64;
        let mut idx = 0usize;
        for (i, part) in bytes.chunks(chunk).enumerate() {
            at += part.len() as u64 * 8 * 1_000_000 / bps;
            if at >= t {
                idx = i;
                break;
            }
        }
        idx
    };
    let (mut s, c) = open_live(config(FAR_US, FAR_US));
    let mut now = 0;
    let mut wake = None;
    let mut events = Vec::new();
    let mut resolved_at = 0;
    for (i, part) in bytes.chunks(chunk).enumerate().take(last_chunk + 1) {
        now += STEP_US;
        s.on_data(c, part, now);
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
