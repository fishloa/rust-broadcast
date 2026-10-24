mod support;

use std::path::{Path, PathBuf};

use broadcast_client::{
    Action, BufferEvent, BufferInput, Config, Event, Hint, Session, TimelineJumpConfig, TrackId,
};
use support::hls_host::{self as hls, Reply};
use support::rtsp_host::{
    self, BASE_URL, MediaFixture, Server, channel_of, interleaved, quiet_config,
};
use support::{read_tf1_bytes, read_tf1_headers};
use transmux::JoinDecision;
use transmux::rtp::RtpMediaKind;

const CHUNK: usize = 1316;
const PACKET: usize = 188;
const STEP_US: u64 = 3_200;
const TICK_US: u64 = 50_000;
const PAUSE_US: u64 = 3_000_000;
const TAIL_US: u64 = 6_000_000;
const URL: &str = "http://example/live.ts";
const SMALL: &str = "ts/h264_aac.ts";
const AUDIO_ONLY: &str = "ts/aac-5_1-640k.ts";
const AUDIO_ONLY_RATE: i64 = 48_000;
const SMALL_AUDIO_RATE: i64 = 44_100;
const PCR_HZ: u64 = 27_000_000;
const SEC_US: i64 = 1_000_000;
const SYNC_BYTE: u8 = 0x47;
const AF_DISCONTINUITY_FLAG: u8 = 0x80;
const AF_PCR_FLAG: u8 = 0x10;
const AF_PRESENT: u8 = 0x20;
const PTS_WRAP: i64 = 1 << 33;
const VIDEO_STREAM_IDS: std::ops::RangeInclusive<u8> = 0xE0..=0xEF;
const AUDIO_STREAM_IDS: std::ops::RangeInclusive<u8> = 0xC0..=0xDF;
const FIRST_KEPT_MARGIN: usize = 30;
const BACKWARD_MARGIN_US: i64 = 100_000;
const FORWARD_SLACK_US: i64 = 5_000_000;
const FAR_JUMP_US: i64 = 30 * SEC_US;
const LONG_PAUSE_US: u64 = 10_000_000;

fn pts_to_us(pts: i64, timescale: u32) -> i64 {
    i64::try_from(i128::from(pts) * 1_000_000 / i128::from(timescale)).unwrap()
}

fn us_to_ticks(us: i64, timescale: u32) -> i64 {
    i64::try_from(i128::from(us) * i128::from(timescale) / 1_000_000).unwrap()
}

fn pts27(pts: i64, timescale: u32) -> u64 {
    u64::try_from(i128::from(pts) * i128::from(PCR_HZ) / i128::from(timescale)).unwrap()
}

#[derive(Clone, Copy)]
enum ClockState {
    Idle,
    Playing { base: u64, since: u64 },
    Held(u64),
}

struct PlayClock(ClockState);

impl PlayClock {
    fn on_buffer(&mut self, now: u64, ev: &BufferEvent) {
        self.0 = match ev {
            BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz } => {
                ClockState::Playing {
                    base: *at_27mhz,
                    since: now,
                }
            }
            BufferEvent::Pause { hold_at_27mhz, .. } => ClockState::Held(*hold_at_27mhz),
            _ => self.0,
        };
    }

    fn position(&self, now: u64) -> Option<u64> {
        match self.0 {
            ClockState::Idle => None,
            ClockState::Held(p) => Some(p),
            ClockState::Playing { base, since } => {
                Some(base + now.saturating_sub(since) * (PCR_HZ / 1_000_000))
            }
        }
    }
}

struct Host {
    session: Session,
    conn: u32,
    clock: PlayClock,
    events: Vec<Event>,
    times: Vec<u64>,
    next_tick: u64,
}

impl Host {
    fn open(config: Config) -> Self {
        Self::open_with(config, read_tf1_headers())
    }

    fn open_plain(config: Config) -> Self {
        Self::open_with(config, (200, Vec::new()))
    }

    fn open_with(config: Config, (status, headers): (u16, Vec<(String, String)>)) -> Self {
        let mut session = Session::open(URL, config, 0).unwrap();
        let conn = match session.poll(0).unwrap() {
            Action::HttpRequest { conn, .. } => conn,
            other => panic!("{other:?}"),
        };
        session.on_response(conn, status, &headers, 0);
        Self {
            session,
            conn,
            clock: PlayClock(ClockState::Idle),
            events: Vec::new(),
            times: Vec::new(),
            next_tick: 0,
        }
    }

    fn drain(&mut self, now: u64) {
        while let Some(e) = self.session.next_event() {
            if let Event::Buffer(b) = &e {
                self.clock.on_buffer(now, b);
            }
            self.events.push(e);
            self.times.push(now);
        }
    }

    fn data(&mut self, bytes: &[u8], now: u64) {
        self.session.on_data(self.conn, bytes, now);
        self.drain(now);
    }

    fn ticks_until(&mut self, now: u64) {
        while self.next_tick <= now {
            let t = self.next_tick;
            let p = self.clock.position(t);
            self.session.on_playback_position(t, p);
            self.drain(t);
            self.next_tick += TICK_US;
        }
    }

    fn feed(&mut self, bytes: &[u8], schedule: &[u64]) {
        for (chunk, &now) in bytes.chunks(CHUNK).zip(schedule) {
            self.ticks_until(now);
            self.data(chunk, now);
        }
    }

    fn log(&self) -> &[BufferInput] {
        self.session.buffer_input_log()
    }

    fn discontinuities(&self) -> Vec<usize> {
        self.events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Event::Discontinuity))
            .map(|(i, _)| i)
            .collect()
    }

    fn starts(&self) -> Vec<(usize, u64)> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                Event::Buffer(BufferEvent::Start { at_27mhz }) => Some((i, *at_27mhz)),
                _ => None,
            })
            .collect()
    }

    fn timescales(&self) -> Vec<(TrackId, u32)> {
        let mut out: Vec<(TrackId, u32)> = Vec::new();
        for e in &self.events {
            if let Event::Tracks(specs) = e {
                for s in specs {
                    out.retain(|(t, _)| *t != s.track_id);
                    out.push((s.track_id, s.timescale));
                }
            }
        }
        out
    }

    fn video(&self) -> TrackId {
        self.timescales()
            .iter()
            .find(|(_, ts)| *ts == VIDEO_TIMESCALE)
            .map(|(t, _)| *t)
            .expect("a 90 kHz video track")
    }

    fn samples_of(&self, track: TrackId) -> Vec<Rec> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                Event::Sample {
                    track: t,
                    sample,
                    join,
                } if *t == track => Some(Rec {
                    idx: i,
                    now: self.times[i],
                    pts: sample.pts.unwrap(),
                    join: *join,
                }),
                _ => None,
            })
            .collect()
    }
}

const VIDEO_TIMESCALE: u32 = 90_000;

#[derive(Clone, Copy, Debug)]
struct Rec {
    idx: usize,
    now: u64,
    pts: i64,
    join: Option<JoinDecision>,
}

impl Rec {
    fn kept(&self) -> bool {
        !matches!(self.join, Some(JoinDecision::Drop(_)))
    }
}

fn realtime(n: usize) -> Vec<u64> {
    (0..n).map(|i| i as u64 * STEP_US).collect()
}

fn chunk_count(bytes: &[u8]) -> usize {
    bytes.len().div_ceil(CHUNK)
}

