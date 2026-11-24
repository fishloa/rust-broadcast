//! Read-ahead backpressure memory bound (issue #4085): a TS recording far
//! larger than the budget must play through a host that honours
//! `PauseRead`/`ResumeRead` in bounded memory.
//!
//! The capture is generated incrementally at test time (a quarter-gigabyte
//! fixture cannot be committed); the process-wide peak allocator reports the
//! live-byte high-water mark of the whole play loop. Set
//! `READ_AHEAD_IGNORE_PAUSE=1` to run the same driver without backpressure:
//! the test then fails and prints the (much larger) peak.

#![cfg(target_pointer_width = "64")]

use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::Mutex;
use std::time::Duration;

use broadcast_client::{Action, Config, ConnId, Event, ReadAhead, Session, Status};
use broadcast_common::clock33::Clock27;
use broadcast_common::stage::Timestamp;
use url::Url;

#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();
/// The allocator is process-wide, so the two measuring windows must not overlap.
static WINDOW: Mutex<()> = Mutex::new(());

/// The generated capture's size: big enough that reading it whole blows the
/// budget several times over.
const FILE_BYTES: u64 = 256 << 20;
/// Live bytes the play loop may hold: the read-ahead cap plus the host's own
/// read buffer, never the file.
const BUDGET: usize = 96 << 20;
/// Host read size.
const CHUNK: usize = 64 * 1024;
/// Simulated host clock step and the matching 1x playback advance.
const TICK_US: u64 = 100_000;
const TICK_TICKS: u64 = 2_700_000;
/// A loop that cannot reach `Ended` is broken, not slow.
const MAX_TICKS: usize = 4_000_000;
/// Test-only queue ceilings: the point of the test is that the read-ahead cap,
/// not the queue's safety net, bounds memory, so the platform file must be able
/// to queue the whole capture when the host ignores the pause.
const QUEUE_EVENTS: usize = 2_000_000;
const QUEUE_BYTES: usize = 1 << 30;
/// Byte-only case: the read-ahead cap alone must keep the peak under `BUDGET`.
const BYTE_CAP: usize = 32 << 20;

fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

fn clock(ticks: u64) -> Clock27 {
    Clock27::from_27mhz_wrapping(ticks)
}

fn headers(len: u64) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    map.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("video/MP2T"),
    );
    map.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&len.to_string()).unwrap(),
    );
    map
}

fn drain_actions(session: &mut Session, now: Timestamp, conn: ConnId, paused: &mut bool) {
    while let Some(action) = session.poll(now) {
        match action {
            Action::PauseRead { conn: named } if named == conn => *paused = true,
            Action::ResumeRead { conn: named } if named == conn => *paused = false,
            Action::HttpRequest { .. } => panic!("a finite body must not restart its request"),
            _ => {}
        }
    }
}

/// Run the play loop, returning the live bytes held above the entry total and
/// the number of samples delivered.
fn capped(mut config: Config) -> Config {
    config.max_queued_events = QUEUE_EVENTS;
    config.max_queued_bytes = QUEUE_BYTES;
    config.timeline_jump = None;
    config
}

fn time_capped() -> Config {
    let mut config = Config::default();
    config.ts_http = config
        .ts_http
        .with_read_ahead(ReadAhead::time_and_bytes(Duration::from_secs(30), 64 << 20));
    capped(config)
}

fn byte_capped() -> Config {
    let mut config = Config::default();
    config.ts_http = config
        .ts_http
        .with_read_ahead(ReadAhead::bytes(BYTE_CAP as u64));
    capped(config)
}

