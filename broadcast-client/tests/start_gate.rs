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

fn garbage_head(bytes: &[u8], pid: u16, packets: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for p in out.chunks_mut(TS_PACKET).take(packets) {
        if pid_of(p) == pid {
            p[PAYLOAD_START..].fill(GARBAGE);
        }
    }
    out
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
    match hold {
        Hold::Early => Config::default(),
        Hold::Disabled => Config {
            tracks_hold_max_wall_us: 0,
            tracks_hold_max_media_us: 0,
            ..Config::default()
        },
        Hold::Backstop => Config {
            tracks_hold_quiet_input_bytes: len / 2,
            tracks_hold_max_input_bytes: len,
            ..Config::default()
        },
    }
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
                            ends.get(t).map(|&e| {
                                (*t, (i128::from(e) - i128::from(pos)) as i64 / 27_000)
                            })
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
        for l in s.buffer_input_log().iter().filter(|l| !matches!(l, broadcast_client::BufferInput::Chunk { .. })).filter(|l| !matches!(l, broadcast_client::BufferInput::Tick { .. })).take(60) {
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
    Some(match (hold, c.garbage_pid) {
        (Hold::Backstop, Some(pid)) => garbage(&b, pid),
        _ => b,
    })
}

const LATE_AUDIO_US: u64 = 2_500_000;

#[test]
fn probe_late_audio() {
    for c in CAPTURES {
        for hold in [Hold::Early, Hold::Backstop, Hold::Disabled] {
            let Some(b0) = bytes_for(c, Hold::Early) else {
                return;
            };
            let bps = bitrate_bps(&b0);
            let n = (LATE_AUDIO_US * bps / 8 / MICROS) as usize / TS_PACKET;
            let b = garbage_head(&b0, c.audio_pid, n);
            for declare in [Declare::Explicit, Declare::Default] {
                let o = run(&b, config(hold, b.len()), declare);
                eprintln!("LATE {} {:?} {:?} tracks@{:?}", c.name, hold, declare, o.tracks_at_us);
                for m in o.marks.iter().take(4) {
                    eprintln!("   {:>8} us {:?} leads_ms {:?}", m.at_us, m.what, m.leads_ms);
                }
            }
        }
    }
}

#[test]
fn probe() {
    for c in CAPTURES {
        for hold in [Hold::Early, Hold::Backstop, Hold::Disabled] {
            if hold == Hold::Backstop && c.garbage_pid.is_none() {
                continue;
            }
            let Some(b) = bytes_for(c, hold) else {
                return;
            };
            for declare in [Declare::Explicit, Declare::Default] {
                let o = run(&b, config(hold, b.len()), declare);
                eprintln!(
                    "{} {:?} {:?} bps={} tracks@{:?} first@{:?}",
                    c.name, hold, declare, bitrate_bps(&b), o.tracks_at_us, o.first_sample_us
                );
                for m in o.marks.iter().take(4) {
                    eprintln!("   {:>8} us {:?} leads_ms {:?}", m.at_us, m.what, m.leads_ms);
                }
            }
        }
    }
}

#[test]
fn grid() {
    let c = &CAPTURES[0];
    let b0 = bytes_for(c, Hold::Early).unwrap();
    let bps = bitrate_bps(&b0);
    let per_s = (bps / 8 / TS_PACKET as u64) as usize;
    for init_ms in [1800u64, 1100] {
        for delay_ms in [0usize, 500, 1000, 1500] {
            for late_ms in [0usize, 1500, 2500] {
                for hold in [Hold::Early, Hold::Disabled] {
                    let mut b = delay_audio(&b0, c.audio_pid, per_s * delay_ms / 1000);
                    b = garbage_head(&b, c.audio_pid, per_s * late_ms / 1000);
                    let mut cfg = config(hold, b.len());
                    cfg.buffer.policy.initial_us = init_ms * 1000;
                    let o = run(&b, cfg, Declare::Default);
                    let st = o.start_us();
                    let fl = o.flap_within(2_000_000).map(|m| m.at_us);
                    let l = o.marks.first().map(|m| m.leads_ms.clone());
                    eprintln!("init={init_ms} delay={delay_ms} late={late_ms} {hold:?}: start={st:?} flap@={fl:?} leads={l:?}");
                }
            }
        }
    }
}