fn with_pause_from(n: usize, split: usize, pause: u64) -> Vec<u64> {
    (0..n)
        .map(|i| i as u64 * STEP_US + if i >= split { pause } else { 0 })
        .collect()
}

fn packet_pid(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1f) << 8) | u16::from(p[2])
}

fn packet_has_pcr(p: &[u8]) -> bool {
    p[3] & AF_PRESENT != 0 && p[4] > 0 && p[5] & AF_PCR_FLAG != 0
}

fn pes_header_offset(p: &[u8]) -> Option<usize> {
    if p[0] != SYNC_BYTE || p[1] & 0x40 == 0 {
        return None;
    }
    let afc = (p[3] >> 4) & 3;
    if afc & 1 == 0 {
        return None;
    }
    let off = 4 + if afc & 2 != 0 {
        1 + usize::from(p[4])
    } else {
        0
    };
    (off + 14 <= PACKET && p[off..off + 3] == [0, 0, 1]).then_some(off)
}

fn read_ts(b: &[u8]) -> i64 {
    (i64::from(b[0] >> 1) & 7) << 30
        | i64::from(b[1]) << 22
        | (i64::from(b[2]) >> 1) << 15
        | i64::from(b[3]) << 7
        | i64::from(b[4]) >> 1
}

fn write_ts(b: &mut [u8], v: i64) {
    let prefix = b[0] & 0xF0;
    b[0] = prefix | ((((v >> 30) & 7) as u8) << 1) | 1;
    b[1] = (v >> 22) as u8;
    b[2] = ((((v >> 15) & 0x7f) as u8) << 1) | 1;
    b[3] = (v >> 7) as u8;
    b[4] = (((v & 0x7f) as u8) << 1) | 1;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shift {
    Video,
    Audio,
    Both,
}

fn stream_kind(p: &[u8], off: usize) -> Option<Shift> {
    let id = p[off + 3];
    if VIDEO_STREAM_IDS.contains(&id) {
        Some(Shift::Video)
    } else if AUDIO_STREAM_IDS.contains(&id) {
        Some(Shift::Audio)
    } else {
        None
    }
}

fn video_pes_starts(bytes: &[u8]) -> Vec<usize> {
    bytes
        .chunks(PACKET)
        .enumerate()
        .filter(|(_, p)| {
            pes_header_offset(p).is_some_and(|o| stream_kind(p, o) == Some(Shift::Video))
        })
        .map(|(i, _)| i)
        .collect()
}

fn shift_pes(bytes: &[u8], from_packet: usize, delta_ticks: i64, which: Shift) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for (i, p) in out.chunks_mut(PACKET).enumerate() {
        if i < from_packet {
            continue;
        }
        let Some(off) = pes_header_offset(p) else {
            continue;
        };
        let Some(kind) = stream_kind(p, off) else {
            continue;
        };
        if which != Shift::Both && which != kind {
            continue;
        }
        let flags = p[off + 7];
        let hdr = off + 9;
        if flags & 0x80 != 0 {
            let v = (read_ts(&p[hdr..hdr + 5]) + delta_ticks).rem_euclid(PTS_WRAP);
            write_ts(&mut p[hdr..hdr + 5], v);
        }
        if flags & 0x40 != 0 {
            let v = (read_ts(&p[hdr + 5..hdr + 10]) + delta_ticks).rem_euclid(PTS_WRAP);
            write_ts(&mut p[hdr + 5..hdr + 10], v);
        }
    }
    out
}

fn feed_all(bytes: &[u8], config: Config) -> Host {
    let mut h = Host::open_plain(config);
    h.feed(bytes, &realtime(chunk_count(bytes)));
    h
}

fn run_burst(bytes: &[u8], config: Config) -> Host {
    let mut h = Host::open_plain(config);
    for c in bytes.chunks(CHUNK) {
        h.data(c, 0);
    }
    h
}

struct Boundary {
    sample: usize,
    pes_packet: usize,
    natural_step_ticks: i64,
    elapsed_us: i64,
    split_chunk: usize,
}

fn boundary(bytes: &[u8], same_chunk: bool) -> Boundary {
    let base = feed_all(bytes, Config::default());
    let vid = base.video();
    let vs = base.samples_of(vid);
    let starts = video_pes_starts(bytes);
    assert!(
        starts.len() >= vs.len() && starts.len() - vs.len() <= 2,
        "one PES per video sample, the demuxer holds back the last ones"
    );
    let chunk_of = |r: &Rec| usize::try_from(r.now / STEP_US).unwrap();
    let k = (FIRST_KEPT_MARGIN..vs.len())
        .find(|&k| {
            vs[k].kept()
                && vs[k - 1].kept()
                && ((chunk_of(&vs[k]) == chunk_of(&vs[k - 1])) == same_chunk)
        })
        .expect("a suitable boundary sample");
    Boundary {
        sample: k,
        pes_packet: starts[k],
        natural_step_ticks: vs[k].pts - vs[k - 1].pts,
        elapsed_us: i64::try_from(vs[k].now - vs[k - 1].now).unwrap(),
        split_chunk: chunk_of(&vs[k]),
    }
}

struct Jumped {
    host: Host,
    boundary: Boundary,
}

fn jumped(
    bytes: &[u8],
    same_chunk: bool,
    pause: u64,
    step_us_of: impl Fn(&Boundary) -> i64,
    config: Config,
) -> Jumped {
    let mut b = boundary(bytes, same_chunk);
    if !same_chunk {
        b.elapsed_us += i64::try_from(pause).unwrap();
    }
    let original = {
        let h = feed_all(bytes, Config::default());
        h.samples_of(h.video())
    };
    let delta = us_to_ticks(step_us_of(&b), VIDEO_TIMESCALE) - b.natural_step_ticks;
    let shifted = shift_pes(bytes, b.pes_packet, delta, Shift::Video);
    let n = chunk_count(bytes);
    let schedule = with_pause_from(n, b.split_chunk, pause);
    let mut host = Host::open_plain(config);
    host.feed(&shifted, &schedule);
    let vid = host.video();
    let got = host.samples_of(vid);
    assert_eq!(got.len(), original.len());
    for (j, (g, o)) in got.iter().zip(&original).enumerate() {
        let want = if j < b.sample {
            o.pts
        } else {
            (o.pts + delta).rem_euclid(PTS_WRAP)
        };
        assert_eq!(g.pts, want, "sample {j} retimed as planned");
    }
    Jumped { host, boundary: b }
}

fn burst_step(extra_us: i64) -> impl Fn(&Boundary) -> i64 {
    move |b| b.elapsed_us + extra_us
}

fn jump_discontinuities(j: &Jumped) -> Vec<usize> {
    j.host.discontinuities()
}

#[test]
fn forward_jump_beyond_elapsed_plus_5s_is_new_timeline() {
    let bytes = test_fixtures::read(SMALL);
    let same = jumped(
        &bytes,
        true,
        0,
        burst_step(FORWARD_SLACK_US + BACKWARD_MARGIN_US),
        Config::default(),
    );
    assert_eq!(same.boundary.elapsed_us, 0, "same delivery instant");
    assert_eq!(jump_discontinuities(&same).len(), 1);
    let paused = jumped(
        &bytes,
        false,
        LONG_PAUSE_US,
        burst_step(FORWARD_SLACK_US + BACKWARD_MARGIN_US),
        Config::default(),
    );
    assert!(paused.boundary.elapsed_us >= LONG_PAUSE_US as i64);
    assert_eq!(jump_discontinuities(&paused).len(), 1);
}

