use crate::support::replay::{TS_PACKET, bitrate_bps, open_live};

use broadcast_client::{
    BufferEvent, BufferInput, Config, ErrorKind, Event, InputLog, PauseReason, Session,
};
use broadcast_common::stage::Timestamp;
use buffering_runtime::Event as RtEvent;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const FIXTURE: &str = "ts/h264-aac-40s.ts";
const CHUNK: usize = TS_PACKET * 22;
const TICK_US: u64 = 50_000;
const MICROS: u64 = 1_000_000;
const TICKS_PER_US: u64 = 27;
const SMALL_CAP: usize = 8;

struct Clock {
    anchor_pos: u64,
    anchor_us: u64,
    running: bool,
}

impl Clock {
    fn position(&self, now: u64) -> u64 {
        if self.running {
            self.anchor_pos + (now - self.anchor_us) * TICKS_PER_US
        } else {
            self.anchor_pos
        }
    }

    fn set(&mut self, now: u64, pos: u64, running: bool) {
        *self = Self {
            anchor_pos: pos,
            anchor_us: now,
            running,
        };
    }
}

struct Run {
    session: Session,
    config: Config,
    inputs: Vec<BufferInput>,
    events: Vec<Event>,
    buffer_events: Vec<BufferEvent>,
}

fn run(config: Config, drain: bool) -> Run {
    let bytes = test_fixtures::read(FIXTURE);
    let bps = bitrate_bps(&bytes);
    let (mut session, conn) = open_live(config.clone());
    let mut out = Run {
        session: open_placeholder(),
        config,
        inputs: Vec::new(),
        events: Vec::new(),
        buffer_events: Vec::new(),
    };
    let mut clock = Clock {
        anchor_pos: 0,
        anchor_us: 0,
        running: false,
    };
    let mut started = false;
    let mut chunks = bytes.chunks(CHUNK).peekable();
    let (mut now, mut next_tick, mut next_chunk) = (0u64, 0u64, 0u64);
    while chunks.peek().is_some() || now < next_chunk + 3 * MICROS {
        let chunk_due = chunks.peek().is_some();
        if chunk_due && next_chunk <= next_tick {
            now = next_chunk;
            let part = chunks.next().unwrap();
            session.on_data(conn, part, ts(now));
            next_chunk = now + part.len() as u64 * 8 * MICROS / bps;
        } else {
            now = next_tick;
            let pos = started.then(|| clock.position(now));
            session.on_playback_position(ts(now), pos);
            next_tick += TICK_US;
        }
        while let Some(e) = session.next_event() {
            if let Event::Buffer(b) = &e {
                match b {
                    BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz } => {
                        started = true;
                        clock.set(now, *at_27mhz, true);
                    }
                    BufferEvent::Pause { hold_at_27mhz, .. } => {
                        clock.set(now, *hold_at_27mhz, false);
                    }
                    _ => {}
                }
                out.buffer_events.push(b.clone());
            }
            out.events.push(e);
        }
        if drain {
            out.inputs.extend(session.drain_buffer_inputs());
        }
        if chunks.peek().is_none() && now >= next_chunk {
            break;
        }
    }
    out.session = session;
    out
}

fn open_placeholder() -> Session {
    Session::open("http://placeholder/live.ts", Config::default(), ts(0)).unwrap()
}

fn recording() -> Config {
    let mut c = Config::default();
    c.record_buffer_inputs = true;
    c.buffer.policy.initial_us = 500_000;
    c
}

fn written_log(r: &Run) -> String {
    let log = InputLog::new(
        r.session.buffer_input_log_config(),
        r.session.buffer_input_log_seed(),
        r.inputs.clone(),
    );
    let mut text = String::new();
    log.write(&mut text).unwrap();
    text
}

fn replayed_buffer_events(text: &str) -> Vec<BufferEvent> {
    InputLog::parse(text)
        .unwrap()
        .replay()
        .unwrap()
        .into_iter()
        .filter_map(|(_, e)| match e {
            RtEvent::StartPlaying { media_27mhz, .. } => Some(BufferEvent::Start {
                at_27mhz: media_27mhz,
            }),
            RtEvent::Resume { media_27mhz, .. } => Some(BufferEvent::Resume {
                at_27mhz: media_27mhz,
            }),
            RtEvent::Pause {
                hold_at_27mhz,
                reason,
                track,
                ..
            } => Some(BufferEvent::Pause {
                hold_at_27mhz,
                reason: match reason {
                    buffering_runtime::Reason::NoData => PauseReason::NoData,
                    buffering_runtime::Reason::DataArriving => PauseReason::DataArriving,
                    buffering_runtime::Reason::Discontinuity => PauseReason::Discontinuity,
                    buffering_runtime::Reason::Reprime => PauseReason::Reprime,
                    _ => PauseReason::NoData,
                },
                starving_track: track,
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn a_written_log_replays_to_the_sessions_own_buffering_events() {
    let r = run(recording(), true);
    assert!(
        r.buffer_events
            .iter()
            .any(|e| matches!(e, BufferEvent::Start { .. })),
        "the run must reach Start or the comparison is vacuous"
    );
    assert_eq!(r.session.dropped_buffer_inputs(), 0);
    let text = written_log(&r);
    assert_eq!(replayed_buffer_events(&text), r.buffer_events);
}

#[test]
fn the_log_carries_every_row_kind_and_the_sessions_config() {
    let r = run(recording(), true);
    let text = written_log(&r);
    for kind in ["chunk,", "media,", "tick,", "register,", "config,"] {
        assert!(
            text.lines().any(|l| l.starts_with(kind)),
            "no {kind} row in the log"
        );
    }
    let log = InputLog::parse(&text).unwrap();
    assert_eq!(log.config, r.config.controller_config());
    assert_eq!(log.inputs, r.inputs);
}

#[test]
fn nothing_is_recorded_unless_asked() {
    let mut config = recording();
    config.record_buffer_inputs = false;
    let r = run(config, true);
    assert!(r.inputs.is_empty());
    assert_eq!(r.session.dropped_buffer_inputs(), 0);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
    );
}

#[test]
fn a_full_queue_drops_counts_and_reports_once() {
    let mut config = recording();
    config.max_recorded_buffer_inputs = SMALL_CAP;
    let r = run(config, false);
    assert!(r.session.dropped_buffer_inputs() > 0);
    let notices = r
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Error {
                    kind: ErrorKind::BufferInputsDropped,
                    fatal: false
                }
            )
        })
        .count();
    assert_eq!(notices, 1);
    let mut session = r.session;
    assert_eq!(session.drain_buffer_inputs().count(), SMALL_CAP);
}

#[test]
fn a_zero_cap_is_rejected() {
    let mut config = Config::default();
    config.max_recorded_buffer_inputs = 0;
    assert!(Session::open("http://h/live.ts", config, ts(0)).is_err());
}
