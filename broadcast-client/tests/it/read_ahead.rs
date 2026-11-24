//! Read-ahead backpressure (issue #4085): a TS-over-HTTP session pauses and
//! resumes host reads around a media-time and a byte cap.

use std::collections::BTreeMap;
use std::time::Duration;

use broadcast_client::{
    Action, BufferEvent, Config, ConnId, Error, Event, PauseReason, ReadAhead, Session, Status,
};
use broadcast_common::clock33::{Clock27, PCR_HZ};
use broadcast_common::stage::Timestamp;
use url::Url;

use crate::support;

/// Chunk size the tests feed, matching a typical host read.
const CHUNK: usize = 64 * 1024;

fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

fn milliseconds(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn seconds(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

fn fixture() -> Vec<u8> {
    test_fixtures::read("ts/h264-aac-40s.ts")
}

fn open(config: Config) -> Session {
    Session::open(
        Url::parse("http://example.invalid/record.ts").unwrap(),
        config,
        ts(0),
    )
    .unwrap()
}

fn limited(read_ahead: ReadAhead) -> Config {
    let mut config = Config::default();
    config.ts_http = config.ts_http.with_read_ahead(read_ahead);
    config
}

fn config_error(config: Config) -> Error {
    Session::open(
        Url::parse("http://example.invalid/live.ts").unwrap(),
        config,
        ts(0),
    )
    .unwrap_err()
}

/// Drive the session to its first request and answer it with a plain TS body.
fn connect(session: &mut Session) -> ConnId {
    loop {
        match session.poll(ts(0)) {
            Some(Action::HttpRequest { conn, .. }) => {
                session.on_response(
                    conn,
                    support::status(200),
                    &support::ts_content_type(),
                    ts(0),
                );
                return conn;
            }
            Some(_) => continue,
            None => panic!("the TS-over-HTTP session issues a request"),
        }
    }
}

fn pump(session: &mut Session, now: Timestamp) -> Vec<Action> {
    let mut actions = Vec::new();
    while let Some(action) = session.poll(now) {
        actions.push(action);
    }
    actions
}

fn pause_read(actions: &[Action], conn: ConnId) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::PauseRead { conn: c } if *c == conn))
}

fn resume_read(actions: &[Action], conn: ConnId) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::ResumeRead { conn: c } if *c == conn))
}

/// Tracks the media end the session measured, from the samples it hands out.
#[derive(Default)]
struct Media {
    timescales: BTreeMap<u32, u32>,
    latest_end: Option<i128>,
}

fn ticks(pts: i64, timescale: u32) -> i128 {
    i128::from(pts) * i128::from(PCR_HZ) / i128::from(timescale)
}

impl Media {
    fn observe(&mut self, event: &Event) {
        if let Event::Tracks(specs) = event {
            for spec in specs {
                self.timescales
                    .insert(spec.track_id.value(), spec.timescale.get());
            }
        }
    }

    fn sample_end(&self, track: u32, sample: &broadcast_client::Sample) -> Option<i128> {
        let timescale = self.timescales.get(&track).copied()?;
        let pts = sample.timing.and_then(|t| t.pts())?;
        let duration = i64::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0));
        Some(ticks(pts.saturating_add(duration), timescale))
    }

    fn drain_each(&mut self, session: &mut Session, mut on_sample: impl FnMut(i128)) {
        while let Some(event) = session.next_event() {
            match &event {
                Event::Sample { track, sample, .. } => {
                    if let Some(end) = self.sample_end(track.value(), sample) {
                        self.latest_end =
                            Some(self.latest_end.map_or(end, |latest| latest.max(end)));
                        on_sample(end);
                    }
                }
                other => self.observe(other),
            }
        }
    }

    fn drain(&mut self, session: &mut Session) {
        self.drain_each(session, |_| {});
    }
}

/// Presentation width of one read-ahead byte entry.
const SLICE_TICKS: i128 = (PCR_HZ / 10) as i128;

/// Mirrors the session's byte accounting so a test can assert what is ahead.
#[derive(Default)]
struct ByteLedger {
    pending: u64,
    entries: Vec<(i128, u64)>,
    fed: u64,
}