#[test]
fn forward_jump_within_elapsed_plus_5s_is_not() {
    let bytes = test_fixtures::read(SMALL);
    let burst = jumped(
        &bytes,
        true,
        0,
        burst_step(FORWARD_SLACK_US - BACKWARD_MARGIN_US),
        Config::default(),
    );
    assert_eq!(burst.boundary.elapsed_us, 0);
    assert!(jump_discontinuities(&burst).is_empty());
    let paused = jumped(
        &bytes,
        false,
        LONG_PAUSE_US,
        burst_step(FORWARD_SLACK_US - BACKWARD_MARGIN_US),
        Config::default(),
    );
    assert!(paused.boundary.elapsed_us >= LONG_PAUSE_US as i64);
    assert!(
        paused.boundary.elapsed_us + FORWARD_SLACK_US - BACKWARD_MARGIN_US > FORWARD_SLACK_US,
        "the step exceeds the bare slack, only the elapsed time excuses it"
    );
    assert!(jump_discontinuities(&paused).is_empty());
}

#[test]
fn backward_step_minus_0_9s_is_not_a_jump() {
    let bytes = test_fixtures::read(SMALL);
    let j = jumped(
        &bytes,
        true,
        0,
        |_| -(SEC_US - BACKWARD_MARGIN_US),
        Config::default(),
    );
    assert!(jump_discontinuities(&j).is_empty());
}

#[test]
fn backward_step_minus_1_1s_is_a_jump() {
    let bytes = test_fixtures::read(SMALL);
    let j = jumped(
        &bytes,
        true,
        0,
        |_| -(SEC_US + BACKWARD_MARGIN_US),
        Config::default(),
    );
    assert_eq!(jump_discontinuities(&j).len(), 1);
}

#[test]
fn detection_happens_before_the_sample_is_emitted() {
    let bytes = test_fixtures::read(SMALL);
    let j = jumped(&bytes, true, 0, |_| FAR_JUMP_US, Config::default());
    let h = &j.host;
    let d = h.discontinuities();
    assert_eq!(d.len(), 1);
    let at = d[0];
    let vid = h.video();
    let first_new = h.samples_of(vid)[j.boundary.sample];
    let first_new_pos = first_new.idx;
    assert!(
        at < first_new_pos,
        "Discontinuity at {at}, first sample of the new timeline at {first_new_pos}"
    );
    let between: Vec<&Event> = h.events[at + 1..first_new_pos].iter().collect();
    assert!(
        between
            .iter()
            .all(|e| !matches!(e, Event::Sample { track, .. } if *track == vid)),
        "no video sample between the Discontinuity and the new timeline: {between:?}"
    );
    match &h.events[at + 1] {
        Event::Sample {
            track,
            sample,
            join,
        } => {
            assert_eq!(*track, vid);
            assert_eq!(sample.pts, Some(first_new.pts));
            assert!(
                matches!(join, Some(JoinDecision::Drop(_))),
                "judged by the fresh join state: {join:?}"
            );
        }
        other => panic!("sample expected right after the Discontinuity: {other:?}"),
    }
}

#[test]
fn controller_inputs_after_jump_include_pcr_discontinuity_hint_and_fresh_join_origin() {
    let bytes = test_fixtures::read(SMALL);
    let clean = feed_all(&bytes, Config::default());
    assert!(
        !clean.log().iter().any(|l| matches!(
            l,
            BufferInput::Hint {
                hint: Hint::PcrDiscontinuity,
                ..
            }
        )),
        "the unmodified stream signals no discontinuity"
    );
    let j = jumped(&bytes, true, 0, |_| FAR_JUMP_US, Config::default());
    let h = &j.host;
    let log = h.log();
    let hints: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            matches!(
                l,
                BufferInput::Hint {
                    hint: Hint::PcrDiscontinuity,
                    ..
                }
            )
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(hints.len(), 1, "{log:?}");
    let vid = h.video();
    let vs = h.samples_of(vid);
    let join_after: Vec<&Rec> = vs
        .iter()
        .filter(|r| {
            r.idx > h.discontinuities()[0] && matches!(r.join, Some(JoinDecision::JoinPoint(_)))
        })
        .collect();
    assert_eq!(join_after.len(), 1, "the new timeline joins once: {vs:?}");
    let origin = pts27(join_after[0].pts, VIDEO_TIMESCALE);
    let first_report_after = log[hints[0]..]
        .iter()
        .find_map(|l| match l {
            BufferInput::TrackMedia { track, end, .. } if *track == vid.value() => Some(*end),
            _ => None,
        })
        .expect("a video media report on the new timeline");
    assert_eq!(
        first_report_after, origin,
        "join origin of the new timeline"
    );
    let before = vs[j.boundary.sample - 1].pts;
    assert!(
        pts27(before, VIDEO_TIMESCALE) < origin,
        "the new timeline lies ahead of the old one"
    );
    let reports_after: Vec<u64> = log[hints[0]..]
        .iter()
        .filter_map(|l| match l {
            BufferInput::TrackMedia { track, end, .. } if *track == vid.value() => Some(*end),
            _ => None,
        })
        .collect();
    assert!(
        reports_after.iter().all(|e| *e >= origin),
        "no end of the old timeline after the hint: {reports_after:?}"
    );
}

#[test]
fn config_none_disables_detector() {
    let bytes = test_fixtures::read(SMALL);
    let on = jumped(&bytes, true, 0, |_| FAR_JUMP_US, Config::default());
    assert_eq!(on.host.discontinuities().len(), 1, "positive control");
    let off = jumped(
        &bytes,
        true,
        0,
        |_| FAR_JUMP_US,
        Config {
            timeline_jump: None,
            ..Config::default()
        },
    );
    assert!(off.host.discontinuities().is_empty());
    assert!(!off.host.log().iter().any(|l| matches!(
        l,
        BufferInput::Hint {
            hint: Hint::PcrDiscontinuity,
            ..
        }
    )));
    assert_eq!(
        Config::default().timeline_jump,
        Some(TimelineJumpConfig::default())
    );
    assert_eq!(TimelineJumpConfig::default().backward_us, 1_000_000);
    assert_eq!(TimelineJumpConfig::default().forward_slack_us, 5_000_000);
}

#[test]
fn config_custom_thresholds_applied() {
    let bytes = test_fixtures::read(SMALL);
    let tight = Config {
        timeline_jump: Some(TimelineJumpConfig {
            backward_us: 500_000,
            forward_slack_us: 1_000_000,
        }),
        ..Config::default()
    };
    let loose = Config {
        timeline_jump: Some(TimelineJumpConfig {
            backward_us: 2_000_000,
            forward_slack_us: 20_000_000,
        }),
        ..Config::default()
    };
    let back = |_: &Boundary| -(SEC_US - BACKWARD_MARGIN_US);
    assert_eq!(
        jump_discontinuities(&jumped(&bytes, true, 0, back, tight.clone())).len(),
        1,
        "-0.9 s exceeds a 0.5 s threshold"
    );
    assert!(
        jump_discontinuities(&jumped(
            &bytes,
            true,
            0,
            |_| -(SEC_US + BACKWARD_MARGIN_US),
            loose.clone()
        ))
        .is_empty(),
        "-1.1 s is inside a 2 s threshold"
    );
    let fwd = |_: &Boundary| 3 * SEC_US;
    assert_eq!(
        jump_discontinuities(&jumped(&bytes, true, 0, fwd, tight)).len(),
        1,
        "+3 s exceeds a 1 s slack"
    );
    assert!(
        jump_discontinuities(&jumped(&bytes, true, 0, |_| 15 * SEC_US, loose)).is_empty(),
        "+15 s is inside a 20 s slack"
    );
}

