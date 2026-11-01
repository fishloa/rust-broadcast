//! Round-7 finding F3: `Event::Buffer(Start)` on live TS does not depend on the host calling
//! `on_playback_position`; the ticks only make the decision earlier.

use crate::support;
use broadcast_client::{BufferEvent, Config, Event, Session};
use support::replay::{TS_PACKET, open_live};

const RAI: &str = "zenith-live/rai-gulp-8MB.ts";
const AUDIO_PID: u16 = 0x130;
const CHUNK: usize = 4_096;
const CHUNK_STEP_US: u64 = 3_000;
const IDLE_TICKS: usize = 400;
const IDLE_STEP_US: u64 = 50_000;

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1]) << 8 | u16::from(p[2])) & 0x1fff
}

fn without(bytes: &[u8], pid: u16) -> Vec<u8> {
    bytes
        .chunks(TS_PACKET)
        .filter(|p| pid_of(p) != pid)
        .flatten()
        .copied()
        .collect()
}

fn rai() -> Option<Vec<u8>> {
    test_fixtures::require(RAI, "r7").map(|path| std::fs::read(path).unwrap())
}

fn drain(s: &mut Session, now: u64, starts: &mut Vec<u64>) {
    while s.poll(now).is_some() {}
    while let Some(e) = s.next_event() {
        if matches!(e, Event::Buffer(BufferEvent::Start { .. })) {
            starts.push(now);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ticks {
    Never,
    IdleOnly,
    EveryChunk,
}

fn start_times(bytes: &[u8], cfg: Config, ticks: Ticks) -> Vec<u64> {
    let (mut s, c) = open_live(cfg);
    let mut starts = Vec::new();
    let mut now = 0u64;
    for part in bytes.chunks(CHUNK) {
        now += CHUNK_STEP_US;
        s.on_data(c, part, now);
        if ticks == Ticks::EveryChunk {
            s.on_playback_position(now, None);
        }
        drain(&mut s, now, &mut starts);
    }
    for _ in 0..IDLE_TICKS {
        now += IDLE_STEP_US;
        if ticks != Ticks::Never {
            s.on_playback_position(now, None);
        }
        drain(&mut s, now, &mut starts);
    }
    starts
}

#[test]
fn start_needs_at_least_one_playback_position_call() {
    let starts = start_times(&skip_none!(rai()), Config::default(), Ticks::Never);
    assert!(starts.is_empty(), "{starts:?}");
}

#[test]
fn start_fires_with_sparse_ticks_just_later_than_with_every_tick() {
    let bytes = without(&skip_none!(rai()), AUDIO_PID);
    let sparse = start_times(&bytes, Config::default(), Ticks::IdleOnly);
    let every = start_times(&bytes, Config::default(), Ticks::EveryChunk);
    eprintln!("{sparse:?} {every:?}");
    assert_eq!(sparse.len(), 1);
    assert_eq!(every.len(), 1);
    assert!(every[0] < sparse[0], "{every:?} {sparse:?}");
}

#[test]
fn start_fires_with_sparse_ticks_on_a_complete_stream() {
    let starts = start_times(&skip_none!(rai()), Config::default(), Ticks::IdleOnly);
    assert_eq!(starts.len(), 1, "{starts:?}");
}
