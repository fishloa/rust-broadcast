mod support;

use broadcast_client::{
    Action, BufferEvent, BufferInput, Config, Event, Hint, Session, Status, TrackId,
};
use support::ts_shift::{PACKET, Shift, VIDEO_TIMESCALE, pes_starts, pts27, shift_pes};
use support::{read_tf1_bytes, read_tf1_headers};
use transmux::JoinDecision;

const CHUNK: usize = 1316;
const STEP_US: u64 = 3_200;
const TICK_US: u64 = 50_000;
const TAIL_US: u64 = 8_000_000;
const URL: &str = "http://example/live.ts";
const PCR_PER_US: u64 = 27;

#[derive(Clone, Copy)]
enum ClockState {
    Idle,
    Playing { base: u64, since: u64 },
    Held(u64),
}

struct LiveHost {
    session: Session,
    conn: u32,
    clock: ClockState,
    events: Vec<Event>,
    times: Vec<u64>,
    now: u64,
    next_tick: u64,
}

impl LiveHost {
    fn open(config: Config) -> Self {
        let (status, headers) = read_tf1_headers();
        let mut session = Session::open(URL, config, 0).unwrap();
        let conn = loop {
            match session.poll(0).expect("first request") {
                Action::HttpRequest { conn, .. } => break conn,
                _ => continue,
            }
        };
        session.on_response(conn, status, &headers, 0);
        Self {
            session,
            conn,
            clock: ClockState::Idle,
            events: Vec::new(),
            times: Vec::new(),
            now: 0,
            next_tick: 0,
        }
    }

    fn position(&self, now: u64) -> Option<u64> {
        match self.clock {
            ClockState::Idle => None,
            ClockState::Held(p) => Some(p),
            ClockState::Playing { base, since } => {
                Some(base + now.saturating_sub(since) * PCR_PER_US)
            }
        }
    }