impl ByteLedger {
    fn accept(&mut self, len: usize) {
        self.fed = self.fed.saturating_add(len as u64);
        self.pending = self.pending.saturating_add(len as u64);
    }

    fn sample(&mut self, end: i128) {
        let take = self.pending;
        self.pending = 0;
        if take == 0 {
            return;
        }
        // The session coalesces samples into 100 ms presentation slices.
        if let Some((last_end, last_bytes)) = self.entries.last_mut()
            && end / SLICE_TICKS <= *last_end / SLICE_TICKS
        {
            *last_bytes = last_bytes.saturating_add(take);
            if end > *last_end {
                *last_end = end;
            }
            return;
        }
        self.entries.push((end, take));
    }

    fn ahead(&self, position: i128) -> u64 {
        let queued: u64 = self
            .entries
            .iter()
            .filter(|(end, _)| *end > position)
            .map(|(_, bytes)| *bytes)
            .sum();
        self.pending.saturating_add(queued)
    }
}

fn clock(ticks: u64) -> Clock27 {
    Clock27::from_27mhz_wrapping(ticks)
}

#[test]
fn a_fast_ts_source_is_paused_at_the_time_cap() {
    let cap = seconds(30);
    let config = limited(ReadAhead::time(cap));
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    let cap_ticks = i128::from(cap.as_secs()) * i128::from(PCR_HZ);
    let mut media = Media::default();
    let mut previous_lead = 0i128;
    let mut pauses = 0;
    for chunk in bytes.chunks(CHUNK) {
        let before = media.latest_end;
        session.on_data(conn, chunk, ts(0));
        media.drain(&mut session);
        let lead = media.latest_end.unwrap_or(0);
        let span = lead - before.unwrap_or(lead);
        let actions = pump(&mut session, ts(0));
        if pause_read(&actions, conn) {
            pauses += 1;
            assert!(
                previous_lead < cap_ticks,
                "paused at {previous_lead} ticks, before the {cap_ticks}-tick cap"
            );
            assert!(
                lead >= cap_ticks,
                "paused at {lead} ticks, before the {cap_ticks}-tick cap"
            );
            assert!(
                lead - cap_ticks <= span,
                "overshot the cap by {} ticks, more than the {span}-tick chunk",
                lead - cap_ticks
            );
            break;
        }
        previous_lead = lead;
    }
    assert_eq!(pauses, 1, "the cap pauses the connection exactly once");
    assert!(session.read_paused(conn));
}

#[test]
fn the_byte_cap_pauses_when_time_cannot_be_measured() {
    let cap = 300_000usize;
    let config = limited(ReadAhead::bytes(cap as u64));
    let mut session = open(config);
    let conn = connect(&mut session);

    let bytes = fixture();
    let mut fed = 0usize;
    let mut pauses = 0;
    for chunk in bytes.chunks(CHUNK) {
        let before = fed;
        session.on_data(conn, chunk, ts(0));
        fed += chunk.len();
        let actions = pump(&mut session, ts(0));
        if pause_read(&actions, conn) {
            pauses += 1;
            assert!(
                before < cap,
                "paused after {before} bytes, before the {cap}-byte cap"
            );
            assert!(
                fed >= cap,
                "paused after {fed} bytes, before the {cap}-byte cap"
            );
            assert!(
                fed - cap < CHUNK,
                "overshot the byte cap by {} bytes, more than one {CHUNK}-byte chunk",
                fed - cap
            );
            break;
        }
    }
    assert_eq!(pauses, 1, "the byte cap pauses the connection exactly once");
    assert!(session.read_paused(conn));
}

