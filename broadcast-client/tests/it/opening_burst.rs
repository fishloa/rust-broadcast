use std::collections::BTreeMap;
use std::fmt;

use broadcast_client::{Action, BufferEvent, Config, Event, PauseReason, Session, TrackId};

use crate::support;
use crate::support::video_run::VideoRun;
use crate::support::replay::open_live;
use crate::support::ts_shift::{PACKET, Shift, packet_has_pcr, packet_pid, shift_pes};

const PAT_PID: u16 = 0x00;
const TF1_PMT_PID: u16 = 0x64;
const TF1_VIDEO_PID: u16 = 0x78;
const TF1_AUDIO_PIDS: std::ops::RangeInclusive<u16> = 0x82..=0x84;
const PES_PTS_HZ: u64 = 90_000;
const PCR_PER_PTS_TICK: u64 = 300;
const TICKS_27_PER_US: u64 = 27;
const MICROS: u64 = 1_000_000;
const CHUNK: usize = 8 * PACKET;
const TIMER_US: u64 = 20_000;
const DECLARE_AFTER_US: u64 = 30_000;
const FILL_US: u64 = 500_000;
const STEADY_AFTER_US: u64 = 10_000_000;
const GAP_AT_US: u64 = 3_000_000;
const GAP_LEN_US: u64 = 1_400_000;
const LOOP_START_COPY: usize = 1;
const MB12: usize = 12 * 1024 * 1024;
const SERVER_MAX_BURST_PACKETS: usize = 20_000;
const SERVER_MAX_BURST_US: u64 = 4_000_000;
const MISSING_TRACK: u32 = u32::MAX;
const AF_RANDOM_ACCESS: u8 = 0x40;
const AF_PRESENT: u8 = 0x20;
const PAYLOAD_PRESENT: u8 = 0x10;
const CC_MASK: u8 = 0x0f;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartPoint {
    Keyframe,
    MidGop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Burst {
    Media(u64),
    GopTail,
    Bytes(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Muxed,
    AudioLater(u64),
    VideoLater(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Throughput {
    Instant,
    Mbps(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Steady,
    Gap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    start: StartPoint,
    burst: Burst,
    order: Order,
    throughput: Throughput,
    after: After,
    audio: usize,
    declare: Declare,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Declare {
    WhenBoth,
    EachTracks,
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let burst = match self.burst {
            Burst::Media(us) => format!("{} ms", us / 1000),
            Burst::GopTail => "whole GOP".to_string(),
            Burst::Bytes(b) => format!("{} KiB", b / 1024),
        };
        let order = match self.order {
            Order::Muxed => "muxed".to_string(),
            Order::AudioLater(us) => format!("video +{} ms", us / 1000),
            Order::VideoLater(us) => format!("audio +{} ms", us / 1000),
        };
        let thr = match self.throughput {
            Throughput::Instant => "instant".to_string(),
            Throughput::Mbps(m) => format!("{m} Mbit/s"),
        };
        write!(
            f,
            "{:?} burst={burst} order={order} {thr} {:?} audio#{} {:?}",
            self.start, self.after, self.audio, self.declare
        )
    }
}

struct Stream {
    packets: Vec<u8>,
    at_us: Vec<u64>,
    copy_us: u64,
}

impl Stream {
    fn len(&self) -> usize {
        self.at_us.len()
    }

    fn packet(&self, i: usize) -> &[u8] {
        &self.packets[i * PACKET..(i + 1) * PACKET]
    }
}

fn read_ts(b: &[u8]) -> i64 {
    (i64::from(b[0] >> 1) & 7) << 30
        | i64::from(b[1]) << 22
        | (i64::from(b[2]) >> 1) << 15
        | i64::from(b[3]) << 7
        | i64::from(b[4]) >> 1
}

fn pes_decode_ts(p: &[u8]) -> Option<i64> {
    if p[1] & 0x40 == 0 || (p[3] >> 4) & 1 == 0 {
        return None;
    }
    let off = 4 + if p[3] & AF_PRESENT != 0 {
        1 + usize::from(p[4])
    } else {
        0
    };
    if off + 19 > PACKET || p[off..off + 3] != [0, 0, 1] || p[off + 7] & 0x80 == 0 {
        return None;
    }
    let hdr = off + 9;
    Some(if p[off + 7] & 0x40 != 0 {
        read_ts(&p[hdr + 5..hdr + 10])
    } else {
        read_ts(&p[hdr..hdr + 5])
    })
}

fn pcr_base(p: &[u8]) -> u64 {
    u64::from(p[6]) << 25
        | u64::from(p[7]) << 17
        | u64::from(p[8]) << 9
        | u64::from(p[9]) << 1
        | u64::from(p[10]) >> 7
}

fn set_pcr_base(p: &mut [u8], v: u64) {
    p[6] = (v >> 25) as u8;
    p[7] = (v >> 17) as u8;
    p[8] = (v >> 9) as u8;
    p[9] = (v >> 1) as u8;
    p[10] = (p[10] & 0x7f) | (((v & 1) as u8) << 7);
}

fn is_rap(p: &[u8]) -> bool {
    packet_pid(p) == TF1_VIDEO_PID
        && p[3] & AF_PRESENT != 0
        && p[4] > 0
        && p[5] & AF_RANDOM_ACCESS != 0
}

fn is_audio(pid: u16) -> bool {
    TF1_AUDIO_PIDS.contains(&pid)
}

fn loop_plan(raw: &[u8]) -> (i64, Vec<bool>) {
    let mut ts: BTreeMap<u16, Vec<i64>> = BTreeMap::new();
    for p in raw.chunks_exact(PACKET) {
        if let Some(v) = pes_decode_ts(p) {
            ts.entry(packet_pid(p)).or_default().push(v);
        }
    }
    let shape = |v: &Vec<i64>| {
        let mut s = v.clone();
        s.sort_unstable();
        let step = s
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|d| *d > 0)
            .min()
            .unwrap();
        (s[s.len() - 1] - s[0] + step, step)
    };
    let media: Vec<(i64, i64)> = ts
        .iter()
        .filter(|(pid, _)| **pid == TF1_VIDEO_PID || is_audio(**pid))
        .map(|(_, v)| shape(v))
        .collect();
    let grid = media.iter().fold(1, |g, (_, step)| lcm(g, *step));
    let span = media.iter().map(|(span, _)| *span).min().unwrap();
    let period = span / grid * grid;
    let pcr0 = raw
        .chunks_exact(PACKET)
        .find(|p| packet_has_pcr(p))
        .map(pcr_base)
        .unwrap();
    let cut = raw
        .chunks_exact(PACKET)
        .position(|p| packet_has_pcr(p) && pcr_base(p) >= pcr0 + u64::try_from(period).unwrap())
        .unwrap();
    let mut drop = vec![false; raw.len() / PACKET];
    let mut dropping: BTreeMap<u16, bool> = BTreeMap::new();
    for (i, p) in raw.chunks_exact(PACKET).enumerate() {
        let pid = packet_pid(p);
        let Some(first) = ts.get(&pid).map(|v| v[0]) else {
            drop[i] = i >= cut;
            continue;
        };
        if let Some(v) = pes_decode_ts(p) {
            dropping.insert(pid, is_audio(pid) && v >= first + period);
        }
        drop[i] = (i >= cut && !is_audio(pid)) || dropping.get(&pid).copied().unwrap_or(true);
    }
    (period, drop)
}

fn lcm(a: i64, b: i64) -> i64 {
    let gcd = |mut x: i64, mut y: i64| {
        while y != 0 {
            (x, y) = (y, x % y);
        }
        x
    };
    a / gcd(a, b) * b
}

fn looped(raw: &[u8], copies: usize) -> (Vec<u8>, u64) {
    let (period, drop) = loop_plan(raw);
    let mut out = Vec::with_capacity(raw.len() * copies);
    let mut cc: BTreeMap<u16, u8> = BTreeMap::new();
    for k in 0..copies {
        let delta = period * i64::try_from(k).unwrap();
        let shifted = shift_pes(raw, 0, delta, Shift::Both);
        for (i, p) in shifted.chunks_exact(PACKET).enumerate() {
            if drop[i] {
                continue;
            }
            let mut p = p.to_vec();
            if packet_has_pcr(&p) {
                let v = pcr_base(&p) + u64::try_from(delta).unwrap();
                set_pcr_base(&mut p, v);
            }
            if p[3] & PAYLOAD_PRESENT != 0 {
                let pid = packet_pid(&p);
                let next = cc.get(&pid).map_or(p[3] & CC_MASK, |c| (c + 1) & CC_MASK);
                p[3] = (p[3] & !CC_MASK) | next;
                cc.insert(pid, next);
            }
            out.extend_from_slice(&p);
        }
    }
    (out, u64::try_from(period).unwrap() * MICROS / PES_PTS_HZ)
}

fn schedule(bytes: &[u8]) -> Vec<u64> {
    let pcrs: Vec<(usize, u64)> = bytes
        .chunks_exact(PACKET)
        .enumerate()
        .filter(|(_, p)| packet_has_pcr(p) && packet_pid(p) == TF1_VIDEO_PID)
        .map(|(i, p)| (i, pcr_base(p)))
        .collect();
    let base = pcrs[0].1;
    let us = |pcr: u64| (pcr - base) * MICROS / PES_PTS_HZ;
    let n = bytes.len() / PACKET;
    let mut out = vec![0u64; n];
    for w in pcrs.windows(2) {
        let ((i0, p0), (i1, p1)) = (w[0], w[1]);
        let (t0, t1) = (us(p0), us(p1));
        for (i, slot) in out.iter_mut().enumerate().take(i1).skip(i0) {
            *slot = t0 + (t1 - t0) * (i - i0) as u64 / (i1 - i0) as u64;
        }
    }
    let (first, last) = (pcrs[0], pcrs[pcrs.len() - 1]);
    let rate = (us(last.1) - us(first.1)) as f64 / (last.0 - first.0) as f64;
    for (i, slot) in out.iter_mut().enumerate().skip(last.0) {
        *slot = us(last.1) + ((i - last.0) as f64 * rate) as u64;
    }
    out
}

fn reorder(bytes: &[u8], at: &[u64], copy_us: u64, order: Order) -> Stream {
    let delay = |pid: u16| match order {
        Order::Muxed => 0,
        Order::AudioLater(d) if is_audio(pid) => d,
        Order::VideoLater(d) if pid == TF1_VIDEO_PID => d,
        _ => 0,
    };
    let mut idx: Vec<(u64, usize)> = bytes
        .chunks_exact(PACKET)
        .enumerate()
        .map(|(i, p)| (at[i] + delay(packet_pid(p)), i))
        .collect();
    idx.sort_by_key(|(t, i)| (*t, *i));
    let mut packets = Vec::with_capacity(bytes.len());
    for (_, i) in &idx {
        packets.extend_from_slice(&bytes[i * PACKET..(i + 1) * PACKET]);
    }
    Stream {
        packets,
        at_us: idx.iter().map(|(t, _)| *t).collect(),
        copy_us,
    }
}

struct Plan {
    deliveries: Vec<(u64, Vec<u8>)>,
    burst_bytes: usize,
    burst_media_us: u64,
}

fn first_at_or_after(s: &Stream, from: usize, t: u64) -> usize {
    (from..s.len()).find(|&i| s.at_us[i] >= t).unwrap()
}

fn plan(s: &Stream, row: Row) -> Plan {
    let first_copy_end = s.copy_us * LOOP_START_COPY as u64;
    let raps: Vec<usize> = (0..s.len())
        .filter(|&i| {
            is_rap(s.packet(i))
                && s.at_us[i] >= first_copy_end
                && s.at_us[i] < first_copy_end + s.copy_us
        })
        .collect();
    let (key, next_key) = raps
        .windows(2)
        .map(|w| (w[0], w[1]))
        .max_by_key(|(a, b)| s.at_us[*b] - s.at_us[*a])
        .unwrap();
    let begin = match row.start {
        StartPoint::Keyframe => key,
        StartPoint::MidGop => first_at_or_after(s, key, (s.at_us[key] + s.at_us[next_key]) / 2),
    };
    let connect = match row.burst {
        Burst::Media(us) => first_at_or_after(s, begin, s.at_us[begin] + us),
        Burst::GopTail => next_key,
        Burst::Bytes(b) => begin + b / PACKET,
    };
    let psi = |pid: u16| {
        (0..begin)
            .rev()
            .find(|&i| packet_pid(s.packet(i)) == pid)
            .unwrap()
    };
    let mut burst = Vec::new();
    for i in [psi(PAT_PID), psi(TF1_PMT_PID)] {
        burst.extend_from_slice(s.packet(i));
    }
    for i in begin..connect {
        burst.extend_from_slice(s.packet(i));
    }
    let link_us = |bytes: usize| match row.throughput {
        Throughput::Instant => 0,
        Throughput::Mbps(m) => bytes as u64 * 8 / m,
    };
    let mut deliveries = Vec::new();
    let mut sent = 0usize;
    match row.throughput {
        Throughput::Instant => {
            sent = burst.len();
            deliveries.push((0, burst.clone()));
        }
        Throughput::Mbps(_) => {
            for c in burst.chunks(CHUNK) {
                sent += c.len();
                deliveries.push((link_us(sent), c.to_vec()));
            }
        }
    }
    let live0 = s.at_us[connect];
    let end_us = link_us(sent) + STEADY_AFTER_US;
    let mut i = connect;
    while i < s.len() {
        let j = (i + CHUNK / PACKET).min(s.len());
        let mut avail = s.at_us[j - 1] - live0;
        if avail > end_us {
            break;
        }
        if row.after == After::Gap && (GAP_AT_US..GAP_AT_US + GAP_LEN_US).contains(&avail) {
            avail = GAP_AT_US + GAP_LEN_US;
        }
        sent += (j - i) * PACKET;
        let at = avail
            .max(link_us(sent))
            .max(deliveries.last().map_or(0, |d| d.0));
        deliveries.push((at, s.packets[i * PACKET..j * PACKET].to_vec()));
        i = j;
    }
    assert!(
        deliveries.last().unwrap().0 + MICROS >= end_us,
        "{row}: looped stream too short"
    );
    Plan {
        deliveries,
        burst_bytes: (connect - begin) * PACKET,
        burst_media_us: live0 - s.at_us[begin],
    }
}

#[derive(Default, Debug)]
struct Outcome {
    start: Option<(u64, BTreeMap<u32, i64>)>,
    pauses: Vec<(u64, PauseReason, Option<u32>)>,
    resumes: Vec<u64>,
    min_lead: BTreeMap<u32, (i64, u64)>,
    failures: Vec<String>,
}

struct Renderer {
    anchor: Option<(u64, u64)>,
    frozen: Option<u64>,
}

impl Renderer {
    fn position(&self, now: u64) -> Option<u64> {
        if let Some(f) = self.frozen {
            return Some(f);
        }
        self.anchor
            .map(|(pos, at)| pos + (now - at) * TICKS_27_PER_US)
    }

    fn playing(&self) -> bool {
        self.anchor.is_some() && self.frozen.is_none()
    }
}

fn run(plan: &Plan, row: Row, starve_us: u64) -> Outcome {
    let mut cfg = Config::default();
    cfg.buffer.policy.initial_us = FILL_US;
    let (mut s, conn) = open_live(cfg);
    let mut out = Outcome::default();
    let mut kinds: BTreeMap<u32, (u32, bool, bool)> = BTreeMap::new();
    let mut ends: BTreeMap<u32, u64> = BTreeMap::new();
    let mut runs: BTreeMap<u32, VideoRun> = BTreeMap::new();
    let mut rendered: Vec<u32> = Vec::new();
    let mut intended: Vec<u32> = vec![MISSING_TRACK];
    let mut starved = std::collections::BTreeSet::new();
    let mut declare_at: Option<u64> = None;
    let mut r = Renderer {
        anchor: None,
        frozen: None,
    };
    let mut wake: Option<u64> = None;
    let mut next_tick = TIMER_US;
    let mut deliveries = plan.deliveries.iter().peekable();
    let stop = plan.deliveries.last().unwrap().0;
    loop {
        let chunk_at = deliveries.peek().map(|d| d.0);
        let Some(chunk_at) = chunk_at else { break };
        let now = chunk_at
            .min(next_tick)
            .min(declare_at.unwrap_or(u64::MAX))
            .min(wake.unwrap_or(u64::MAX));
        if declare_at == Some(now) {
            declare_at = None;
            let ids: Vec<TrackId> = rendered.iter().map(|t| TrackId::new(*t)).collect();
            s.set_rendered_tracks(now, &ids);
        } else if now == chunk_at {
            let (_, bytes) = deliveries.next().unwrap();
            s.on_data(conn, bytes, now);
            s.on_playback_position(now, r.position(now));
        } else if now == next_tick {
            next_tick += TIMER_US;
            s.on_playback_position(now, r.position(now));
        } else {
            wake = None;
        }
        drain(&mut s, now, &mut wake);
        while let Some(e) = s.next_event() {
            match e {
                Event::Tracks(v) => {
                    for t in &v {
                        kinds.insert(
                            t.track_id.value(),
                            (
                                t.timescale,
                                t.config.is_video(),
                                t.config.audio_sample_rate().is_some(),
                            ),
                        );
                    }
                    let video = kinds.iter().find(|(_, k)| k.1).map(|(t, _)| *t);
                    let audio = kinds
                        .iter()
                        .filter(|(_, k)| k.2)
                        .map(|(t, _)| *t)
                        .nth(row.audio);
                    intended = video.into_iter().chain(audio).collect();
                    let target = match (row.declare, video, audio) {
                        (Declare::WhenBoth, Some(v), Some(a)) => vec![v, a],
                        (Declare::EachTracks, Some(_), _) => intended.clone(),
                        _ => rendered.clone(),
                    };
                    if target != rendered {
                        rendered = target;
                        declare_at = Some(now + DECLARE_AFTER_US);
                    }
                    if intended.len() < 2 {
                        intended.push(MISSING_TRACK);
                    }
                }
                Event::Sample { track, sample, .. } => {
                    if let (Some(pts), Some((ts, is_video, _))) =
                        (sample.pts, kinds.get(&track.value()))
                    {
                        let dur = i64::from(sample.duration.unwrap_or(0));
                        let end = if *is_video {
                            i128::from(
                                runs.entry(track.value())
                                    .or_insert_with(VideoRun::new)
                                    .push(pts, sample.dts.unwrap_or(pts), dur),
                            )
                        } else {
                            i128::from(pts) + i128::from(dur)
                        };
                        let end27 = u64::try_from(
                            end * i128::from(PES_PTS_HZ * PCR_PER_PTS_TICK) / i128::from(*ts),
                        )
                        .unwrap();
                        let e = ends.entry(track.value()).or_insert(0);
                        *e = (*e).max(end27);
                    }
                }
                Event::Buffer(BufferEvent::Start { at_27mhz }) => {
                    let leads = intended
                        .iter()
                        .map(|t| (*t, lead_us(ends.get(t), at_27mhz)))
                        .collect();
                    out.start = Some((now, leads));
                    r.anchor = Some((at_27mhz, now));
                    r.frozen = None;
                }
                Event::Buffer(BufferEvent::Resume { at_27mhz }) => {
                    out.resumes.push(now);
                    r.anchor = Some((at_27mhz, now));
                    r.frozen = None;
                }
                Event::Buffer(BufferEvent::Pause {
                    hold_at_27mhz,
                    reason,
                    starving_track,
                }) => {
                    out.pauses.push((now, reason, starving_track));
                    r.frozen = Some(hold_at_27mhz);
                }
                _ => {}
            }
        }
        if r.playing() && now < stop {
            let pos = r.position(now).unwrap();
            for t in &intended {
                let lead = lead_us(ends.get(t), pos);
                let m = out.min_lead.entry(*t).or_insert((i64::MAX, 0));
                if lead < m.0 {
                    *m = (lead, now);
                }
                if lead < i64::try_from(starve_us).unwrap() && starved.insert(*t) {
                    out.failures.push(format!(
                        "(d) track {t} lead {lead} us < starve threshold at {now} us while Playing"
                    ));
                }
            }
        }
    }
    out
}

fn lead_us(end: Option<&u64>, pos: u64) -> i64 {
    end.map_or(i64::MIN / 2, |e| {
        (i64::try_from(*e).unwrap() - i64::try_from(pos).unwrap()) / TICKS_27_PER_US as i64
    })
}

fn drain(s: &mut Session, now: u64, wake: &mut Option<u64>) {
    while let Some(a) = s.poll(now) {
        if let Action::WakeAt(t) = a
            && t > now
        {
            *wake = Some(wake.map_or(t, |w| w.min(t)));
        }
    }
}

fn judge(row: Row, o: &mut Outcome) {
    match &o.start {
        None => o.failures.push("no Start".to_string()),
        Some((_, leads)) => {
            for (t, l) in leads {
                if *l < i64::try_from(FILL_US).unwrap() {
                    o.failures.push(format!(
                        "(a) track {t} lead {l} us at Start < fill {FILL_US} us"
                    ));
                }
            }
        }
    }
    for (at, reason, track) in &o.pauses {
        if *reason == PauseReason::DataArriving {
            o.failures.push(format!(
                "(b) Pause DataArriving at {at} us, track {track:?}"
            ));
        }
    }
    match row.after {
        After::Steady => {
            if !o.pauses.is_empty() {
                o.failures
                    .push(format!("(b) pauses in steady real time: {:?}", o.pauses));
            }
        }
        After::Gap => {
            let ok = o.pauses.len() <= 1
                && o.pauses.iter().all(|p| p.1 == PauseReason::NoData)
                && o.resumes.len() == o.pauses.len();
            if !ok {
                o.failures.push(format!(
                    "(c) gap row: pauses {:?} resumes {:?}",
                    o.pauses, o.resumes
                ));
            }
        }
    }
}

fn realistic_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    for after in [After::Steady, After::Gap] {
        for burst in [
            0, 110_000, 250_000, 500_000, 1_000_000, 1_500_000, 2_000_000,
        ]
        .map(Burst::Media)
        .into_iter()
        .chain([Burst::GopTail])
        {
            rows.push(Row {
                start: StartPoint::Keyframe,
                burst,
                order: Order::Muxed,
                throughput: Throughput::Instant,
                after,
                audio: 0,
                declare: Declare::WhenBoth,
            });
        }
        rows.push(Row {
            start: StartPoint::MidGop,
            burst: Burst::Media(0),
            order: Order::Muxed,
            throughput: Throughput::Instant,
            after,
            audio: 0,
            declare: Declare::WhenBoth,
        });
    }
    rows
}

fn field_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    for burst in [256, 384, 512].map(|k| Burst::Bytes(k * 1024)) {
        for order in [
            Order::Muxed,
            Order::AudioLater(150_000),
            Order::AudioLater(250_000),
        ] {
            for audio in 0..TF1_AUDIO_PIDS.len() {
                for declare in [Declare::WhenBoth, Declare::EachTracks] {
                    rows.push(Row {
                        start: StartPoint::Keyframe,
                        burst,
                        order,
                        throughput: Throughput::Instant,
                        after: After::Steady,
                        audio,
                        declare,
                    });
                }
            }
        }
    }
    rows
}

const BURSTS: [Burst; 5] = [
    Burst::Media(0),
    Burst::Media(500_000),
    Burst::Media(2_000_000),
    Burst::Media(6_000_000),
    Burst::Bytes(MB12),
];
const ORDERS: [Order; 4] = [
    Order::Muxed,
    Order::AudioLater(500_000),
    Order::AudioLater(1_500_000),
    Order::VideoLater(1_100_000),
];
const STARTS: [StartPoint; 2] = [StartPoint::Keyframe, StartPoint::MidGop];
const THROUGHPUTS: [Throughput; 3] = [
    Throughput::Instant,
    Throughput::Mbps(50),
    Throughput::Mbps(10),
];
const AFTERS: [After; 2] = [After::Steady, After::Gap];

fn synthetic_rows(full: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for burst in BURSTS {
        for order in ORDERS {
            for start in STARTS {
                for throughput in THROUGHPUTS {
                    for after in AFTERS {
                        let row = Row {
                            start,
                            burst,
                            order,
                            throughput,
                            after,
                            audio: 0,
                            declare: Declare::WhenBoth,
                        };
                        let base = throughput == Throughput::Instant && after == After::Steady;
                        let rate = start == StartPoint::Keyframe
                            && after == After::Steady
                            && matches!(order, Order::Muxed | Order::AudioLater(1_500_000))
                            && matches!(burst, Burst::Media(2_000_000) | Burst::Bytes(_));
                        let gap = start == StartPoint::Keyframe
                            && throughput == Throughput::Instant
                            && after == After::Gap
                            && matches!(order, Order::Muxed | Order::AudioLater(1_500_000))
                            && matches!(burst, Burst::Media(500_000) | Burst::Media(6_000_000));
                        if full || base || rate || gap {
                            rows.push(row);
                        }
                    }
                }
            }
        }
    }
    rows
}

fn needed_copies(raw_us: u64, row: Row) -> usize {
    let burst_us = match row.burst {
        Burst::Media(us) => us,
        Burst::GopTail => raw_us,
        Burst::Bytes(_) => 4 * raw_us,
    };
    let total = 2 * raw_us + burst_us + STEADY_AFTER_US + GAP_LEN_US + 2 * MICROS;
    usize::try_from(total.div_ceil(raw_us)).unwrap() + 1
}

fn matrix(rows: &[Row]) -> Vec<(Row, Plan, Outcome)> {
    let Some(raw) = support::read_tf1_bytes() else {
        return Vec::new();
    };
    let raw_us = {
        let at = schedule(&raw);
        at[at.len() - 1]
    };
    let starve = Config::default().buffer.starve_threshold_us;
    let mut cache: BTreeMap<(usize, String), Stream> = BTreeMap::new();
    let mut results = Vec::new();
    for row in rows {
        let copies = needed_copies(raw_us, *row);
        let key = (copies, format!("{:?}", row.order));
        let stream = cache.entry(key).or_insert_with(|| {
            let (bytes, copy_us) = looped(&raw, copies);
            let at = schedule(&bytes);
            reorder(&bytes, &at, copy_us, row.order)
        });
        let p = plan(stream, *row);
        let mut o = run(&p, *row, starve);
        judge(*row, &mut o);
        results.push((*row, p, o));
    }
    results
}

fn report(results: &[(Row, Plan, Outcome)]) -> Vec<String> {
    let mut failed = Vec::new();
    for (row, p, o) in results {
        let start = o
            .start
            .as_ref()
            .map(|(at, l)| format!("start {at} us leads {l:?}"))
            .unwrap_or_default();
        let min: Vec<(u32, i64)> = o.min_lead.iter().map(|(t, (l, _))| (*t, *l)).collect();
        eprintln!(
            "{} | {row} | burst {} B {} ms | {start} | min lead {min:?} | pauses {:?}",
            if o.failures.is_empty() {
                "ok  "
            } else {
                "FAIL"
            },
            p.burst_bytes,
            p.burst_media_us / 1000,
            o.pauses
        );
        for f in &o.failures {
            failed.push(format!("{row}: {f}"));
        }
    }
    failed
}

fn soak() -> bool {
    std::env::var_os("RUST_BROADCAST_SOAK").is_some()
}

#[test]
fn opening_burst_headline_tf1_field_shape() {
    let results = matrix(&field_rows());
    let failed = report(&results);
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn opening_burst_realistic_server_shape() {
    let rows = realistic_rows();
    let results = matrix(&rows);
    for (row, p, _) in &results {
        if row.start == StartPoint::Keyframe {
            assert!(
                p.burst_bytes <= SERVER_MAX_BURST_PACKETS * PACKET
                    && p.burst_media_us <= SERVER_MAX_BURST_US,
                "{row}: outside the server's burst bounds"
            );
        }
    }
    let failed = report(&results);
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn opening_burst_synthetic_matrix() {
    let rows = synthetic_rows(soak());
    let results = matrix(&rows);
    let failed = report(&results);
    assert!(failed.is_empty(), "{failed:#?}");
}