    fn drain(&mut self) {
        while let Some(e) = self.session.next_event() {
            if let Event::Buffer(b) = &e {
                self.clock = match b {
                    BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz } => {
                        ClockState::Playing {
                            base: *at_27mhz,
                            since: self.now,
                        }
                    }
                    BufferEvent::Pause { hold_at_27mhz, .. } => ClockState::Held(*hold_at_27mhz),
                    _ => self.clock,
                };
            }
            self.events.push(e);
            self.times.push(self.now);
        }
    }

    fn ticks_until(&mut self, now: u64) {
        while self.next_tick <= now {
            let t = self.next_tick;
            self.now = t;
            let p = self.position(t);
            self.session.on_playback_position(t, p);
            self.drain();
            self.next_tick += TICK_US;
        }
        self.now = now;
    }

    fn stream(&mut self, bytes: &[u8], from_us: u64) -> u64 {
        let mut at = from_us;
        for chunk in bytes.chunks(CHUNK) {
            self.ticks_until(at);
            self.session.on_data(self.conn, chunk, at);
            self.drain();
            at += STEP_US;
        }
        at
    }

    fn tail(&mut self, from_us: u64) {
        self.ticks_until(from_us + TAIL_US);
    }

    fn reconnect(&mut self, now: u64) {
        let (status, headers) = read_tf1_headers();
        self.ticks_until(now);
        self.session.on_closed(self.conn, None, now);
        let mut conn = None;
        while let Some(a) = self.session.poll(now) {
            if let Action::HttpRequest { conn: c, .. } = a {
                conn = Some(c);
            }
        }
        self.conn = conn.expect("a reconnect request");
        self.session.on_response(self.conn, status, &headers, now);
        self.drain();
    }

    fn marker(&self) -> usize {
        self.events.len()
    }

    fn log(&self) -> &[BufferInput] {
        self.session.buffer_input_log()
    }

    fn statuses_since(&self, idx: usize) -> Vec<Status> {
        self.events
            .iter()
            .skip(idx)
            .filter_map(|e| match e {
                Event::Status(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    fn buffer_events_since(&self, idx: usize) -> Vec<(usize, BufferEvent)> {
        self.events
            .iter()
            .enumerate()
            .skip(idx)
            .filter_map(|(i, e)| match e {
                Event::Buffer(b) => Some((i, b.clone())),
                _ => None,
            })
            .collect()
    }

    fn video(&self) -> TrackId {
        let mut found = None;
        for e in &self.events {
            if let Event::Tracks(specs) = e {
                for s in specs {
                    if s.timescale == VIDEO_TIMESCALE && found.is_none() {
                        found = Some(s.track_id);
                    }
                }
            }
        }
        found.expect("video track")
    }

    fn video_samples_since(&self, idx: usize) -> Vec<(usize, i64, Option<JoinDecision>)> {
        let vid = self.video();
        self.events
            .iter()
            .enumerate()
            .skip(idx)
            .filter_map(|(i, e)| match e {
                Event::Sample {
                    track,
                    sample,
                    join,
                } if *track == vid => Some((i, sample.pts.unwrap(), *join)),
                _ => None,
            })
            .collect()
    }
}

fn chunk_boundary_in_gop(bytes: &[u8]) -> usize {
    let starts = pes_starts(bytes, Shift::Video);
    let mid = starts[starts.len() / 3];
    (mid * PACKET / CHUNK) * CHUNK + CHUNK
}

fn after_discontinuity(h: &LiveHost, since: usize) -> usize {
    h.events
        .iter()
        .enumerate()
        .skip(since)
        .find_map(|(i, e)| matches!(e, Event::Discontinuity).then_some(i))
        .expect("a Discontinuity for the reconnect")
}

fn first_join(h: &LiveHost, since: usize) -> (usize, i64) {
    h.video_samples_since(since)
        .into_iter()
        .find_map(|(i, pts, j)| matches!(j, Some(JoinDecision::JoinPoint(_))).then_some((i, pts)))
        .expect("a join point after the reconnect")
}

fn go_events(h: &LiveHost, since: usize) -> Vec<(u64, u64)> {
    h.buffer_events_since(since)
        .into_iter()
        .filter_map(|(i, b)| match b {
            BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz } => {
                Some((at_27mhz, h.times[i]))
            }
            _ => None,
        })
        .collect()
}

struct Reconnected {
    host: LiveHost,
    mark: usize,
    reconnected_at: u64,
}

fn lower_base_reconnect(from_chunk: usize) -> Reconnected {
    let bytes = read_tf1_bytes();
    let mut h = LiveHost::open(Config::default());
    let end = h.stream(&bytes[..1_500 * CHUNK], 0);
    h.ticks_until(end);
    let mark = h.marker();
    let reconnected_at = end + 1_000;
    h.reconnect(reconnected_at);
    let end2 = h.stream(&bytes[from_chunk * CHUNK..], end + 1_000);
    h.tail(end2);
    Reconnected {
        host: h,
        mark,
        reconnected_at,
    }
}

#[test]
fn a_reconnect_onto_a_lower_pts_base_starts_playing_on_the_new_timeline() {
    let r = lower_base_reconnect(0);
    let h = &r.host;
    let (_, join_pts) = first_join(h, r.mark);
    let origin = pts27(join_pts, VIDEO_TIMESCALE);
    let go = go_events(h, r.mark);
    assert!(!go.is_empty(), "no Start/Resume after the reconnect");
    assert!(
        go.iter()
            .all(|(at, _)| *at >= origin && *at - origin < 60 * PCR_HZ_U),
        "go positions {go:?} must lie on the new timeline starting at {origin}"
    );
    assert!(
        go[0].1 <= r.reconnected_at + GO_DEADLINE_US,
        "first go at {} us, reconnect at {}",
        go[0].1,
        r.reconnected_at
    );
    let reports: Vec<u64> = h
        .log()
        .iter()
        .skip_while(|l| {
            !matches!(
                l,
                BufferInput::Hint {
                    hint: Hint::Reconnected,
                    ..
                }
            )
        })
        .filter_map(|l| match l {
            BufferInput::TrackMedia { end, .. } => Some(*end),
            _ => None,
        })
        .collect();
    assert!(
        reports.iter().all(|e| *e >= origin),
        "an end of the old timeline reached the controller after the reconnect: {reports:?}"
    );
}

const PCR_HZ_U: u64 = 27_000_000;
const GO_DEADLINE_US: u64 = 5_000_000;

#[test]
fn a_played_reconnect_reports_attempt_zero_then_live_without_a_climbing_counter() {
    let r = lower_base_reconnect(0);
    let attempts: Vec<u64> = r
        .host
        .statuses_since(r.mark)
        .into_iter()
        .filter_map(|s| match s {
            Status::Reconnecting { attempt } => Some(attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![0], "{attempts:?}");
    let statuses = r.host.statuses_since(r.mark);
    let pos = |want: &Status| statuses.iter().position(|s| s == want);
    assert!(
        pos(&Status::Live).is_some() && pos(&Status::Establishing).is_some(),
        "{statuses:?}"
    );
    assert!(
        pos(&Status::Reconnecting { attempt: 0 }) < pos(&Status::Establishing),
        "{statuses:?}"
    );
    assert!(
        pos(&Status::Establishing) < pos(&Status::Live),
        "{statuses:?}"
    );
}

#[test]
fn a_reconnect_mid_gop_drops_video_until_a_random_access_point() {
    let bytes = read_tf1_bytes();
    let cut = chunk_boundary_in_gop(&bytes) / CHUNK;
    let r = lower_base_reconnect(cut);
    let vs = r
        .host
        .video_samples_since(after_discontinuity(&r.host, r.mark));
    let first = vs.first().expect("video after the reconnect");
    assert!(
        matches!(first.2, Some(JoinDecision::Drop(_))),
        "first video sample after the reconnect must be dropped until an IDR: {:?}",
        first.2
    );
    assert!(
        vs.iter()
            .any(|v| matches!(v.2, Some(JoinDecision::JoinPoint(_)))),
        "and a join point must follow"
    );
}

fn reconnect_with(first: &[u8], second: &[u8], idle_us: u64) -> Reconnected {
    let mut h = LiveHost::open(Config::default());
    let end = h.stream(first, 0);
    h.ticks_until(end + idle_us);
    let mark = h.marker();
    let reconnected_at = end + idle_us + 1_000;
    h.reconnect(reconnected_at);
    let end2 = h.stream(second, reconnected_at);
    h.tail(end2);
    Reconnected {
        host: h,
        mark,
        reconnected_at,
    }
}

fn assert_goes_on_new_timeline(r: &Reconnected) {
    let h = &r.host;
    let (_, join_pts) = first_join(h, after_discontinuity(h, r.mark));
    let origin = pts27(join_pts, VIDEO_TIMESCALE);
    println!(
        "DBG origin={origin} at={} ev={:?}",
        r.reconnected_at,
        h.buffer_events_since(r.mark)
            .iter()
            .map(|(i, b)| (h.times[*i], b.clone()))
            .collect::<Vec<_>>()
    );
    let go = go_events(h, r.mark);
    assert!(!go.is_empty(), "no Start/Resume after the reconnect");
    assert!(
        go.iter()
            .all(|(at, _)| *at >= origin && *at - origin < 60 * PCR_HZ_U),
        "go positions {go:?} must lie on the new timeline starting at {origin}"
    );
    assert!(
        go[0].1 <= r.reconnected_at + GO_DEADLINE_US,
        "first go at {} us, reconnect at {}",
        go[0].1,
        r.reconnected_at
    );
}

const STARVE_IDLE_US: u64 = 4_000_000;
const ADVANCE_TICKS: i64 = 40 * 90_000;

#[test]
fn a_reconnect_of_a_starved_session_resumes_on_the_new_timeline() {
    let bytes = read_tf1_bytes();
    let r = reconnect_with(&bytes[..1_500 * CHUNK], &bytes, STARVE_IDLE_US);
    assert!(
        r.host
            .buffer_events_since(0)
            .iter()
            .any(|(i, b)| *i < r.mark && matches!(b, BufferEvent::Pause { .. })),
        "the idle gap must have starved the controller"
    );
    assert_goes_on_new_timeline(&r);
}

#[test]
fn a_reconnect_onto_an_advanced_timeline_resumes_on_the_new_timeline() {
    let bytes = read_tf1_bytes();
    let advanced = shift_pes(&bytes, 0, ADVANCE_TICKS, Shift::Both);
    let r = reconnect_with(&bytes[..1_500 * CHUNK], &advanced, 0);
    assert_goes_on_new_timeline(&r);
}

const FINITE_LEN: usize = 2_000 * PACKET;
const FINITE_PLAY_TICKS: u64 = 30 * PCR_HZ_U;

fn finite_pass(
    session: &mut Session,
    conn: u32,
    bytes: &[u8],
    now: u64,
    host_position: Option<u64>,
) -> Vec<Event> {
    let mut headers = read_tf1_headers().1;
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    session.on_response(conn, 200, &headers, now);
    let mut out = Vec::new();
    let mut at = now;
    let mut position = host_position;
    let mut playing_since: Option<(u64, u64)> = None;
    for chunk in bytes.chunks(CHUNK) {
        let reported = match playing_since {
            Some((base, since)) => Some(base + (at - since) * PCR_PER_US),
            None => position,
        };
        session.on_playback_position(at, reported);
        session.on_data(conn, chunk, at);
        while let Some(e) = session.next_event() {
            if let Event::Buffer(
                BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz },
            ) = &e
            {
                playing_since = Some((*at_27mhz, at));
                position = Some(*at_27mhz);
            }
            out.push(e);
        }
        at += STEP_US * 4;
    }
    session.on_closed(conn, None, at);
    while let Some(e) = session.next_event() {
        out.push(e);
    }
    out
}

fn request_conn(session: &mut Session, now: u64) -> u32 {
    let mut conn = None;
    while let Some(a) = session.poll(now) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    conn.expect("an HttpRequest")
}

fn starts(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Buffer(BufferEvent::Start { at_27mhz }) => Some(*at_27mhz),
            _ => None,
        })
        .collect()
}

#[test]
fn a_finite_ts_that_is_looped_with_reconnect_starts_the_gate_again() {
    let bytes = read_tf1_bytes();
    finite_loop(&bytes[..FINITE_LEN]);
}

#[test]
fn a_looped_small_finite_ts_is_not_paused_by_the_hosts_stale_position() {
    finite_loop(&test_fixtures::read("ts/h264_aac.ts"));
}

fn finite_loop(file: &[u8]) {
    let mut session = Session::open(URL, Config::default(), 0).unwrap();
    let conn = request_conn(&mut session, 0);
    let first = finite_pass(&mut session, conn, file, 0, None);
    assert!(
        first
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Ended))),
        "the first pass ends"
    );
    let first_starts = starts(&first);
    assert_eq!(first_starts.len(), 1, "first pass starts once");
    let end_pos = first_starts[0] + FINITE_PLAY_TICKS;
    for t in 1..=40u64 {
        session.on_playback_position(
            t * TICK_US,
            Some(first_starts[0] + t * TICK_US * PCR_PER_US),
        );
    }
    session.on_playback_position(3_000_000, Some(end_pos));
    while session.next_event().is_some() {}

    session.reconnect(3_100_000);
    let conn = request_conn(&mut session, 3_100_000);
    let stale = Some(end_pos);
    let second = finite_pass(&mut session, conn, file, 3_100_000, stale);
    let statuses: Vec<Status> = second
        .iter()
        .filter_map(|e| match e {
            Event::Status(s) => Some(*s),
            _ => None,
        })
        .collect();
    let pos = |want: Status| statuses.iter().position(|s| *s == want);
    assert!(
        pos(Status::Establishing).is_some() && pos(Status::Establishing) < pos(Status::Live),
        "{statuses:?}"
    );
    assert!(
        !statuses
            .iter()
            .any(|s| matches!(s, Status::Reconnecting { .. })),
        "a caller-initiated reconnect is not a failed attempt: {statuses:?}"
    );
    let new_starts = starts(&second);
    assert_eq!(
        new_starts,
        first_starts,
        "the looped pass starts again from the same origin: {second_len}",
        second_len = second.len()
    );
    let d = second
        .iter()
        .position(|e| matches!(e, Event::Discontinuity))
        .expect("a Discontinuity for the loop");
    let s = second
        .iter()
        .position(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
        .unwrap();
    assert!(d < s, "Start follows the Discontinuity");
    assert!(
        !second
            .iter()
            .any(|e| matches!(e, Event::Buffer(BufferEvent::Pause { .. }))),
        "a position the host reported before Start must not pause the new pass: {second:?}"
    );
}

