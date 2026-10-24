use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use broadcast_client::{Action, BufferEvent, Config, Event, Session};

pub const TS_PACKET: usize = 188;
const PID_MASK: u16 = 0x1fff;
const PCR_FLAG: u8 = 0x10;
const ADAPTATION_PRESENT: u8 = 0x20;
const PCR_BASE_MODULUS_27MHZ: u64 = (1 << 33) * 300;
const PCR_HZ: u64 = 27_000_000;
const FALLBACK_BPS: u64 = 12_000_000;
const MICROS: u64 = 1_000_000;

pub fn bitrate_bps(bytes: &[u8]) -> u64 {
    let mut first: Option<(usize, u16, u64)> = None;
    let mut last: Option<(usize, u64)> = None;
    for (i, p) in bytes.chunks_exact(TS_PACKET).enumerate() {
        if p[0] != 0x47 || p[3] & ADAPTATION_PRESENT == 0 || p[4] < 7 || p[5] & PCR_FLAG == 0 {
            continue;
        }
        let pid = (u16::from(p[1]) << 8 | u16::from(p[2])) & PID_MASK;
        let base = u64::from(p[6]) << 25
            | u64::from(p[7]) << 17
            | u64::from(p[8]) << 9
            | u64::from(p[9]) << 1
            | u64::from(p[10] >> 7);
        let ext = u64::from(p[10] & 1) << 8 | u64::from(p[11]);
        let pcr = base * 300 + ext;
        match first {
            None => first = Some((i, pid, pcr)),
            Some((_, fp, _)) if fp == pid => last = Some((i, pcr)),
            _ => {}
        }
    }
    let (Some((fi, _, fpcr)), Some((li, lpcr))) = (first, last) else {
        return FALLBACK_BPS;
    };
    let dt = (lpcr + PCR_BASE_MODULUS_27MHZ - fpcr) % PCR_BASE_MODULUS_27MHZ;
    if dt == 0 || li <= fi {
        return FALLBACK_BPS;
    }
    ((li - fi) as u64 * TS_PACKET as u64 * 8 * PCR_HZ / dt).max(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleRec {
    pub track: String,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub is_sync: bool,
    pub join: String,
    pub len: usize,
    pub hash: u64,
}

#[derive(Debug, Clone)]
pub struct TimedEvent {
    pub at_us: u64,
    pub event: Event,
}

pub struct Replay {
    pub events: Vec<TimedEvent>,
}

impl Replay {
    pub fn samples(&self) -> Vec<SampleRec> {
        self.events
            .iter()
            .filter_map(|t| match &t.event {
                Event::Sample {
                    track,
                    sample,
                    join,
                } => {
                    let mut h = DefaultHasher::new();
                    sample.data.hash(&mut h);
                    Some(SampleRec {
                        track: format!("{track:?}"),
                        pts: sample.pts,
                        dts: sample.dts,
                        is_sync: sample.flags.is_sync,
                        join: format!("{join:?}"),
                        len: sample.data.len(),
                        hash: h.finish(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    pub fn tracks_times(&self) -> Vec<u64> {
        self.events
            .iter()
            .filter(|t| matches!(t.event, Event::Tracks(_)))
            .map(|t| t.at_us)
            .collect()
    }

    pub fn first_sample_us(&self) -> Option<u64> {
        self.events
            .iter()
            .find(|t| matches!(t.event, Event::Sample { .. }))
            .map(|t| t.at_us)
    }

    pub fn buffer_events(&self) -> Vec<(u64, BufferEvent)> {
        self.events
            .iter()
            .filter_map(|t| match &t.event {
                Event::Buffer(b) => Some((t.at_us, b.clone())),
                _ => None,
            })
            .collect()
    }
}

pub fn open_live(config: Config) -> (Session, u32) {
    let mut s = Session::open("http://h/live.ts", config, 0).unwrap();
    let mut conn = None;
    while let Some(a) = s.poll(0) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    let c = conn.expect("request");
    s.on_response(
        c,
        200,
        &[("Content-Type".to_string(), "video/MP2T".to_string())],
        0,
    );
    (s, c)
}

pub fn replay_paced(bytes: &[u8], config: Config, chunk: usize, bps: u64) -> Replay {
    replay_with_host(bytes, config, chunk, bps, None).0
}

const PTS_HZ_27MHZ: i128 = 27_000_000;
const MICROS_PER_SEC: i128 = 1_000_000;

#[derive(Debug, Default, Clone)]
pub struct HostStats {
    pub delivered_after_start: usize,
    pub late_drops: usize,
    pub late_video_drops: usize,
    pub max_late_us: i128,
    pub start_at_us: Option<u64>,
    pub first_sample_after_start_us: Option<u64>,
    pub pauses: usize,
    pub resumes: usize,
    pub burst_samples_at_start: usize,
}

struct Playout {
    anchor_pos_27: u64,
    anchor_us: u64,
    paused_at: Option<u64>,
}

impl Playout {
    fn position(&self, now_us: u64) -> u64 {
        let at = self.paused_at.map_or(now_us, |p| p.max(self.anchor_us));
        self.anchor_pos_27 + (at - self.anchor_us) * 27
    }
}

pub fn replay_with_host(
    bytes: &[u8],
    config: Config,
    chunk: usize,
    bps: u64,
    tick_us: Option<u64>,
) -> (Replay, HostStats) {
    let (mut s, c) = open_live(config);
    let mut events: Vec<TimedEvent> = Vec::new();
    let mut stats = HostStats::default();
    let mut timescales: std::collections::BTreeMap<String, (u32, bool)> = Default::default();
    let mut playout: Option<Playout> = None;
    let mut wake: Option<u64> = None;
    let mut now = 0u64;
    let mut seen = 0usize;

    let mut step = |s: &mut Session,
                    now: u64,
                    wake: &mut Option<u64>,
                    playout: &mut Option<Playout>,
                    events: &mut Vec<TimedEvent>,
                    stats: &mut HostStats| {
        while let Some(a) = s.poll(now) {
            if let Action::WakeAt(t) = a {
                *wake = Some(t);
            }
        }
        while let Some(event) = s.next_event() {
            match &event {
                Event::Tracks(v) => {
                    for t in v {
                        timescales.insert(
                            format!("{:?}", t.track_id),
                            (t.timescale, t.config.audio_sample_rate().is_none()),
                        );
                    }
                }
                Event::Buffer(BufferEvent::Start { at_27mhz }) => {
                    stats.start_at_us.get_or_insert(now);
                    *playout = Some(Playout {
                        anchor_pos_27: *at_27mhz,
                        anchor_us: now,
                        paused_at: None,
                    });
                    stats.burst_samples_at_start = seen;
                }
                Event::Buffer(BufferEvent::Pause { .. }) => {
                    stats.pauses += 1;
                    if let Some(p) = playout.as_mut() {
                        p.paused_at = Some(now);
                    }
                }
                Event::Buffer(BufferEvent::Resume { at_27mhz }) => {
                    stats.resumes += 1;
                    *playout = Some(Playout {
                        anchor_pos_27: *at_27mhz,
                        anchor_us: now,
                        paused_at: None,
                    });
                }
                Event::Sample { track, sample, .. } => {
                    seen += 1;
                    if let (Some(p), Some(pts), Some((ts, video))) = (
                        playout.as_ref(),
                        sample.pts,
                        timescales.get(&format!("{track:?}")),
                    ) {
                        stats.delivered_after_start += 1;
                        stats.first_sample_after_start_us.get_or_insert(now);
                        let pts27 = i128::from(pts) * PTS_HZ_27MHZ / i128::from(*ts);
                        let late =
                            (i128::from(p.position(now)) - pts27) * MICROS_PER_SEC / PTS_HZ_27MHZ;
                        if late > 0 {
                            stats.late_drops += 1;
                            if *video {
                                stats.late_video_drops += 1;
                            }
                            stats.max_late_us = stats.max_late_us.max(late);
                        }
                    }
                }
                _ => {}
            }
            events.push(TimedEvent { at_us: now, event });
        }
    };

    let mut chunks = bytes.chunks(chunk).peekable();
    let mut next_tick = tick_us.unwrap_or(u64::MAX);
    let mut next_chunk_at = 0u64;
    if let Some(first) = chunks.peek() {
        next_chunk_at = first.len() as u64 * 8 * MICROS / bps;
    }
    while chunks.peek().is_some() {
        let w = wake.unwrap_or(u64::MAX);
        let t = next_chunk_at.min(next_tick).min(w);
        now = now.max(t);
        if t == w && w <= next_chunk_at && w <= next_tick {
            wake = None;
            step(
                &mut s,
                now,
                &mut wake,
                &mut playout,
                &mut events,
                &mut stats,
            );
        } else if t == next_tick && next_tick <= next_chunk_at {
            if let Some(p) = playout.as_ref() {
                s.on_playback_position(now, Some(p.position(now)));
            } else {
                s.on_playback_position(now, None);
            }
            step(
                &mut s,
                now,
                &mut wake,
                &mut playout,
                &mut events,
                &mut stats,
            );
            next_tick = next_tick.saturating_add(tick_us.unwrap_or(u64::MAX));
        } else {
            let part = chunks.next().unwrap();
            s.on_data(c, part, now);
            step(
                &mut s,
                now,
                &mut wake,
                &mut playout,
                &mut events,
                &mut stats,
            );
            if let Some(n) = chunks.peek() {
                next_chunk_at = now + n.len() as u64 * 8 * MICROS / bps;
            }
        }
    }
    s.on_closed(c, None, now);
    step(
        &mut s,
        now,
        &mut wake,
        &mut playout,
        &mut events,
        &mut stats,
    );
    (Replay { events }, stats)
}