fn tf1_prefix_chunks() -> Vec<u8> {
    read_tf1_bytes()
}

#[test]
fn same_capture_fed_twice_after_3s_pause_emits_discontinuity_before_first_new_sample() {
    let bytes = tf1_prefix_chunks();
    let n = chunk_count(&bytes);
    let mut h = Host::open(Config::default());
    h.feed(&bytes, &realtime(n));
    let end = (n as u64) * STEP_US;
    let first_pass_events = h.events.len();
    let vid = h.video();
    let first_pts = h.samples_of(vid)[0].pts;
    assert!(h.discontinuities().is_empty());
    let at = end + PAUSE_US;
    h.ticks_until(at);
    for c in bytes.chunks(CHUNK) {
        h.data(c, at);
    }
    let d = h.discontinuities();
    assert_eq!(d.len(), 1, "one new timeline: {d:?}");
    assert!(d[0] >= first_pass_events);
    match &h.events[d[0] + 1] {
        Event::Sample {
            track,
            sample,
            join,
        } => {
            assert_eq!(*track, vid);
            assert_eq!(
                sample.pts,
                Some(first_pts),
                "first sample of the second pass"
            );
            assert!(
                matches!(join, Some(JoinDecision::JoinPoint(_))),
                "the capture starts on an IDR: {join:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn second_pass_buffer_start_follows_from_new_timeline_media() {
    let bytes = tf1_prefix_chunks();
    let n = chunk_count(&bytes);
    let mut h = Host::open(Config::default());
    h.feed(&bytes, &realtime(n));
    let end = (n as u64) * STEP_US;
    let vid = h.video();
    let first_starts = h.starts();
    assert_eq!(first_starts.len(), 1, "first pass starts once");
    assert_eq!(
        first_starts[0].1,
        pts27(h.samples_of(vid)[0].pts, VIDEO_TIMESCALE)
    );
    let at = end + PAUSE_US;
    h.ticks_until(at);
    for c in bytes.chunks(CHUNK) {
        h.data(c, at);
    }
    h.ticks_until(at + TAIL_US);
    let d = h.discontinuities();
    assert_eq!(d.len(), 1);
    let origin = h
        .samples_of(vid)
        .iter()
        .find(|r| r.idx > d[0] && matches!(r.join, Some(JoinDecision::JoinPoint(_))))
        .map(|r| pts27(r.pts, VIDEO_TIMESCALE))
        .expect("join point on the second timeline");
    let go: Vec<(usize, u64)> = h
        .events
        .iter()
        .enumerate()
        .skip(d[0])
        .filter_map(|(i, e)| match e {
            Event::Buffer(BufferEvent::Start { at_27mhz })
            | Event::Buffer(BufferEvent::Resume { at_27mhz }) => Some((i, *at_27mhz)),
            _ => None,
        })
        .collect();
    assert_eq!(go.len(), 1, "one go after the new timeline: {go:?}");
    assert!(
        go[0].1 >= origin && go[0].1 - origin <= TAIL_US * (PCR_HZ / 1_000_000),
        "the go position {} lies on the new timeline starting at {origin}",
        go[0].1
    );
    assert!(
        h.events[..d[0]]
            .iter()
            .any(|e| matches!(e, Event::Buffer(BufferEvent::Pause { .. }))),
        "the controller starved during the pause, so the go is a Resume (the controller never emits a second StartPlaying)"
    );
}

#[test]
fn bframe_reordering_never_triggers() {
    let bytes = tf1_prefix_chunks();
    let n = chunk_count(&bytes);
    let mut h = Host::open(Config::default());
    h.feed(&bytes, &realtime(n));
    let vid = h.video();
    let vs: Vec<Rec> = h.samples_of(vid).into_iter().filter(Rec::kept).collect();
    assert!(vs.len() > 100);
    let min_step = vs.windows(2).map(|w| w[1].pts - w[0].pts).min().unwrap();
    assert!(min_step < 0, "the capture really reorders: {min_step}");
    assert!(
        pts_to_us(min_step, VIDEO_TIMESCALE) > -SEC_US,
        "and stays inside the threshold"
    );
    assert!(h.discontinuities().is_empty());
    let burst = run_burst_tf1(&bytes);
    assert!(burst.discontinuities().is_empty());
}

fn run_burst_tf1(bytes: &[u8]) -> Host {
    let mut h = Host::open(Config::default());
    for c in bytes.chunks(CHUNK) {
        h.data(c, 0);
    }
    h
}

#[test]
fn demux_discontinuity_and_jump_on_same_sample_emit_one_discontinuity() {
    let bytes = tf1_prefix_chunks();
    let n = chunk_count(&bytes);
    let starts = video_pes_starts(&bytes);
    let from = starts[starts.len() / 2];
    let video_pid = packet_pid(&bytes[starts[0] * PACKET..]);
    let flag_at = bytes
        .chunks(PACKET)
        .enumerate()
        .take(from)
        .filter(|(_, p)| packet_has_pcr(p) && packet_pid(p) == video_pid)
        .map(|(i, _)| i)
        .next_back()
        .expect("a PCR packet on the video PID before the jump");
    let pcr_hints = |h: &Host| {
        h.log()
            .iter()
            .filter(|l| {
                matches!(
                    l,
                    BufferInput::Hint {
                        hint: Hint::PcrDiscontinuity,
                        ..
                    }
                )
            })
            .count()
    };
    let mut flag_only = bytes.clone();
    flag_only[flag_at * PACKET + 5] |= AF_DISCONTINUITY_FLAG;
    let mut h = Host::open(Config::default());
    h.feed(&flag_only, &realtime(n));
    assert_eq!(pcr_hints(&h), 1, "the demuxer alone hints once");
    assert!(h.discontinuities().is_empty(), "and tells the host nothing");

    let mut jump_only = shift_pes(&bytes, from, FAR_JUMP_US * 9 / 100, Shift::Video);
    let mut h = Host::open(Config::default());
    h.feed(&jump_only, &realtime(n));
    assert_eq!(pcr_hints(&h), 1, "the detector alone hints once");
    assert_eq!(h.discontinuities().len(), 1);

    jump_only[flag_at * PACKET + 5] |= AF_DISCONTINUITY_FLAG;
    let mut h = Host::open(Config::default());
    h.feed(&jump_only, &realtime(n));
    assert_eq!(h.discontinuities().len(), 1, "{:?}", h.discontinuities());
    assert_eq!(pcr_hints(&h), 1, "demuxer and jump on one sample hint once");
}

fn audio_tracks(h: &Host) -> Vec<TrackId> {
    h.timescales()
        .into_iter()
        .filter(|(_, ts)| *ts != VIDEO_TIMESCALE)
        .map(|(t, _)| t)
        .collect()
}

fn audio_pes_starts(bytes: &[u8]) -> Vec<usize> {
    bytes
        .chunks(PACKET)
        .enumerate()
        .filter(|(_, p)| {
            pes_header_offset(p).is_some_and(|o| stream_kind(p, o) == Some(Shift::Audio))
        })
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn audio_only_stream_jump_detected() {
    let bytes = test_fixtures::read(AUDIO_ONLY);
    let from = audio_pes_starts(&bytes)[20];
    let shifted = shift_pes(&bytes, from, FAR_JUMP_US * 9 / 100, Shift::Audio);
    let clean = feed_all(&bytes, Config::default());
    assert_eq!(audio_tracks(&clean).len(), 1);
    assert!(
        clean
            .timescales()
            .iter()
            .all(|(_, ts)| *ts != VIDEO_TIMESCALE),
        "no video track announced"
    );
    assert!(clean.discontinuities().is_empty());
    let h = feed_all(&shifted, Config::default());
    let a = audio_tracks(&h)[0];
    let samples = h.samples_of(a);
    let max = samples.iter().map(|r| r.pts).max().unwrap();
    assert!(
        max - samples[0].pts > 29 * AUDIO_ONLY_RATE,
        "the audio really jumped"
    );
    assert_eq!(h.discontinuities().len(), 1);
    let d = h.discontinuities()[0];
    match &h.events[d + 1] {
        Event::Sample { track, sample, .. } => {
            assert_eq!(*track, a);
            assert!(sample.pts.unwrap() - samples[0].pts > 29 * AUDIO_ONLY_RATE);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn audio_jump_ignored_when_video_present() {
    let bytes = test_fixtures::read(SMALL);
    let from = audio_pes_starts(&bytes)[6];
    let shifted = shift_pes(&bytes, from, FAR_JUMP_US * 9 / 100, Shift::Audio);
    let h = feed_all(&shifted, Config::default());
    let a = audio_tracks(&h)[0];
    let samples = h.samples_of(a);
    let max = samples.iter().map(|r| r.pts).max().unwrap();
    assert!(
        max - samples[0].pts > 29 * SMALL_AUDIO_RATE,
        "the audio really jumped"
    );
    assert!(h.discontinuities().is_empty());
    let off = feed_all(
        &shifted,
        Config {
            timeline_jump: None,
            ..Config::default()
        },
    );
    assert_eq!(
        h.log(),
        off.log(),
        "the controller sees what it sees without the detector"
    );
}

fn fixture_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            fixture_paths(&p, out);
        } else if p.extension().is_some_and(|e| e == "ts") {
            out.push(p);
        }
    }
}

fn unflagged_jumps(h: &Host) -> Vec<(TrackId, i64, i64)> {
    let mut found = Vec::new();
    let scales = h.timescales();
    let mut last: Vec<(TrackId, Rec)> = Vec::new();
    for (i, e) in h.events.iter().enumerate() {
        match e {
            Event::Discontinuity => last.clear(),
            Event::Sample {
                track,
                sample,
                join,
            } => {
                if matches!(join, Some(JoinDecision::Drop(_))) {
                    continue;
                }
                let rec = Rec {
                    idx: i,
                    now: h.times[i],
                    pts: sample.pts.unwrap(),
                    join: *join,
                };
                let is_video = join.is_some();
                if is_video {
                    if let Some((_, prev)) = last.iter().find(|(t, _)| t == track) {
                        let ts = scales.iter().find(|(t, _)| t == track).unwrap().1;
                        let step = pts_to_us(rec.pts - prev.pts, ts);
                        let elapsed = i64::try_from(rec.now - prev.now).unwrap();
                        if step < -SEC_US || step > elapsed + FORWARD_SLACK_US {
                            found.push((*track, step, elapsed));
                        }
                    }
                    last.retain(|(t, _)| t != track);
                    last.push((*track, rec));
                }
            }
            _ => {}
        }
    }
    found
}

#[test]
fn no_false_positive_on_normal_stream_all_fixtures() {
    let mut paths = Vec::new();
    fixture_paths(&test_fixtures::path("ts"), &mut paths);
    assert!(paths.len() > 20, "{} public ts fixtures", paths.len());
    let mut with_video = 0;
    let mut report = Vec::new();
    for p in &paths {
        let bytes = std::fs::read(p).unwrap();
        for burst in [true, false] {
            let on = if burst {
                run_burst(&bytes, Config::default())
            } else {
                feed_all(&bytes, Config::default())
            };
            let off = if burst {
                run_burst(
                    &bytes,
                    Config {
                        timeline_jump: None,
                        ..Config::default()
                    },
                )
            } else {
                feed_all(
                    &bytes,
                    Config {
                        timeline_jump: None,
                        ..Config::default()
                    },
                )
            };
            let flagged_off = off.discontinuities().len();
            let flagged_on = on.discontinuities().len();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                unflagged_jumps(&on).is_empty(),
                "{name} burst={burst}: a jump went unflagged: {:?}",
                unflagged_jumps(&on)
            );
            if flagged_on != flagged_off {
                report.push(format!(
                    "{name} burst={burst}: {flagged_off} -> {flagged_on}"
                ));
            }
            if on
                .events
                .iter()
                .any(|e| matches!(e, Event::Sample { join: Some(_), .. }))
            {
                with_video += 1;
            }
        }
    }
    assert!(
        with_video > 20,
        "fixtures with video exercised: {with_video}"
    );
    assert!(
        report.is_empty(),
        "fixtures where the detector fired on a normal stream:\n{}",
        report.join("\n")
    );
}

#[test]
fn detector_state_resets_on_reconnect_and_rejoin() {
    let bytes = test_fixtures::read(SMALL);
    let n = chunk_count(&bytes);
    let half = n / 2;
    let mut h = Host::open_plain(Config::default());
    h.feed(&bytes, &realtime(n)[..half]);
    let first = h.events.len();
    h.session.on_closed(h.conn, None, 1_000_000);
    let mut conn = None;
    while let Some(a) = h.session.poll(1_000_000) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    h.conn = conn.expect("reconnect request");
    h.session.on_response(h.conn, 200, &[], 1_000_000);
    h.drain(1_000_000);
    for (i, c) in bytes.chunks(CHUNK).enumerate() {
        h.data(c, 2_000_000 + i as u64 * STEP_US);
    }
    let d: Vec<usize> = h
        .discontinuities()
        .into_iter()
        .filter(|i| *i >= first)
        .collect();
    assert_eq!(d.len(), 1, "only the reconnect's own Discontinuity: {d:?}");
    let vid = h.video();
    let after: Vec<Rec> = h
        .samples_of(vid)
        .into_iter()
        .filter(|r| r.idx > d[0])
        .collect();
    assert!(after.len() > 10);

    let mut r = Host::open_plain(Config::default());
    let sched = realtime(n);
    r.feed(&bytes, &sched[..half]);
    r.session.rejoin();
    let starts = video_pes_starts(&bytes);
    let resume = starts
        .iter()
        .copied()
        .find(|s| *s >= half * CHUNK / PACKET)
        .unwrap();
    let shifted = shift_pes(&bytes, resume, FAR_JUMP_US * 9 / 100, Shift::Video);
    let rest = &shifted[(half * CHUNK)..];
    for (i, c) in rest.chunks(CHUNK).enumerate() {
        r.data(c, sched[half - 1] + (i as u64 + 1) * STEP_US);
    }
    assert!(
        r.discontinuities().is_empty(),
        "the player already announced the new timeline with rejoin()"
    );
}

#[test]
fn hint_pcr_discontinuity_from_player_still_works() {
    let bytes = test_fixtures::read(SMALL);
    let n = chunk_count(&bytes);
    let mut h = Host::open_plain(Config::default());
    h.feed(&bytes, &realtime(n)[..n / 2]);
    let before = h
        .log()
        .iter()
        .filter(|l| {
            matches!(
                l,
                BufferInput::Hint {
                    hint: Hint::PcrDiscontinuity,
                    ..
                }
            )
        })
        .count();
    h.session.hint(Hint::PcrDiscontinuity);
    h.drain(0);
    let after = h
        .log()
        .iter()
        .filter(|l| {
            matches!(
                l,
                BufferInput::Hint {
                    hint: Hint::PcrDiscontinuity,
                    ..
                }
            )
        })
        .count();
    assert_eq!(before, 0);
    assert_eq!(after, 1);
    assert!(
        h.discontinuities().is_empty(),
        "a player hint is not a host event"
    );
}

#[test]
fn rejoin_from_player_still_works() {
    let bytes = test_fixtures::read(SMALL);
    let n = chunk_count(&bytes);
    let mut h = Host::open_plain(Config::default());
    let sched = realtime(n);
    h.feed(&bytes, &sched[..n / 2]);
    let vid = h.video();
    let joins_before = h
        .samples_of(vid)
        .iter()
        .filter(|r| matches!(r.join, Some(JoinDecision::JoinPoint(_))))
        .count();
    assert_eq!(joins_before, 1);
    let log_len = h.log().len();
    h.session.rejoin();
    assert_eq!(
        h.log().len(),
        log_len,
        "rejoin does not touch the controller"
    );
    for (i, c) in bytes.chunks(CHUNK).enumerate().skip(n / 2) {
        h.data(c, sched[i]);
    }
    let vs = h.samples_of(vid);
    let joins_after = vs
        .iter()
        .filter(|r| matches!(r.join, Some(JoinDecision::JoinPoint(_))))
        .count();
    assert_eq!(
        joins_after, 2,
        "the join re-armed and joined at the next IDR"
    );
    assert!(!h.log().iter().any(|l| matches!(
        l,
        BufferInput::Hint {
            hint: Hint::PcrDiscontinuity,
            ..
        }
    )));
    assert!(h.discontinuities().is_empty());
}

const RTP_TS_RANGE: std::ops::Range<usize> = 4..8;
const RTSP_FRAME_US: u64 = 10_000;
const RTSP_VIDEO_AU: usize = 30;
const RTSP_VIDEO_TIMESCALE: u32 = 90_000;

fn rtp_ts(p: &[u8]) -> u32 {
    u32::from_be_bytes(p[RTP_TS_RANGE].try_into().unwrap())
}

fn au_of_packets(packets: &[Vec<u8>]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut au = 0usize;
    for (i, p) in packets.iter().enumerate() {
        if i > 0 && rtp_ts(p) != rtp_ts(&packets[i - 1]) {
            au += 1;
        }
        out.push(au);
    }
    out
}

#[derive(Clone, Copy)]
struct RtpMod {
    stream: usize,
    from_au: usize,
    delta_ticks: i64,
}

struct RtspPlan {
    frames: Vec<Vec<u8>>,
    streams: Vec<(usize, usize)>,
}

fn rtsp_plan(fix: &MediaFixture, mods: &[RtpMod]) -> RtspPlan {
    let longest = fix.tracks.iter().map(|t| t.packets.len()).max().unwrap();
    let aus: Vec<Vec<usize>> = fix
        .tracks
        .iter()
        .map(|t| au_of_packets(&t.packets))
        .collect();
    let mut frames = Vec::new();
    let mut streams = Vec::new();
    for i in 0..longest {
        for (n, track) in fix.tracks.iter().enumerate() {
            let Some(packet) = track.packets.get(i) else {
                continue;
            };
            let au = aus[n].get(i).copied().unwrap_or(0);
            let mut packet = packet.clone();
            for m in mods {
                if m.stream == n && au >= m.from_au {
                    let ts = i64::from(rtp_ts(&packet)) + m.delta_ticks;
                    let ts = u32::try_from(ts.rem_euclid(1 << 32)).unwrap();
                    packet[RTP_TS_RANGE].copy_from_slice(&ts.to_be_bytes());
                }
            }
            frames.push(interleaved(channel_of(n), &packet));
            streams.push((n, i));
        }
    }
    RtspPlan { frames, streams }
}

struct Trace {
    events: Vec<Event>,
    times: Vec<u64>,
}

impl Trace {
    fn discontinuities(&self) -> Vec<usize> {
        self.events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Event::Discontinuity))
            .map(|(i, _)| i)
            .collect()
    }

    fn samples_of(&self, track: TrackId) -> Vec<Rec> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                Event::Sample {
                    track: t,
                    sample,
                    join,
                } if *t == track => Some(Rec {
                    idx: i,
                    now: self.times[i],
                    pts: sample.pts.unwrap(),
                    join: *join,
                }),
                _ => None,
            })
            .collect()
    }

    fn tracks(&self) -> Vec<(TrackId, u32)> {
        let mut out: Vec<(TrackId, u32)> = Vec::new();
        for e in &self.events {
            if let Event::Tracks(specs) = e {
                for s in specs {
                    out.retain(|(t, _)| *t != s.track_id);
                    out.push((s.track_id, s.timescale));
                }
            }
        }
        out
    }
}