const WRAP_HALF_TICKS: i64 = 1 << 32;
const LOW_BASE_TICKS: i64 = 60 * 90_000;
const NEGATIVE_JUMP_EXTRA_TICKS: i64 = 10 * 90_000;

fn first_video_pts(bytes: &[u8]) -> i64 {
    let mut h = LiveHost::open(Config::default());
    h.stream(&bytes[..CHUNK * 400], 0);
    h.video_samples_since(0)[0].1
}

fn negative_timeline_stream() -> (Vec<u8>, usize) {
    let bytes = read_tf1_bytes();
    let base = first_video_pts(&bytes);
    let low = shift_pes(&bytes, 0, LOW_BASE_TICKS - base, Shift::Both);
    let starts = pes_starts(&low, Shift::Video);
    let from = starts[starts.len() / 2];
    let jumped = shift_pes(
        &low,
        from,
        WRAP_HALF_TICKS + NEGATIVE_JUMP_EXTRA_TICKS,
        Shift::Both,
    );
    (jumped, from * PACKET / CHUNK)
}

#[test]
fn a_jump_to_a_negative_unrolled_timeline_keeps_the_controller_fed() {
    let (stream, _) = negative_timeline_stream();
    let mut h = LiveHost::open(Config::default());
    let end = h.stream(&stream, 0);
    h.tail(end);
    let d = h
        .events
        .iter()
        .position(|e| matches!(e, Event::Discontinuity))
        .expect("the jump is detected");
    let vs = h.video_samples_since(d);
    let negative = vs.iter().filter(|(_, pts, _)| *pts < 0).count();
    assert!(
        negative > 10,
        "the new timeline must unroll below zero: {negative} negative of {}",
        vs.len()
    );
    let reports: Vec<u64> = h
        .log()
        .iter()
        .skip_while(|l| {
            !matches!(
                l,
                BufferInput::Hint {
                    hint: Hint::PcrDiscontinuity,
                    ..
                }
            )
        })
        .filter_map(|l| match l {
            BufferInput::TrackMedia { track, end, .. } if *track == h.video().value() => Some(*end),
            _ => None,
        })
        .collect();
    let spread = reports.iter().max().unwrap() - reports.iter().min().unwrap();
    assert!(
        spread > 2 * PCR_HZ_U,
        "video ends after the jump must advance, not sit clamped: spread {spread} of {reports:?}"
    );
    let go = go_events(&h, d);
    assert!(!go.is_empty(), "no Start/Resume after the jump");
    assert!(
        go[0].1 <= h.times[d] + GO_DEADLINE_US,
        "first go at {} us, jump at {}",
        go[0].1,
        h.times[d]
    );
}