fn play(
    path: &std::path::Path,
    config: Config,
    honour_pause: bool,
    expected_bytes: u64,
) -> (usize, u64) {
    let mut session = Session::open(
        Url::parse("http://example.invalid/record.ts").unwrap(),
        config,
        ts(0),
    )
    .unwrap();
    let conn = loop {
        match session.poll(ts(0)) {
            Some(Action::HttpRequest { conn, .. }) => {
                session.on_response(conn, http::StatusCode::OK, &headers(expected_bytes), ts(0));
                break conn;
            }
            Some(_) => continue,
            None => panic!("the session issues a request"),
        }
    };
    session.on_playback_position(ts(0), Some(clock(0)));

    let mut file = BufReader::new(File::open(path).unwrap());
    let mut buffer = vec![0u8; CHUNK];
    let mut now_us = 0u64;
    let mut position_ticks = 0u64;
    let mut paused = false;
    let mut eof = false;
    let mut samples = 0u64;
    let mut ended = false;

    A.reset();
    let baseline = A.live();

    for _ in 0..MAX_TICKS {
        while !eof && (!honour_pause || !paused) {
            let read = file.read(&mut buffer).unwrap();
            if read == 0 {
                eof = true;
                session.on_closed(conn, None, ts(now_us));
                drain_actions(&mut session, ts(now_us), conn, &mut paused);
                break;
            }
            session.on_data(conn, &buffer[..read], ts(now_us));
            drain_actions(&mut session, ts(now_us), conn, &mut paused);
        }

        now_us += TICK_US;
        position_ticks += TICK_TICKS;
        session.on_playback_position(ts(now_us), Some(clock(position_ticks)));
        drain_actions(&mut session, ts(now_us), conn, &mut paused);
        while let Some(event) = session.next_event() {
            match event {
                Event::Sample { .. } => samples += 1,
                Event::Status(Status::Ended) => ended = true,
                _ => {}
            }
        }
        drain_actions(&mut session, ts(now_us), conn, &mut paused);
        if ended {
            break;
        }
    }
    assert!(ended, "the finite capture must reach Ended");

    let held = A.peak().saturating_sub(baseline);
    (held, samples)
}

#[test]
fn a_big_ts_plays_in_bounded_memory_with_backpressure() {
    let _window = WINDOW.lock().unwrap();
    let honour_pause = std::env::var_os("READ_AHEAD_IGNORE_PAUSE").is_none();
    let dir = std::env::temp_dir().join(format!(
        "broadcast-client-read-ahead-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("big.ts");
    let made = test_harness::big_media::write_big_ts(
        &path,
        &test_fixtures::path("ts/h264-aac-40s.ts"),
        FILE_BYTES,
    );
    let expected: u64 = made.samples_per_track.iter().map(|(_, n)| *n).sum();

    let (held, samples) = play(&path, time_capped(), honour_pause, made.bytes);
    eprintln!(
        "peak {held} bytes above baseline for a {} -byte TS capture ({samples} samples, honour={honour_pause})",
        made.bytes
    );

    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(samples, expected, "every sample of the capture must play");
    assert!(
        held < BUDGET,
        "playing a {}-byte file held {held} live bytes, over the {BUDGET} budget \
         (set READ_AHEAD_IGNORE_PAUSE to see the unbounded peak)",
        made.bytes
    );
}

#[test]
fn a_big_ts_plays_in_bounded_memory_with_only_the_byte_cap() {
    let _window = WINDOW.lock().unwrap();
    let honour_pause = std::env::var_os("READ_AHEAD_IGNORE_PAUSE").is_none();
    let dir = std::env::temp_dir().join(format!(
        "broadcast-client-read-ahead-bytes-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("big.ts");
    let made = test_harness::big_media::write_big_ts(
        &path,
        &test_fixtures::path("ts/h264-aac-40s.ts"),
        FILE_BYTES,
    );
    let expected: u64 = made.samples_per_track.iter().map(|(_, n)| *n).sum();

    let (held, samples) = play(&path, byte_capped(), honour_pause, made.bytes);
    eprintln!(
        "peak {held} bytes above baseline for a {} -byte TS capture with only the {BYTE_CAP}-byte cap ({samples} samples, honour={honour_pause})",
        made.bytes
    );

    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(samples, expected, "every sample of the capture must play");
    assert!(
        held < BUDGET,
        "byte-only read-ahead held {held} live bytes, over the {BUDGET} budget \
         (set READ_AHEAD_IGNORE_PAUSE to see the unbounded peak)"
    );
}