#[test]
fn reading_resumes_as_playback_advances() {
    let cap = seconds(5);
    let config = limited(ReadAhead::time(cap));
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    let mut media = Media::default();
    let mut index = 0usize;
    while index < chunks.len() {
        session.on_data(conn, chunks[index], ts(0));
        index += 1;
        media.drain(&mut session);
        if pause_read(&pump(&mut session, ts(0)), conn) {
            break;
        }
    }
    assert!(session.read_paused(conn));
    let latest = media.latest_end.unwrap();
    let latest_ticks = u64::try_from(latest).unwrap();

    let well_ahead = latest_ticks - 4 * PCR_HZ;
    session.on_playback_position(ts(0), Some(clock(well_ahead)));
    assert!(
        !resume_read(&pump(&mut session, ts(0)), conn),
        "4 s of lead is still over the 2.5 s resume threshold"
    );
    assert!(session.read_paused(conn));

    let under_threshold = latest_ticks - 2 * PCR_HZ;
    session.on_playback_position(ts(0), Some(clock(under_threshold)));
    let resumed = pump(&mut session, ts(0));
    assert_eq!(
        resumed
            .iter()
            .filter(|a| matches!(a, Action::ResumeRead { conn: c } if *c == conn))
            .count(),
        1,
        "the lead under half the cap resumes the connection exactly once"
    );
    assert!(!session.read_paused(conn));

    while index < chunks.len() {
        session.on_data(conn, chunks[index], ts(0));
        index += 1;
        media.drain(&mut session);
        let actions = pump(&mut session, ts(0));
        if pause_read(&actions, conn) {
            assert!(session.read_paused(conn));
            return;
        }
    }
    panic!("a second fast burst pauses the connection again");
}

#[test]
fn a_read_pause_is_not_a_stall() {
    let ts_http = broadcast_client::TsHttpConfig::default()
        .with_stall_notice(milliseconds(100))
        .with_stall_reconnect(milliseconds(200))
        .with_read_ahead(ReadAhead::time(seconds(2)));
    let mut config = Config::default();
    config.ts_http = ts_http;
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    let mut media = Media::default();
    for chunk in bytes.chunks(CHUNK) {
        session.on_data(conn, chunk, ts(0));
        media.drain(&mut session);
        if pause_read(&pump(&mut session, ts(0)), conn) {
            break;
        }
    }
    assert!(session.read_paused(conn));

    let mut now_us = 0u64;
    for _ in 0..20 {
        now_us += 100_000;
        session.on_playback_position(ts(now_us), Some(clock(0)));
        let actions = pump(&mut session, ts(now_us));
        for action in &actions {
            assert!(
                !matches!(action, Action::Close { .. } | Action::HttpRequest { .. }),
                "a deliberate read pause reconnected: {action:?}"
            );
        }
        while let Some(event) = session.next_event() {
            match event {
                Event::Status(Status::Stalled { .. }) => {
                    panic!("a read pause was reported as a stall")
                }
                Event::Starved(kind) => panic!("a read pause was reported as starvation: {kind}"),
                Event::Buffer(BufferEvent::Pause { reason, .. }) => {
                    assert!(
                        !matches!(reason, PauseReason::NoData | PauseReason::Reprime),
                        "a read pause caused a buffer pause: {reason:?}"
                    );
                }
                _ => {}
            }
        }
    }
    assert!(session.read_paused(conn));
}

#[test]
fn a_real_time_source_is_never_paused() {
    let mut session = open(limited(ReadAhead::time(seconds(30))));
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    let mut media = Media::default();
    let mut base = None;
    let mut now_us = 0u64;
    for chunk in bytes.chunks(CHUNK) {
        session.on_data(conn, chunk, ts(now_us));
        media.drain(&mut session);
        let latest = media.latest_end.unwrap();
        let start = *base.get_or_insert(latest);
        let elapsed = u128::try_from(latest - start).unwrap();
        now_us = u64::try_from(elapsed * 1_000_000 / u128::from(PCR_HZ)).unwrap();
        let position = u64::try_from(latest - i128::from(PCR_HZ)).unwrap_or(0);
        session.on_playback_position(ts(now_us), Some(clock(position)));
        let actions = pump(&mut session, ts(now_us));
        assert!(
            !pause_read(&actions, conn),
            "a real-time source reached the read-ahead cap"
        );
    }
    assert!(!session.read_paused(conn));
}