fn rtsp_config(timeline_jump: Option<TimelineJumpConfig>) -> Config {
    Config {
        timeline_jump,
        ..quiet_config()
    }
}

fn play_rtsp(
    fix: &MediaFixture,
    plan: &RtspPlan,
    split_frame: usize,
    pause: u64,
    config: Config,
) -> Trace {
    play_rtsp_rendering(fix, plan, split_frame, pause, config, None)
}

fn play_rtsp_rendering(
    fix: &MediaFixture,
    plan: &RtspPlan,
    split_frame: usize,
    pause: u64,
    config: Config,
    rendered: Option<&[TrackId]>,
) -> Trace {
    let mut host = rtsp_host::Host::new(BASE_URL, config, Server::new(&fix.sdp));
    host.connect();
    if let Some(r) = rendered {
        host.session.set_rendered_tracks(r);
    }
    let mut trace = Trace {
        events: Vec::new(),
        times: Vec::new(),
    };
    for e in host.events() {
        trace.events.push(e);
        trace.times.push(host.now);
    }
    for (f, frame) in plan.frames.iter().enumerate() {
        host.now = f as u64 * RTSP_FRAME_US + if f >= split_frame { pause } else { 0 };
        host.deliver(frame);
        host.drain();
        for e in host.events() {
            trace.events.push(e);
            trace.times.push(host.now);
        }
    }
    trace
}

