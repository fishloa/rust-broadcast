mod support;

use std::collections::BTreeMap;

use broadcast_client::{Action, BufferEvent, Config, Event, Session, TrackId};
use support::replay::{TS_PACKET, bitrate_bps, open_live};

const CHUNK: usize = 4096;
const TICK_US: u64 = 50_000;
const PID_MASK: u16 = 0x1fff;
const PAYLOAD_START: usize = 4;
const GARBAGE: u8 = 0xa5;
const TICKS_PER_US: u64 = 27;
const MICROS: u64 = 1_000_000;
const BACKSTOP_WALL_US: u64 = 200_000;
const FLAP_WINDOW_US: u64 = 2_000_000;
const DEFAULT_START_LEAD_US: u64 = 1_800_000;
const START_LEADS_US: [u64; 2] = [1_800_000, 1_100_000];
const LEAD_TOLERANCE_PERCENT: i64 = 90;
const SKEWS_MS: [usize; 3] = [0, 500, 1_000];
const SKEW_SLACK_US: u64 = 300_000;
const ALL_HOLDS: [Hold; 3] = [Hold::Early, Hold::Backstop, Hold::Disabled];
const BOTH: [Declare; 2] = [Declare::Explicit, Declare::Default];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hold {
    Early,
    Backstop,
    Disabled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Declare {
    Explicit,
    Default,
}

struct Capture {
    name: &'static str,
    path: &'static str,
    garbage_pid: Option<u16>,
    video_pid: u16,
    audio_pid: u16,
    skip_packets: usize,
}

const CAPTURES: &[Capture] = &[
    Capture {
        name: "rai-gulp",
        path: "zenith-live/rai-gulp-8MB.ts",
        garbage_pid: Some(0x194),
        video_pid: 0xcc,
        audio_pid: 0x130,
        skip_packets: 0,
    },
    Capture {
        name: "rai-news-24",
        path: "zenith-live/rai-news-24-8MB.ts",
        garbage_pid: None,
        video_pid: 0xcf,
        audio_pid: 0x133,
        skip_packets: 0,
    },
    Capture {
        name: "h264_aac_40s",
        path: "ts/h264_aac_40s.ts",
        garbage_pid: None,
        video_pid: 0x100,
        audio_pid: 0x101,
        skip_packets: 0,
    },
    Capture {
        name: "france2",
        path: "ts/france2.ts",
        garbage_pid: None,
        video_pid: 0x78,
        audio_pid: 0x82,
        skip_packets: 0,
    },
];

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1]) << 8 | u16::from(p[2])) & PID_MASK
}