#[test]
fn the_byte_cap_holds_while_the_position_advances() {
    let cap = 300_000usize;
    let config = limited(ReadAhead::bytes(cap as u64));
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    let mut media = Media::default();
    let mut ledger = ByteLedger::default();
    let mut position = 0u64;
    let mut pauses = 0;
    for chunk in bytes.chunks(CHUNK) {
        ledger.accept(chunk.len());
        session.on_data(conn, chunk, ts(0));
        media.drain_each(&mut session, |end| ledger.sample(end));
        let ahead = ledger.ahead(i128::from(position));
        position = position.saturating_add(PCR_HZ / 25);
        session.on_playback_position(ts(0), Some(clock(position)));
        let actions = pump(&mut session, ts(0));
        if pause_read(&actions, conn) {
            pauses += 1;
            assert!(
                ahead >= cap as u64,
                "paused with {ahead} bytes ahead, under the {cap}-byte cap"
            );
            break;
        }
    }
    assert_eq!(
        pauses, 1,
        "the byte cap pauses even though the position advances every chunk"
    );
    assert!(session.read_paused(conn));
}

#[test]
fn bytes_behind_the_position_stop_counting() {
    let cap = 300_000usize;
    let config = limited(ReadAhead::bytes(cap as u64));
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    let mut media = Media::default();
    let mut ledger = ByteLedger::default();
    for chunk in bytes.chunks(CHUNK) {
        ledger.accept(chunk.len());
        session.on_data(conn, chunk, ts(0));
        media.drain_each(&mut session, |end| ledger.sample(end));
        if pause_read(&pump(&mut session, ts(0)), conn) {
            break;
        }
    }
    assert!(session.read_paused(conn));

    let latest = u64::try_from(media.latest_end.unwrap()).unwrap();
    let half = cap as u64 / 2;
    let step = PCR_HZ / 10;
    let mut position = 0u64;
    let mut resumed = false;
    while position < latest {
        position = (position + step).min(latest);
        let ahead = ledger.ahead(i128::from(position));
        session.on_playback_position(ts(0), Some(clock(position)));
        let actions = pump(&mut session, ts(0));
        if resume_read(&actions, conn) {
            assert!(
                ahead < half,
                "resumed with {ahead} bytes ahead, at or over the {half}-byte threshold"
            );
            resumed = true;
            break;
        }
        assert!(
            ahead >= half,
            "no resume although only {ahead} bytes were ahead, under the {half}-byte threshold"
        );
    }
    assert!(
        resumed,
        "the position passing the queued media did not resume"
    );
    assert!(!session.read_paused(conn));

    let past_half = latest / 2 + step;
    if position < past_half {
        session.on_playback_position(ts(0), Some(clock(past_half)));
        let actions = pump(&mut session, ts(0));
        assert!(
            !resume_read(&actions, conn),
            "past half the media the resumed connection announced a second resume"
        );
        assert!(!session.read_paused(conn));
    }
}

#[test]
fn a_stream_without_timed_samples_is_still_capped() {
    let cap = 300_000usize;
    let config = limited(ReadAhead::bytes(cap as u64));
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));

    let mut packet = [0xFFu8; 188];
    packet[0] = 0x47;
    packet[1] = 0x1F;
    packet[2] = 0xFF;
    packet[3] = 0x10;
    let chunk = packet.repeat(CHUNK / 188);

    let mut position = 0u64;
    let mut fed = 0usize;
    let mut pauses = 0;
    for _ in 0..20 {
        fed += chunk.len();
        session.on_data(conn, &chunk, ts(0));
        position = position.saturating_add(PCR_HZ / 10);
        session.on_playback_position(ts(0), Some(clock(position)));
        let actions = pump(&mut session, ts(0));
        if pause_read(&actions, conn) {
            pauses += 1;
            assert!(
                fed >= cap,
                "paused after {fed} bytes, before the {cap}-byte cap"
            );
            break;
        }
    }
    assert_eq!(pauses, 1, "a stream with no samples is still capped");
    assert!(session.read_paused(conn));
}