fn stream_of_kind(fix: &MediaFixture, kind: RtpMediaKind) -> usize {
    fix.tracks.iter().position(|t| t.kind == kind).unwrap()
}

fn first_frame_of_au(plan: &RtspPlan, fix: &MediaFixture, stream: usize, au: usize) -> usize {
    let aus = au_of_packets(&fix.tracks[stream].packets);
    plan.streams
        .iter()
        .position(|&(n, i)| n == stream && aus[i] == au)
        .unwrap()
}

#[test]
fn rtsp_jump_detected() {
    let fix = MediaFixture::load();
    let video = stream_of_kind(&fix, RtpMediaKind::H264);
    let clean = rtsp_plan(&fix, &[]);
    let split = first_frame_of_au(&clean, &fix, video, RTSP_VIDEO_AU);
    let base = play_rtsp(
        &fix,
        &clean,
        split,
        LONG_PAUSE_US,
        rtsp_config(Some(TimelineJumpConfig::default())),
    );
    assert!(base.discontinuities().is_empty());
    let vid = base
        .tracks()
        .into_iter()
        .find(|(_, ts)| *ts == RTSP_VIDEO_TIMESCALE)
        .map(|(t, _)| t)
        .unwrap();
    let original = base.samples_of(vid);
    let delta = us_to_ticks(FAR_JUMP_US, RTSP_VIDEO_TIMESCALE);
    let plan = rtsp_plan(
        &fix,
        &[RtpMod {
            stream: video,
            from_au: RTSP_VIDEO_AU,
            delta_ticks: delta,
        }],
    );
    let jumped = play_rtsp(
        &fix,
        &plan,
        split,
        LONG_PAUSE_US,
        rtsp_config(Some(TimelineJumpConfig::default())),
    );
    let got = jumped.samples_of(vid);
    assert_eq!(got.len(), original.len());
    for (j, (g, o)) in got.iter().zip(&original).enumerate() {
        let want = if j < RTSP_VIDEO_AU {
            o.pts
        } else {
            o.pts + delta
        };
        assert_eq!(g.pts, want, "sample {j} retimed as planned");
    }
    let d = jumped.discontinuities();
    assert_eq!(d.len(), 1, "{d:?}");
    match &jumped.events[d[0] + 1] {
        Event::Sample { track, sample, .. } => {
            assert_eq!(*track, vid);
            assert_eq!(sample.pts, Some(got[RTSP_VIDEO_AU].pts));
        }
        other => panic!("{other:?}"),
    }
    let off = play_rtsp(&fix, &plan, split, LONG_PAUSE_US, rtsp_config(None));
    assert!(off.discontinuities().is_empty());
}