fn delay_audio(bytes: &[u8], pid: u16, packets: usize) -> Vec<u8> {
    let all: Vec<&[u8]> = bytes.chunks(TS_PACKET).collect();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut held: std::collections::VecDeque<(usize, &[u8])> = Default::default();
    for (i, p) in all.iter().enumerate() {
        if pid_of(p) == pid {
            held.push_back((i + packets, p));
        } else {
            out.extend_from_slice(p);
        }
        while held.front().is_some_and(|(due, _)| *due <= i) {
            out.extend_from_slice(held.pop_front().unwrap().1);
        }
    }
    for (_, p) in held {
        out.extend_from_slice(p);
    }
    out
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

fn config(hold: Hold, len: usize) -> Config {
    config_with_lead(hold, len, DEFAULT_START_LEAD_US)
}

fn config_with_lead(hold: Hold, len: usize, start_lead_us: u64) -> Config {
    let mut cfg = match hold {
        Hold::Early => Config::default(),
        Hold::Disabled => Config {
            tracks_hold_max_wall_us: 0,
            tracks_hold_max_media_us: 0,
            ..Config::default()
        },
        Hold::Backstop => Config {
            tracks_hold_quiet_input_bytes: len / 2,
            tracks_hold_max_input_bytes: len,
            tracks_hold_max_wall_us: BACKSTOP_WALL_US,
            ..Config::default()
        },
    };
    cfg.buffer.policy.initial_us = start_lead_us;
    cfg
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    Start,
    Pause,
    Resume,
}

#[derive(Debug, Clone)]
pub struct Mark {
    pub at_us: u64,
    pub what: Transition,
    pub leads_ms: BTreeMap<u32, i64>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub marks: Vec<Mark>,
    pub tracks_at_us: Option<u64>,
    pub first_sample_us: Option<u64>,
    pub rendered: Vec<u32>,
}

impl Outcome {
    pub fn start_us(&self) -> Option<u64> {
        self.marks
            .iter()
            .find(|m| m.what == Transition::Start)
            .map(|m| m.at_us)
    }

    pub fn flap_within(&self, window_us: u64) -> Option<&Mark> {
        let start = self.start_us()?;
        self.marks
            .iter()
            .find(|m| m.what == Transition::Pause && m.at_us <= start + window_us)
    }
}

struct Host {
    anchor_pos: u64,
    anchor_us: u64,
    paused_at: Option<u64>,
}

impl Host {
    fn position(&self, now: u64) -> u64 {
        let at = self.paused_at.map_or(now, |p| p.max(self.anchor_us));
        self.anchor_pos + (at - self.anchor_us) * TICKS_PER_US
    }
}

pub fn run(bytes: &[u8], cfg: Config, declare: Declare) -> Outcome {
    let bps = bitrate_bps(bytes);
    let (mut s, c) = open_live(cfg);
    let mut out = Outcome::default();
    if declare == Declare::Explicit {
        let set = [TrackId::new(1), TrackId::new(2)];
        out.rendered = set.iter().map(|t| t.value()).collect();
        s.set_rendered_tracks(&set);
    }
    let mut timescale: BTreeMap<u32, u32> = BTreeMap::new();
    let mut video: BTreeMap<u32, bool> = BTreeMap::new();
    let mut ends: BTreeMap<u32, u64> = BTreeMap::new();
    let mut host: Option<Host> = None;
    let mut wake: Option<u64> = None;
    let mut now = 0u64;

    let mut step = |s: &mut Session,
                    now: u64,
                    wake: &mut Option<u64>,
                    host: &mut Option<Host>,
                    out: &mut Outcome| {
        while let Some(a) = s.poll(now) {
            if let Action::WakeAt(t) = a {
                *wake = Some(t);
            }
        }
        while let Some(e) = s.next_event() {
            match e {
                Event::Tracks(v) => {
                    out.tracks_at_us.get_or_insert(now);
                    for t in &v {
                        timescale.insert(t.track_id.value(), t.timescale);
                        video.insert(t.track_id.value(), t.config.audio_sample_rate().is_none());
                    }
                }
                Event::Sample { track, sample, .. } => {
                    out.first_sample_us.get_or_insert(now);
                    if let (Some(pts), Some(&ts)) = (sample.pts, timescale.get(&track.value())) {
                        let end = (i128::from(pts) + i128::from(sample.duration.unwrap_or(0)))
                            * 27_000_000
                            / i128::from(ts);
                        let end = u64::try_from(end.max(0)).unwrap_or(0);
                        let e = ends.entry(track.value()).or_insert(0);
                        *e = (*e).max(end);
                    }
                }
                Event::Buffer(b) => {
                    let (what, pos) = match &b {
                        BufferEvent::Start { at_27mhz } => (Transition::Start, *at_27mhz),
                        BufferEvent::Pause { hold_at_27mhz, .. } => {
                            (Transition::Pause, *hold_at_27mhz)
                        }
                        BufferEvent::Resume { at_27mhz } => (Transition::Resume, *at_27mhz),
                        _ => continue,
                    };
                    let rendered: Vec<u32> = if out.rendered.is_empty() {
                        let (v_id, a_id) = s.default_rendered_tracks();
                        v_id.into_iter().chain(a_id).map(|t| t.value()).collect()
                    } else {
                        out.rendered.clone()
                    };
                    let leads_ms = rendered
                        .iter()
                        .filter_map(|t| {
                            ends.get(t)
                                .map(|&e| (*t, (i128::from(e) - i128::from(pos)) as i64 / 27_000))
                        })
                        .collect();
                    out.marks.push(Mark {
                        at_us: now,
                        what: what.clone(),
                        leads_ms,
                    });
                    match what {
                        Transition::Pause => {
                            if let Some(h) = host.as_mut() {
                                h.paused_at = Some(now);
                            }
                        }
                        _ => {
                            *host = Some(Host {
                                anchor_pos: pos,
                                anchor_us: now,
                                paused_at: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    };

    let mut chunks = bytes.chunks(CHUNK).peekable();
    let mut next_chunk_at = chunks
        .peek()
        .map_or(0, |f| f.len() as u64 * 8 * MICROS / bps);
    let mut next_tick = TICK_US;
    while chunks.peek().is_some() {
        let w = wake.unwrap_or(u64::MAX);
        let t = next_chunk_at.min(next_tick).min(w);
        now = now.max(t);
        if t == w && w <= next_chunk_at && w <= next_tick {
            wake = None;
            step(&mut s, now, &mut wake, &mut host, &mut out);
        } else if t == next_tick && next_tick <= next_chunk_at {
            s.on_playback_position(now, host.as_ref().map(|h| h.position(now)));
            step(&mut s, now, &mut wake, &mut host, &mut out);
            next_tick += TICK_US;
        } else {
            let part = chunks.next().unwrap();
            s.on_data(c, part, now);
            step(&mut s, now, &mut wake, &mut host, &mut out);
            if let Some(n) = chunks.peek() {
                next_chunk_at = now + n.len() as u64 * 8 * MICROS / bps;
            }
        }
    }
    #[cfg(feature = "test-seams")]
    if std::env::var("DUMP").is_ok() {
        for l in s
            .buffer_input_log()
            .iter()
            .filter(|l| !matches!(l, broadcast_client::BufferInput::Chunk { .. }))
            .filter(|l| !matches!(l, broadcast_client::BufferInput::Tick { .. }))
            .take(60)
        {
            eprintln!("{l:?}");
        }
    }
    out
}

const CC_MASK: u8 = 0x0f;
const HAS_PAYLOAD: u8 = 0x10;
const NULL_PID: u16 = 0x1fff;

fn continuous(mut b: Vec<u8>) -> Vec<u8> {
    let mut next: BTreeMap<u16, u8> = BTreeMap::new();
    for p in b.chunks_mut(TS_PACKET) {
        let pid = pid_of(p);
        if pid == NULL_PID || p[3] & HAS_PAYLOAD == 0 {
            continue;
        }
        let cc = next.entry(pid).or_insert(p[3] & CC_MASK);
        p[3] = p[3] & !CC_MASK | *cc;
        *cc = (*cc + 1) & CC_MASK;
    }
    b
}

fn trimmed(b: Vec<u8>, c: &Capture) -> Vec<u8> {
    let b = continuous(b);
    if c.skip_packets == 0 {
        return b;
    }
    let psi = b
        .chunks(TS_PACKET)
        .position(|p| pid_of(p) == c.video_pid)
        .unwrap();
    let mut out = b[..psi * TS_PACKET].to_vec();
    out.extend_from_slice(&b[c.skip_packets * TS_PACKET..]);
    out
}

fn bytes_for(c: &Capture, hold: Hold) -> Option<Vec<u8>> {
    let b = trimmed(test_fixtures::require_bytes(c.path, "start_gate")?, c);
    let _ = hold;
    Some(b)
}

fn skewed(c: &Capture, skew_ms: usize) -> Option<Vec<u8>> {
    let b = bytes_for(c, Hold::Early)?;
    let per_s = (bitrate_bps(&b) / 8 / TS_PACKET as u64) as usize;
    Some(delay_audio(&b, c.audio_pid, per_s * skew_ms / 1000))
}

fn describe(c: &Capture, hold: Hold, declare: Declare, skew_ms: usize, o: &Outcome) -> String {
    format!(
        "{} {hold:?} {declare:?} skew {skew_ms} ms: start {:?} marks {:?}",
        c.name,
        o.start_us(),
        o.marks
            .iter()
            .take(3)
            .map(|m| (m.at_us, &m.what, &m.leads_ms))
            .collect::<Vec<_>>()
    )
}

#[test]
fn start_never_precedes_a_rendered_track_that_trails_the_video() {
    let mut ran = 0;
    for c in CAPTURES {
        for skew in SKEWS_MS {
            let Some(b) = skewed(c, skew) else {
                return;
            };
            for (hold, lead_us) in ALL_HOLDS
                .into_iter()
                .flat_map(|h| START_LEADS_US.map(|l| (h, l)))
            {
                for declare in BOTH {
                    let o = run(&b, config_with_lead(hold, b.len(), lead_us), declare);
                    let why = format!(
                        "lead {lead_us} us: {}",
                        describe(c, hold, declare, skew, &o)
                    );
                    let Some(start) = o.marks.iter().find(|m| m.what == Transition::Start) else {
                        continue;
                    };
                    ran += 1;
                    assert_eq!(start.leads_ms.len(), 2, "audio and video both led: {why}");
                    for (track, lead) in &start.leads_ms {
                        assert!(
                            *lead >= lead_us as i64 / 1_000 * LEAD_TOLERANCE_PERCENT / 100,
                            "track {track} had only {lead} ms at Start: {why}"
                        );
                    }
                    assert!(
                        o.flap_within(FLAP_WINDOW_US).is_none(),
                        "Start then Pause: {why}"
                    );
                }
            }
        }
    }
    assert!(ran > 0, "no capture reached Start");
}

#[test]
fn start_latency_grows_by_no_more_than_the_audio_skew() {
    for c in CAPTURES {
        for (hold, lead_us) in ALL_HOLDS
            .into_iter()
            .flat_map(|h| START_LEADS_US.map(|l| (h, l)))
        {
            let Some(base) = skewed(c, 0) else {
                return;
            };
            let Some(base_start) = run(
                &base,
                config_with_lead(hold, base.len(), lead_us),
                Declare::Default,
            )
            .start_us() else {
                continue;
            };
            for skew in SKEWS_MS {
                let b = skewed(c, skew).unwrap();
                for declare in BOTH {
                    let o = run(&b, config_with_lead(hold, b.len(), lead_us), declare);
                    let start = o.start_us().unwrap_or(u64::MAX);
                    let allowed = base_start + skew as u64 * 1_000 + SKEW_SLACK_US;
                    assert!(
                        start <= allowed,
                        "{}: start {start} us, no-skew start {base_start} us + skew {skew} ms: {}",
                        c.name,
                        describe(c, hold, declare, skew, &o)
                    );
                }
            }
        }
    }
}

#[test]
fn a_declared_audio_pid_that_never_sends_does_not_block_start_for_long() {
    for c in CAPTURES {
        let Some(b0) = bytes_for(c, Hold::Early) else {
            return;
        };
        let b: Vec<u8> = b0
            .chunks(TS_PACKET)
            .filter(|p| pid_of(p) != c.audio_pid)
            .flatten()
            .copied()
            .collect();
        for hold in ALL_HOLDS {
            for declare in [Declare::Default] {
                let o = run(&b, config(hold, b.len()), declare);
                let why = describe(c, hold, declare, 0, &o);
                let start = o
                    .start_us()
                    .unwrap_or_else(|| panic!("never started: {why}"));
                let first = o.first_sample_us.unwrap();
                assert!(
                    start <= first + START_BOUND_US,
                    "start {start} us is more than {START_BOUND_US} us after first media {first}: {why}"
                );
                assert!(o.flap_within(FLAP_WINDOW_US).is_none(), "{why}");
            }
        }
    }
}

const START_BOUND_US: u64 = 4_000_000;

#[test]
fn start_waits_for_audio_when_an_unresolvable_pid_is_in_the_mux() {
    let c = &CAPTURES[0];
    let Some(b0) = bytes_for(c, Hold::Early) else {
        return;
    };
    let per_s = (bitrate_bps(&b0) / 8 / TS_PACKET as u64) as usize;
    let b = garbage(
        &delay_audio(&b0, c.audio_pid, per_s * 500 / 1000),
        c.garbage_pid.unwrap(),
    );
    for lead_us in START_LEADS_US {
        for declare in BOTH {
            let mut cfg = config_with_lead(Hold::Early, b.len(), lead_us);
            cfg.tracks_hold_quiet_input_bytes = b.len() / 2;
            cfg.tracks_hold_max_input_bytes = b.len();
            let o = run(&b, cfg, declare);
            let why = format!(
                "lead {lead_us} us: {}",
                describe(c, Hold::Early, declare, 500, &o)
            );
            let start = o
                .marks
                .iter()
                .find(|m| m.what == Transition::Start)
                .expect(&why);
            assert_eq!(start.leads_ms.len(), 2, "{why}");
            assert!(o.flap_within(FLAP_WINDOW_US).is_none(), "{why}");
        }
    }
}