const JUMP_SECONDS: i64 = 30;
const CONSISTENCY_WINDOW_SECONDS: i64 = 5;

fn audio_track_of(h: &LiveHost) -> (TrackId, u32) {
    let mut found = None;
    for e in &h.events {
        if let Event::Tracks(specs) = e {
            for s in specs {
                if s.timescale != VIDEO_TIMESCALE && found.is_none() {
                    found = Some((s.track_id, s.timescale));
                }
            }
        }
    }
    found.expect("an audio track")
}

#[test]
fn no_old_timeline_audio_follows_a_detected_jump() {
    let bytes = read_tf1_bytes();
    let starts = pes_starts(&bytes, Shift::Video);
    let from = starts[starts.len() / 2];
    let shifted = shift_pes(&bytes, from, JUMP_SECONDS * 90_000, Shift::Both);
    let mut h = LiveHost::open(Config::default());
    let end = h.stream(&shifted, 0);
    h.tail(end);
    let d = h
        .events
        .iter()
        .position(|e| matches!(e, Event::Discontinuity))
        .expect("the jump is detected");
    let vid = h.video();
    let first_new_video = h.video_samples_since(d)[0].1;
    let (audio, audio_ts) = audio_track_of(&h);
    let new_video_us = first_new_video * 1_000_000 / i64::from(VIDEO_TIMESCALE);
    let after: Vec<i64> = h.events[d..]
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } if *track == audio => sample.pts,
            _ => None,
        })
        .collect();
    assert!(!after.is_empty(), "the new timeline carries audio");
    let stale: Vec<i64> = after
        .iter()
        .copied()
        .filter(|pts| {
            let us = pts * 1_000_000 / i64::from(audio_ts);
            (us - new_video_us).abs() > CONSISTENCY_WINDOW_SECONDS * 1_000_000
        })
        .collect();
    assert!(
        stale.is_empty(),
        "audio samples of the old timeline after the Discontinuity (video {vid:?} at {first_new_video}): {stale:?}"
    );
    assert!(h.session.stale_samples_dropped() > 0, "the drop is counted");
}