fn media_with(extra_video: usize, extra_audio: usize) -> MediaFixture {
    let mut media = MediaFixture::demux();
    let video = media
        .tracks
        .iter()
        .find(|t| t.spec.timescale == RTSP_VIDEO_TIMESCALE)
        .unwrap()
        .clone();
    let audio = media
        .tracks
        .iter()
        .find(|t| t.spec.timescale != RTSP_VIDEO_TIMESCALE)
        .unwrap()
        .clone();
    let mut next_id = 10;
    for _ in 0..extra_video {
        let mut t = video.clone();
        t.spec.track_id = TrackId::new(next_id);
        next_id += 1;
        media.tracks.push(t);
    }
    for _ in 0..extra_audio {
        let mut t = audio.clone();
        t.spec.track_id = TrackId::new(next_id);
        next_id += 1;
        media.tracks.push(t);
    }
    MediaFixture::from_media(&media)
}

fn streams_of(fix: &MediaFixture, kind: RtpMediaKind) -> Vec<usize> {
    fix.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == kind)
        .map(|(i, _)| i)
        .collect()
}

fn video_tracks(t: &Trace) -> Vec<TrackId> {
    let mut v: Vec<TrackId> = t
        .tracks()
        .into_iter()
        .filter(|(_, ts)| *ts == RTSP_VIDEO_TIMESCALE)
        .map(|(id, _)| id)
        .collect();
    v.sort();
    v
}

#[test]
fn multiple_video_tracks_tracked_independently() {
    let fix = media_with(1, 0);
    let videos = streams_of(&fix, RtpMediaKind::H264);
    assert_eq!(videos.len(), 2);
    let offset = RtpMod {
        stream: videos[1],
        from_au: 0,
        delta_ticks: us_to_ticks(100 * SEC_US, RTSP_VIDEO_TIMESCALE),
    };
    let config = || rtsp_config(Some(TimelineJumpConfig::default()));

    let steady = rtsp_plan(&fix, &[offset]);
    let t = play_rtsp(&fix, &steady, usize::MAX, 0, config());
    let ids = video_tracks(&t);
    assert_eq!(ids.len(), 2);
    let a = t.samples_of(ids[0]);
    let b = t.samples_of(ids[1]);
    assert!(a.len() > 60 && b.len() > 60);
    assert!(
        (b[0].pts - a[0].pts).abs() > 99 * i64::from(RTSP_VIDEO_TIMESCALE),
        "the two video tracks run on timelines 100 s apart"
    );
    let interleaved = t
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, .. } if ids.contains(track) => Some(*track),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        interleaved.windows(2).filter(|w| w[0] != w[1]).count() > 20,
        "deliveries alternate between the tracks"
    );
    assert!(
        t.discontinuities().is_empty(),
        "no cross-track false positive"
    );

    let jump = RtpMod {
        stream: videos[1],
        from_au: RTSP_VIDEO_AU,
        delta_ticks: us_to_ticks(FAR_JUMP_US, RTSP_VIDEO_TIMESCALE),
    };
    let plan = rtsp_plan(&fix, &[offset, jump]);
    let t = play_rtsp(&fix, &plan, usize::MAX, 0, config());
    let ids = video_tracks(&t);
    let d = t.discontinuities();
    assert_eq!(
        d.len(),
        1,
        "only the jumping track announces a new timeline"
    );
    let b = t.samples_of(ids[1]);
    match &t.events[d[0] + 1] {
        Event::Sample { track, sample, .. } => {
            assert_eq!(*track, ids[1]);
            assert_eq!(sample.pts, Some(b[RTSP_VIDEO_AU].pts));
        }
        other => panic!("{other:?}"),
    }
}

const LIVE_PLAYLIST_URL: &str = "http://origin.test/live/index.m3u8";
const LIVE_SEG0_URL: &str = "http://origin.test/live/index0.ts";
const LIVE_SEG1_URL: &str = "http://origin.test/live/index1.ts";
const HLS_STEPS: usize = 400;
const HLS_LIVE_START_LEAD_US: u64 = 500_000;

fn hls_trace(h: &hls::Host) -> Trace {
    Trace {
        events: h.events.clone(),
        times: vec![0; h.events.len()],
    }
}

fn starts_of(events: &[Event]) -> Vec<(usize, u64)> {
    events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::Buffer(BufferEvent::Start { at_27mhz }) => Some((i, *at_27mhz)),
            _ => None,
        })
        .collect()
}

fn video_ids(events: &[Event]) -> Vec<TrackId> {
    let mut out = Vec::new();
    for e in events {
        if let Event::Tracks(specs) = e {
            for sp in specs {
                if sp.timescale == VIDEO_TIMESCALE && !out.contains(&sp.track_id) {
                    out.push(sp.track_id);
                }
            }
        }
    }
    out
}

fn video_sample_pts(events: &[Event]) -> Vec<i64> {
    video_pts_of(&video_ids(events), events)
}

fn video_pts_of(ids: &[TrackId], events: &[Event]) -> Vec<i64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } if ids.contains(track) => sample.pts,
            _ => None,
        })
        .collect()
}