fn sample_sequence(session: &mut Session) -> Vec<(u32, Option<i64>, usize)> {
    let mut sequence = Vec::new();
    while let Some(event) = session.next_event() {
        if let Event::Sample { track, sample, .. } = event {
            sequence.push((
                track.value(),
                sample.timing.and_then(|t| t.pts()),
                sample.data.len(),
            ));
        }
    }
    sequence
}

#[test]
fn bytes_fed_while_paused_are_kept() {
    let mut capped = open(limited(ReadAhead::time(seconds(30))));
    let conn = connect(&mut capped);
    capped.on_playback_position(ts(0), Some(clock(0)));
    let uncapped = limited(ReadAhead::Unlimited);
    let mut open_ended = open(uncapped);
    let other = connect(&mut open_ended);
    open_ended.on_playback_position(ts(0), Some(clock(0)));

    let bytes = fixture();
    for chunk in bytes.chunks(CHUNK) {
        capped.on_data(conn, chunk, ts(0));
        open_ended.on_data(other, chunk, ts(0));
    }
    let actions = pump(&mut capped, ts(0));
    assert!(
        pause_read(&actions, conn),
        "the capped session paused during the burst"
    );
    assert_eq!(
        sample_sequence(&mut capped),
        sample_sequence(&mut open_ended),
        "a capped session still delivers every sample the uncapped one does"
    );
}

#[test]
fn a_reconnect_clears_the_pause_without_a_resume_for_the_old_connection() {
    let config = limited(ReadAhead::time(seconds(2)));
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));
    let bytes = fixture();
    let mut media = Media::default();
    for chunk in bytes.chunks(CHUNK) {
        session.on_data(conn, chunk, ts(0));
        media.drain(&mut session);
        if pause_read(&pump(&mut session, ts(0)), conn) {
            break;
        }
    }
    assert!(session.read_paused(conn));

    session.reconnect(ts(0));
    let actions = pump(&mut session, ts(0));
    assert!(
        !resume_read(&actions, conn),
        "a resume for a connection that no longer exists is not emitted"
    );
    assert!(!session.read_paused(conn));
}

#[test]
fn a_limit_with_no_bound_is_a_config_error() {
    let config = limited(ReadAhead::Limit {
        time: None,
        bytes: None,
    });
    assert_eq!(
        config_error(config),
        Error::InvalidConfig(broadcast_client::ConfigField::ReadAhead)
    );
}

#[test]
fn a_zero_bound_is_a_config_error() {
    let time = limited(ReadAhead::time(Duration::ZERO));
    assert_eq!(
        config_error(time),
        Error::InvalidConfig(broadcast_client::ConfigField::ReadAhead)
    );
    let bytes = limited(ReadAhead::bytes(0));
    assert_eq!(
        config_error(bytes),
        Error::InvalidConfig(broadcast_client::ConfigField::ReadAhead)
    );
}

fn assert_never_pauses(config: Config) {
    let mut session = open(config);
    let conn = connect(&mut session);
    session.on_playback_position(ts(0), Some(clock(0)));
    let bytes = fixture();
    let mut paused = false;
    for chunk in bytes.chunks(CHUNK) {
        session.on_data(conn, chunk, ts(0));
        let actions = pump(&mut session, ts(0));
        paused |= pause_read(&actions, conn) || resume_read(&actions, conn);
    }
    assert!(!paused, "an unbounded session never asks the host to pause");
    assert!(!session.read_paused(conn));
}

#[test]
fn unlimited_never_pauses() {
    assert_never_pauses(limited(ReadAhead::Unlimited));
}

#[test]
fn the_default_is_unlimited() {
    assert_eq!(ReadAhead::default(), ReadAhead::Unlimited);
    assert_eq!(Config::default().ts_http.read_ahead, ReadAhead::Unlimited);
    assert_never_pauses(Config::default());
}

#[test]
fn large_limits_are_accepted_and_never_pause_a_small_file() {
    assert_never_pauses(limited(ReadAhead::time_and_bytes(
        Duration::from_secs(90),
        128 << 30,
    )));
}