fn hls_run(
    playlist_url: &str,
    playlist: Vec<u8>,
    seg0: (&str, Vec<u8>),
    seg1: (&str, Vec<u8>),
    config: Config,
    until_ended: bool,
) -> hls::Host {
    let mut h = hls::Host::new(playlist_url, config);
    h.ok(playlist_url, playlist)
        .route(seg0.0, vec![Reply::Ok(seg0.1)])
        .route(seg1.0, vec![Reply::Ok(seg1.1)]);
    if until_ended {
        h.run_to_end();
    } else {
        h.run_until(
            |evs| video_sample_pts(evs).len() >= HLS_LIVE_VIDEO_SAMPLES,
            HLS_STEPS,
        );
    }
    h
}

const HLS_LIVE_VIDEO_SAMPLES: usize = 70;

fn assert_hls_jump(clean: &hls::Host, jumped: &hls::Host, delta_ticks: i64) {
    let c = hls_trace(clean);
    assert!(c.discontinuities().is_empty(), "clean playlist has no jump");
    assert_eq!(starts_of(&clean.events).len(), 1, "clean: one Start");
    let clean_pts = video_sample_pts(&clean.events);
    let jumped_pts = video_sample_pts(&jumped.events);
    assert_eq!(clean_pts.len(), jumped_pts.len());
    let moved: Vec<usize> = clean_pts
        .iter()
        .zip(&jumped_pts)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert!(!moved.is_empty(), "the second segment really moved");
    assert!(
        clean_pts
            .iter()
            .zip(&jumped_pts)
            .skip(moved[0])
            .all(|(a, b)| (a + delta_ticks).rem_euclid(PTS_WRAP) == *b)
    );
    let first_moved = jumped_pts[moved[0]];
    let t = hls_trace(jumped);
    let d = t.discontinuities();
    assert_eq!(d.len(), 1, "{d:?}");
    let ids = video_ids(&jumped.events);
    let video_after = video_pts_of(&ids, &jumped.events[d[0] + 1..]);
    assert_eq!(
        video_after[0], first_moved,
        "the first sample after the Discontinuity opens the new timeline"
    );
    assert!(
        !video_pts_of(&ids, &jumped.events[..d[0]]).contains(&first_moved),
        "no sample of the new timeline precedes the Discontinuity"
    );
    let starts = starts_of(&jumped.events);
    assert_eq!(
        starts.len(),
        2,
        "the gate reset: Start fires again: {starts:?}"
    );
    assert!(starts[0].0 < d[0] && starts[1].0 > d[0]);
    assert_eq!(
        starts[1].1,
        pts27(first_moved, VIDEO_TIMESCALE),
        "the second Start is anchored at the new timeline's first sample"
    );
}

#[test]
fn hls_vod_playlist_pts_jump_without_discontinuity_tag_resets_gate_and_emits_discontinuity() {
    let seg0 = hls::ts_segment(0);
    let seg1 = hls::ts_segment(1);
    let delta = us_to_ticks(FAR_JUMP_US, VIDEO_TIMESCALE);
    let shifted = shift_pes(&seg1, 0, delta, Shift::Both);
    let playlist = hls::ts_playlist();
    let clean = hls_run(
        hls::TS_PLAYLIST_URL,
        playlist.clone(),
        (hls::TS_SEG0_URL, seg0.clone()),
        (hls::TS_SEG1_URL, seg1),
        hls::hls_config(),
        true,
    );
    let jumped = hls_run(
        hls::TS_PLAYLIST_URL,
        playlist,
        (hls::TS_SEG0_URL, seg0),
        (hls::TS_SEG1_URL, shifted),
        hls::hls_config(),
        true,
    );
    assert_hls_jump(&clean, &jumped, delta);
}

#[test]
fn hls_live_jump_resets_gate() {
    let seg0 = hls::ts_segment(0);
    let seg1 = hls::ts_segment(1);
    let delta = us_to_ticks(FAR_JUMP_US, VIDEO_TIMESCALE);
    let shifted = shift_pes(&seg1, 0, delta, Shift::Both);
    let playlist = hls::live_playlist_text(0, &["index0.ts", "index1.ts"]).into_bytes();
    let config = || {
        let mut c = hls::hls_config();
        c.playout.live_start_lead_us = HLS_LIVE_START_LEAD_US;
        c
    };
    let clean = hls_run(
        LIVE_PLAYLIST_URL,
        playlist.clone(),
        (LIVE_SEG0_URL, seg0.clone()),
        (LIVE_SEG1_URL, seg1),
        config(),
        false,
    );
    let jumped = hls_run(
        LIVE_PLAYLIST_URL,
        playlist,
        (LIVE_SEG0_URL, seg0),
        (LIVE_SEG1_URL, shifted),
        config(),
        false,
    );
    assert_hls_jump(&clean, &jumped, delta);
    let off_config = Config {
        timeline_jump: None,
        ..config()
    };
    let off = hls_run(
        LIVE_PLAYLIST_URL,
        hls::live_playlist_text(0, &["index0.ts", "index1.ts"]).into_bytes(),
        (LIVE_SEG0_URL, hls::ts_segment(0)),
        (
            LIVE_SEG1_URL,
            shift_pes(&hls::ts_segment(1), 0, delta, Shift::Both),
        ),
        off_config,
        false,
    );
    assert!(hls_trace(&off).discontinuities().is_empty());
    assert_eq!(
        starts_of(&off.events).len(),
        1,
        "without the detector the gate never resets"
    );
}

const AUDIO_AU: usize = 20;

fn audio_only_media(audio_tracks: usize) -> MediaFixture {
    let mut media = MediaFixture::demux();
    let audio = media
        .tracks
        .iter()
        .find(|t| t.spec.timescale != RTSP_VIDEO_TIMESCALE)
        .unwrap()
        .clone();
    media.tracks.clear();
    for n in 0..audio_tracks {
        let mut t = audio.clone();
        t.spec.track_id = TrackId::new(u32::try_from(n).unwrap() + 1);
        media.tracks.push(t);
    }
    MediaFixture::from_media(&media)
}

#[test]
fn only_the_first_rendered_audio_track_is_watched() {
    let fix = audio_only_media(2);
    let rate = SMALL_AUDIO_RATE;
    let delta = us_to_ticks(FAR_JUMP_US, 44_100);
    let jump = |stream: usize| {
        rtsp_plan(
            &fix,
            &[RtpMod {
                stream,
                from_au: AUDIO_AU,
                delta_ticks: delta,
            }],
        )
    };
    let config = || rtsp_config(Some(TimelineJumpConfig::default()));
    let clean = play_rtsp(&fix, &rtsp_plan(&fix, &[]), usize::MAX, 0, config());
    assert!(clean.discontinuities().is_empty());
    let mut ids: Vec<TrackId> = clean.tracks().into_iter().map(|(t, _)| t).collect();
    ids.sort();
    assert_eq!(ids.len(), 2, "two audio tracks and no video");

    let first = play_rtsp(&fix, &jump(0), usize::MAX, 0, config());
    assert_eq!(
        first.discontinuities().len(),
        1,
        "the first audio track is watched"
    );
    let second = play_rtsp(&fix, &jump(1), usize::MAX, 0, config());
    assert!(
        second.discontinuities().is_empty(),
        "the second audio track is not"
    );
    let max = second
        .samples_of(ids[1])
        .iter()
        .map(|r| r.pts)
        .max()
        .unwrap();
    assert!(max > 29 * rate, "the second track really jumped");

    let switched = play_rtsp_rendering(&fix, &jump(1), usize::MAX, 0, config(), Some(&[ids[1]]));
    assert_eq!(
        switched.discontinuities().len(),
        1,
        "rendering the second audio track watches it"
    );
}
