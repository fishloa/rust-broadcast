mod support;

use std::collections::BTreeMap;

use broadcast_client::{
    Action, BufferEvent, BufferInput, Config, Event, Hint, Session, Status, TrackId,
};
use buffering_runtime::{
    BufferController, ControllerConfig, Event as RtEvent, StallMemorySnapshot,
};
use support::{read_tf1_bytes, read_tf1_headers};
use transmux::{DemuxEvent, JoinDecision, LiveJoin, StreamingTsDemux, TrackSpec};

const CHUNK: usize = 1316;
const PACKET: usize = 188;
const PACKETS_PER_CHUNK: usize = CHUNK / PACKET;
const STEP_US: u64 = 3_200;
const TICK_US: u64 = 50_000;
const GAP_US: u64 = 3_000_000;
const GAP_AFTER_CHUNK: usize = 1_300;
const TAIL_US: u64 = 6_000_000;
const PCR_HZ: u64 = 27_000_000;
const VIDEO: u32 = 1;
const AUDIO: u32 = 2;
const OTHER_AUDIO: u32 = 3;
const VIDEO_PID: u16 = 0x78;
const AUDIO_PID: u16 = 0x82;
const PAT_PID: u16 = 0;
const PMT_PID: u16 = 0x64;
const SYNC_BYTE: u8 = 0x47;
const AF_PCR_FLAG: u8 = 0x10;
const AF_DISCONTINUITY_FLAG: u8 = 0x80;
const AF_PRESENT: u8 = 0x20;
const URL: &str = "http://example/live.ts";

fn pts27(pts: i64, timescale: u32) -> u64 {
    u64::try_from(i128::from(pts) * i128::from(PCR_HZ) / i128::from(timescale)).unwrap()
}

fn packet_pid(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1f) << 8) | u16::from(p[2])
}

fn packet_has_pcr(p: &[u8]) -> bool {
    p[3] & AF_PRESENT != 0 && p[4] > 0 && p[5] & AF_PCR_FLAG != 0
}

fn drop_pid_from(bytes: &[u8], pid: u16, from_packet: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (i, p) in bytes.chunks(PACKET).enumerate() {
        if i >= from_packet && packet_pid(p) == pid {
            continue;
        }
        out.extend_from_slice(p);
    }
    out
}

fn realtime(n: usize) -> Vec<u64> {
    (0..n).map(|i| i as u64 * STEP_US).collect()
}

fn with_gap(n: usize, after_chunk: usize) -> Vec<u64> {
    (0..n)
        .map(|i| i as u64 * STEP_US + if i > after_chunk { GAP_US } else { 0 })
        .collect()
}

#[derive(Clone, Copy)]
enum Step {
    Tick(u64),
    Data(usize, u64),
}

fn timeline(arrivals: &[u64], tail_us: u64) -> Vec<Step> {
    let end = arrivals.last().copied().unwrap_or(0) + tail_us;
    let mut steps = Vec::new();
    let mut next_tick = 0;
    for (i, &at) in arrivals.iter().enumerate() {
        while next_tick <= at {
            steps.push(Step::Tick(next_tick));
            next_tick += TICK_US;
        }
        steps.push(Step::Data(i, at));
    }
    while next_tick <= end {
        steps.push(Step::Tick(next_tick));
        next_tick += TICK_US;
    }
    steps
}

#[derive(Clone, Copy)]
enum ClockState {
    Idle,
    Playing { base: u64, since: u64 },
    Held(u64),
}

struct PlayClock(ClockState);

impl PlayClock {
    fn new() -> Self {
        Self(ClockState::Idle)
    }

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
    chunk_marks: Vec<usize>,
    ticks: Vec<(u64, Option<u64>)>,
}

impl Host {
    fn open(seed: Option<StallMemorySnapshot>) -> Self {
        let mut session = Session::open_with_memory(URL, Config::default(), 0, seed).unwrap();
        let (status, headers) = read_tf1_headers();
        let conn = match session.poll(0).unwrap() {
            Action::HttpRequest { conn, .. } => conn,
            other => panic!("unexpected first action {other:?}"),
        };
        session.on_response(conn, status, &headers, 0);
        Self {
            session,
            conn,
            clock: PlayClock::new(),
            events: Vec::new(),
            times: Vec::new(),
            chunk_marks: Vec::new(),
            ticks: Vec::new(),
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
        self.chunk_marks.push(self.events.len());
        self.session.on_data(self.conn, bytes, now);
        self.drain(now);
    }

    fn tick(&mut self, now: u64) {
        let p = self.clock.position(now);
        self.ticks.push((now, p));
        self.session.on_playback_position(now, p);
        self.drain(now);
    }

    fn run(&mut self, bytes: &[u8], chunk: usize, arrivals: &[u64], tail_us: u64) {
        let chunks: Vec<&[u8]> = bytes.chunks(chunk).collect();
        for step in timeline(&arrivals[..chunks.len().min(arrivals.len())], tail_us) {
            match step {
                Step::Tick(t) => self.tick(t),
                Step::Data(i, t) => self.data(chunks[i], t),
            }
        }
    }

    fn log(&self) -> &[BufferInput] {
        self.session.buffer_input_log()
    }

    fn buffer_events(&self) -> Vec<BufferEvent> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Buffer(b) => Some(b.clone()),
                _ => None,
            })
            .collect()
    }

    fn samples(&self) -> Vec<(usize, TrackId, transmux::Sample, Option<JoinDecision>)> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                Event::Sample {
                    track,
                    sample,
                    join,
                } => Some((i, *track, sample.clone(), *join)),
                _ => None,
            })
            .collect()
    }

    fn timescales(&self) -> BTreeMap<u32, u32> {
        let mut m = BTreeMap::new();
        for e in &self.events {
            if let Event::Tracks(specs) = e {
                for s in specs {
                    m.insert(s.track_id.value(), s.timescale);
                }
            }
        }
        m
    }

    fn track_ids(&self) -> BTreeMap<u32, TrackId> {
        let mut m = BTreeMap::new();
        for e in &self.events {
            if let Event::Tracks(specs) = e {
                for s in specs {
                    m.insert(s.track_id.value(), s.track_id);
                }
            }
        }
        m
    }

    fn chunk_of_event(&self, event_idx: usize) -> usize {
        self.chunk_marks
            .iter()
            .rposition(|&m| m <= event_idx)
            .unwrap()
    }
}

fn media_reports(log: &[BufferInput]) -> Vec<(usize, u32, u64)> {
    let mut chunk = 0usize;
    let mut out = Vec::new();
    for l in log {
        match l {
            BufferInput::Chunk { .. } => chunk += 1,
            BufferInput::TrackMedia { track, end, .. } => out.push((chunk - 1, *track, *end)),
            _ => {}
        }
    }
    out
}

fn hints(log: &[BufferInput]) -> Vec<(usize, Hint)> {
    log.iter()
        .enumerate()
        .filter_map(|(i, l)| match l {
            BufferInput::Hint { hint, .. } => Some((i, *hint)),
            _ => None,
        })
        .collect()
}

fn chunk_count(log: &[BufferInput]) -> usize {
    log.iter()
        .filter(|l| matches!(l, BufferInput::Chunk { .. }))
        .count()
}

fn without_registration(log: &[BufferInput]) -> Vec<BufferInput> {
    log.iter()
        .filter(|l| !matches!(l, BufferInput::Register(_) | BufferInput::Unregister(_)))
        .cloned()
        .collect()
}

fn first_join_point(h: &Host) -> (usize, i64) {
    h.samples()
        .iter()
        .find_map(|(i, t, s, j)| {
            (t.value() == VIDEO && matches!(j, Some(JoinDecision::JoinPoint(_))))
                .then(|| (*i, s.pts.unwrap()))
        })
        .unwrap()
}

fn join_points(h: &Host) -> Vec<i64> {
    h.samples()
        .iter()
        .filter(|(_, t, _, j)| t.value() == VIDEO && matches!(j, Some(JoinDecision::JoinPoint(_))))
        .map(|(_, _, s, _)| s.pts.unwrap())
        .collect()
}

fn run_realtime(bytes: &[u8]) -> Host {
    let mut h = Host::open(None);
    let n = bytes.len().div_ceil(CHUNK);
    h.run(bytes, CHUNK, &realtime(n), TAIL_US);
    h
}

fn run_gap(bytes: &[u8]) -> Host {
    let mut h = Host::open(None);
    let n = bytes.len().div_ceil(CHUNK);
    h.run(bytes, CHUNK, &with_gap(n, GAP_AFTER_CHUNK), TAIL_US);
    h
}

fn declared_track_count(bytes: &[u8]) -> usize {
    let mut demux = StreamingTsDemux::new();
    demux.feed(bytes);
    let mut n = 0;
    while let Some(ev) = demux.poll_event() {
        if matches!(ev, DemuxEvent::TrackAdded(_)) {
            n += 1;
        }
    }
    n
}

struct Oracle {
    demux: StreamingTsDemux,
    ctl: BufferController,
    specs: BTreeMap<TrackId, TrackSpec>,
    announced: Vec<TrackId>,
    announce_pending: Vec<TrackId>,
    joins: BTreeMap<TrackId, LiveJoin>,
    video: Option<TrackId>,
    audio: Option<TrackId>,
    joined: bool,
    join_27: u64,
    max_pts: i64,
    audio_max_end: u64,
    declared_tracks: usize,
    released: bool,
    held: Vec<(TrackId, transmux::Sample)>,
    clock: PlayClock,
    inputs: Vec<BufferInput>,
    events: Vec<BufferEvent>,
}

impl Oracle {
    fn new(declared_tracks: usize) -> Self {
        Self {
            demux: StreamingTsDemux::new(),
            ctl: BufferController::new(ControllerConfig::default(), None).unwrap(),
            specs: BTreeMap::new(),
            announced: Vec::new(),
            announce_pending: Vec::new(),
            joins: BTreeMap::new(),
            video: None,
            audio: None,
            joined: false,
            join_27: 0,
            max_pts: 0,
            audio_max_end: 0,
            declared_tracks,
            released: false,
            held: Vec::new(),
            clock: PlayClock::new(),
            inputs: Vec::new(),
            events: Vec::new(),
        }
    }

    fn announce(&mut self) {
        for id in std::mem::take(&mut self.announce_pending) {
            if self.announced.contains(&id) {
                continue;
            }
            self.announced.push(id);
            let spec = self.specs[&id].clone();
            if let Some(j) = LiveJoin::for_config(&spec.config) {
                self.joins.insert(id, j);
                if self.video.is_none() {
                    self.video = Some(id);
                    self.ctl.register_track(id.value());
                }
            } else if spec.config.audio_sample_rate().is_some() && self.audio.is_none() {
                self.audio = Some(id);
                self.ctl.register_track(id.value());
            }
        }
    }

    fn drain_events(&mut self, now: u64) {
        while let Some(ev) = self.ctl.poll_event() {
            let mapped = match ev {
                RtEvent::StartPlaying { media_27mhz, .. } => BufferEvent::Start {
                    at_27mhz: media_27mhz,
                },
                RtEvent::Pause {
                    hold_at_27mhz,
                    track,
                    ..
                } => BufferEvent::Pause {
                    hold_at_27mhz,
                    starving_track: track,
                },
                RtEvent::Resume { media_27mhz, .. } => BufferEvent::Resume {
                    at_27mhz: media_27mhz,
                },
                _ => continue,
            };
            self.clock.on_buffer(now, &mapped);
            self.events.push(mapped);
        }
    }

    fn on_sample(
        &mut self,
        track_id: TrackId,
        sample: transmux::Sample,
        zero: &mut Option<(TrackId, u64)>,
        ends: &mut BTreeMap<TrackId, u64>,
    ) {
        self.announce();
        let ts = self.specs[&track_id].timescale;
        let dur = i64::from(sample.duration.unwrap_or(0));
        let pts = sample.pts.unwrap();
        let decision = self.joins.get_mut(&track_id).map(|j| j.push(&sample));
        if Some(track_id) == self.video {
            match decision {
                Some(JoinDecision::JoinPoint(_)) if !self.joined => {
                    self.joined = true;
                    self.join_27 = pts27(pts, ts);
                    self.max_pts = pts;
                    *zero = Some((track_id, pts27(pts, ts)));
                    ends.insert(track_id, pts27(pts + dur, ts));
                }
                Some(JoinDecision::Keep) if self.joined => {
                    self.max_pts = self.max_pts.max(pts);
                    ends.insert(track_id, pts27(self.max_pts + dur, ts));
                }
                _ => {}
            }
        } else if Some(track_id) == self.audio && self.joined {
            self.audio_max_end = self.audio_max_end.max(pts27(pts + dur, ts));
            if self.audio_max_end > self.join_27 {
                ends.insert(track_id, self.audio_max_end);
            }
        }
    }

    fn chunk(&mut self, bytes: &[u8], now: u64) {
        self.demux.feed(bytes);
        let mut pcr = None;
        let mut zero: Option<(TrackId, u64)> = None;
        let mut ends: BTreeMap<TrackId, u64> = BTreeMap::new();
        while let Some(ev) = self.demux.poll_event() {
            match ev {
                DemuxEvent::TrackAdded(s) | DemuxEvent::TrackUpdated(s) => {
                    self.announce_pending.push(s.track_id);
                    self.specs.insert(s.track_id, s);
                }
                DemuxEvent::ClockReference {
                    ticks, clock_hz, ..
                } => {
                    assert_eq!(u64::from(clock_hz), PCR_HZ);
                    pcr = Some(ticks);
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if self.released {
                        self.on_sample(track_id, sample, &mut zero, &mut ends);
                    } else {
                        self.held.push((track_id, sample));
                    }
                }
                _ => {}
            }
        }
        if !self.released && self.specs.len() >= self.declared_tracks {
            self.released = true;
            for (track_id, sample) in std::mem::take(&mut self.held) {
                self.on_sample(track_id, sample, &mut zero, &mut ends);
            }
        }
        self.ctl.on_chunk(now, bytes.len() as u32, pcr);
        self.inputs.push(BufferInput::Chunk {
            now_us: now,
            bytes: bytes.len() as u32,
            pcr,
        });
        if let Some((t, end)) = zero {
            self.ctl.on_track_media(now, t.value(), end);
            self.inputs.push(BufferInput::TrackMedia {
                now_us: now,
                track: t.value(),
                end,
            });
        }
        for (t, end) in ends {
            self.ctl.on_track_media(now, t.value(), end);
            self.inputs.push(BufferInput::TrackMedia {
                now_us: now,
                track: t.value(),
                end,
            });
        }
        self.drain_events(now);
    }

    fn tick(&mut self, now: u64) {
        if self.inputs.is_empty() {
            return;
        }
        let p = self.clock.position(now);
        self.ctl.on_tick(now, p);
        self.inputs.push(BufferInput::Tick {
            now_us: now,
            pts: p,
        });
        self.drain_events(now);
    }
}

#[test]
fn buffer_events_match_bare_controller_oracle() {
    let bytes = read_tf1_bytes();
    let n = bytes.len().div_ceil(CHUNK);
    let arrivals = with_gap(n, GAP_AFTER_CHUNK);
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();

    let mut oracle = Oracle::new(declared_track_count(&bytes));
    for step in timeline(&arrivals, TAIL_US) {
        match step {
            Step::Tick(t) => oracle.tick(t),
            Step::Data(i, t) => oracle.chunk(chunks[i], t),
        }
    }
    let mut host = Host::open(None);
    host.run(&bytes, CHUNK, &arrivals, TAIL_US);

    assert!(
        oracle
            .events
            .iter()
            .any(|e| matches!(e, BufferEvent::Start { .. })),
        "oracle never started: {:?}",
        oracle.events
    );
    assert!(
        oracle
            .events
            .iter()
            .any(|e| matches!(e, BufferEvent::Pause { .. })),
        "oracle never paused: {:?}",
        oracle.events
    );
    assert!(
        oracle
            .events
            .iter()
            .any(|e| matches!(e, BufferEvent::Resume { .. })),
        "oracle never resumed: {:?}",
        oracle.events
    );
    assert_eq!(host.buffer_events(), oracle.events);
    assert_eq!(without_registration(host.log()), oracle.inputs);
}

#[test]
fn start_emitted_once_after_join_point() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let starts: Vec<(usize, u64)> = h
        .events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::Buffer(BufferEvent::Start { at_27mhz }) => Some((i, *at_27mhz)),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 1, "{starts:?}");
    let (join_idx, join_pts) = first_join_point(&h);
    assert!(starts[0].0 > join_idx);
    assert_eq!(starts[0].1, pts27(join_pts, h.timescales()[&VIDEO]));
}

#[test]
fn no_pause_for_steady_delivery() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let last_arrival = (bytes.len().div_ceil(CHUNK) as u64 - 1) * STEP_US;
    let timed: Vec<(u64, BufferEvent)> = h
        .events
        .iter()
        .zip(&h.times)
        .filter_map(|(e, t)| match e {
            Event::Buffer(b) => Some((*t, b.clone())),
            _ => None,
        })
        .collect();
    assert!(
        timed
            .iter()
            .any(|(_, e)| matches!(e, BufferEvent::Start { .. })),
        "{timed:?}"
    );
    assert!(
        !timed
            .iter()
            .any(|(t, e)| *t <= last_arrival && matches!(e, BufferEvent::Pause { .. })),
        "{timed:?}"
    );
    assert!(
        timed
            .iter()
            .any(|(t, e)| *t > last_arrival && matches!(e, BufferEvent::Pause { .. })),
        "the tail after the last chunk must exhaust the lead: {timed:?}"
    );
    let ticks_with_position = h
        .log()
        .iter()
        .filter(|l| matches!(l, BufferInput::Tick { pts: Some(_), .. }))
        .count();
    assert!(ticks_with_position > 10);
}

fn first_pause_track(bytes: &[u8]) -> Option<u32> {
    let h = run_realtime(bytes);
    h.buffer_events().into_iter().find_map(|e| match e {
        BufferEvent::Pause { starving_track, .. } => Some(starving_track),
        _ => None,
    })?
}

#[test]
fn pause_names_starving_track_when_one_track_starves() {
    let bytes = read_tf1_bytes();
    let from = 1_100 * PACKETS_PER_CHUNK;
    let no_audio = drop_pid_from(&bytes, AUDIO_PID, from);
    assert_eq!(first_pause_track(&no_audio), Some(AUDIO));
    let no_video = drop_pid_from(&bytes, VIDEO_PID, from);
    assert_eq!(first_pause_track(&no_video), Some(VIDEO));
}

#[test]
fn resume_after_delivery_returns() {
    let bytes = read_tf1_bytes();
    let h = run_gap(&bytes);
    let timed: Vec<(u64, BufferEvent)> = h
        .events
        .iter()
        .zip(&h.times)
        .filter_map(|(e, t)| match e {
            Event::Buffer(b) => Some((*t, b.clone())),
            _ => None,
        })
        .collect();
    let kinds: Vec<&str> = timed.iter().map(|(_, e)| e.name()).collect();
    assert_eq!(kinds, ["start", "pause", "resume", "pause"], "{timed:?}");
    let last_arrival = (bytes.len().div_ceil(CHUNK) as u64 - 1) * STEP_US + GAP_US;
    assert!(timed[3].0 > last_arrival);
    let gap_end = (GAP_AFTER_CHUNK as u64 + 1) * STEP_US + GAP_US;
    assert!(timed[1].0 < gap_end, "pause must happen inside the gap");
    assert!(timed[2].0 >= gap_end, "resume must wait for data");
    let (BufferEvent::Pause { hold_at_27mhz, .. }, BufferEvent::Resume { at_27mhz, .. }) =
        (&timed[1].1, &timed[2].1)
    else {
        panic!("unexpected {timed:?}");
    };
    assert_eq!(hold_at_27mhz, at_27mhz);
}

#[test]
fn first_media_report_is_zero_length_video_join_point() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let reports = media_reports(h.log());
    let (_, join_pts) = first_join_point(&h);
    let join_27 = pts27(join_pts, h.timescales()[&VIDEO]);
    assert_eq!(reports[0].1, VIDEO);
    assert_eq!(reports[0].2, join_27);
    let next_video = reports.iter().skip(1).find(|r| r.1 == VIDEO).unwrap();
    assert!(next_video.2 > join_27);
}

#[test]
fn audio_withheld_until_video_join_point() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let (join_idx, _) = first_join_point(&h);
    let audio_before_join = h
        .samples()
        .iter()
        .filter(|(i, t, _, _)| *i < join_idx && t.value() == AUDIO)
        .count();
    assert!(
        audio_before_join > 0,
        "fixture must carry audio ahead of the video join point"
    );
    let reports = media_reports(h.log());
    let first_video = reports.iter().position(|r| r.1 == VIDEO).unwrap();
    let first_audio = reports.iter().position(|r| r.1 == AUDIO).unwrap();
    assert!(first_video < first_audio);
}

fn mid_gop_stream(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in bytes.chunks(PACKET).take(PACKETS_PER_CHUNK * 4) {
        let pid = packet_pid(p);
        if pid == PAT_PID || pid == PMT_PID {
            out.extend_from_slice(p);
        }
    }
    let start = bytes
        .chunks(PACKET)
        .enumerate()
        .skip(PACKETS_PER_CHUNK * 300)
        .find(|(_, p)| packet_pid(p) == VIDEO_PID && p[1] & 0x40 != 0 && p[3] & AF_PRESENT == 0)
        .map(|(i, _)| i)
        .unwrap();
    out.extend_from_slice(&bytes[start * PACKET..]);
    out
}

#[test]
fn video_before_join_point_not_reported() {
    let bytes = mid_gop_stream(&read_tf1_bytes());
    let h = run_realtime(&bytes);
    let video: Vec<_> = h
        .samples()
        .into_iter()
        .filter(|(_, t, _, _)| t.value() == VIDEO)
        .collect();
    let dropped_before_join = video
        .iter()
        .take_while(|(_, _, _, j)| !matches!(j, Some(JoinDecision::JoinPoint(_))))
        .filter(|(_, _, _, j)| matches!(j, Some(JoinDecision::Drop(_))))
        .count();
    assert!(
        dropped_before_join > 0,
        "stream must start mid-GOP so the joiner drops samples"
    );
    let (_, join_pts) = first_join_point(&h);
    let ts = h.timescales()[&VIDEO];
    let join_27 = pts27(join_pts, ts);
    let min_dropped_end = video
        .iter()
        .filter(|(_, _, _, j)| matches!(j, Some(JoinDecision::Drop(_))))
        .map(|(_, _, s, _)| pts27(s.pts.unwrap(), ts))
        .min()
        .unwrap();
    let reports = media_reports(h.log());
    let video_reports: Vec<u64> = reports
        .iter()
        .filter(|r| r.1 == VIDEO)
        .map(|r| r.2)
        .collect();
    assert_eq!(video_reports[0], join_27);
    assert!(video_reports.iter().all(|&e| e >= join_27));
    assert!(min_dropped_end < join_27 || video_reports.iter().all(|&e| e >= join_27));
}

fn expected_pcrs(bytes: &[u8], chunk: usize) -> Vec<(Option<u64>, usize)> {
    let mut demux = StreamingTsDemux::new();
    let mut expected = Vec::new();
    for ch in bytes.chunks(chunk) {
        demux.feed(ch);
        let mut last = None;
        let mut count = 0;
        while let Some(ev) = demux.poll_event() {
            if let DemuxEvent::ClockReference { ticks, .. } = ev {
                last = Some(ticks);
                count += 1;
            }
        }
        expected.push((last, count));
    }
    expected
}

fn fed_pcrs(bytes: &[u8], chunk: usize) -> Vec<Option<u64>> {
    let mut h = Host::open(None);
    for (i, ch) in bytes.chunks(chunk).enumerate() {
        h.data(ch, i as u64 * 10_000);
    }
    h.log()
        .iter()
        .filter_map(|l| match l {
            BufferInput::Chunk { pcr, .. } => Some(*pcr),
            _ => None,
        })
        .collect()
}

#[test]
fn pcr_taken_once_per_chunk_from_last_clock_reference() {
    const BIG: usize = 65_536;
    let bytes = read_tf1_bytes();

    let big = expected_pcrs(&bytes, BIG);
    assert!(
        big.iter().any(|(_, c)| *c >= 2),
        "need a chunk carrying several PCRs"
    );
    let want: Vec<Option<u64>> = big.iter().map(|(p, _)| *p).collect();
    assert_eq!(fed_pcrs(&bytes, BIG), want);

    let small = expected_pcrs(&bytes, CHUNK);
    let want_small: Vec<Option<u64>> = small.iter().map(|(p, _)| *p).collect();
    assert!(
        want_small
            .windows(2)
            .any(|w| w[0].is_some() && w[1].is_none()),
        "a chunk without a PCR must not inherit the previous one"
    );
    assert_eq!(fed_pcrs(&bytes, CHUNK), want_small);
}

fn reconnect(h: &mut Host, now: u64) {
    h.session.on_closed(h.conn, None, now);
    let (status, headers) = read_tf1_headers();
    let mut new_conn = None;
    while let Some(a) = h.session.poll(now) {
        if let Action::HttpRequest { conn, .. } = a {
            new_conn = Some(conn);
        }
    }
    h.conn = new_conn.unwrap();
    h.session.on_response(h.conn, status, &headers, now);
    h.drain(now);
}

#[test]
fn reconnect_sends_reconnected_hint_before_first_chunk_and_keeps_memory() {
    let bytes = read_tf1_bytes();
    let n = bytes.len().div_ceil(CHUNK);
    let mut h = Host::open(None);
    let split = 2_000;
    let arrivals = with_gap(n, GAP_AFTER_CHUNK);
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    for step in timeline(&arrivals[..split], 0) {
        match step {
            Step::Tick(t) => h.tick(t),
            Step::Data(i, t) => h.data(chunks[i], t),
        }
    }
    assert!(
        !hints(h.log())
            .iter()
            .any(|(_, hint)| *hint == Hint::Reconnected)
    );
    let before = h.session.stall_memory();
    assert_ne!(before.peak_us, 0);
    let now = arrivals[split - 1] + 1_000;
    reconnect(&mut h, now);

    let log = h.log();
    let hs = hints(log);
    assert_eq!(
        hs.iter().map(|(_, hint)| *hint).collect::<Vec<_>>(),
        [Hint::SourceEnded, Hint::Reconnected]
    );
    assert_eq!(
        hs[1].0,
        log.len() - 1,
        "Reconnected must be the latest input"
    );
    assert_eq!(h.session.stall_memory().peak_us, before.peak_us);

    let chunks_before = chunk_count(h.log());
    h.data(chunks[split], now + 10_000);
    let log = h.log();
    assert_eq!(chunk_count(log), chunks_before + 1);
    assert!(matches!(log[hs[1].0 + 1], BufferInput::Chunk { .. }));
    assert!(!h.log().iter().any(|l| matches!(
        l,
        BufferInput::Hint {
            hint: Hint::PcrDiscontinuity,
            ..
        }
    )));
}

#[test]
fn source_ended_and_error_hints() {
    let bytes = read_tf1_bytes();
    let mut ended = Host::open(None);
    ended.data(&bytes[..CHUNK * 4], 1_000);
    ended.session.on_closed(ended.conn, None, 2_000);
    assert_eq!(
        hints(ended.log())
            .into_iter()
            .map(|(_, h)| h)
            .collect::<Vec<_>>(),
        [Hint::SourceEnded]
    );

    let mut failed = Host::open(None);
    failed.data(&bytes[..CHUNK * 4], 1_000);
    failed
        .session
        .on_closed(failed.conn, Some("connection reset".to_string()), 2_000);
    assert_eq!(
        hints(failed.log())
            .into_iter()
            .map(|(_, h)| h)
            .collect::<Vec<_>>(),
        [Hint::SourceError]
    );

    let mut forbidden = Session::open(URL, Config::default(), 0).unwrap();
    let conn = match forbidden.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    forbidden.on_response(conn, 403, &[], 0);
    assert_eq!(
        hints(forbidden.buffer_input_log())
            .into_iter()
            .map(|(_, h)| h)
            .collect::<Vec<_>>(),
        [Hint::SourceError]
    );
}

#[test]
fn demux_discontinuity_sends_pcr_discontinuity_and_rejoins() {
    let bytes = read_tf1_bytes();
    let clean = run_realtime(&bytes);
    assert!(
        !hints(clean.log())
            .iter()
            .any(|(_, h)| *h == Hint::PcrDiscontinuity),
        "unmodified capture must not signal a discontinuity"
    );
    assert_eq!(join_points(&clean).len(), 1);

    let mut modified = bytes.clone();
    let target = modified
        .chunks(PACKET)
        .enumerate()
        .skip(1_500 * PACKETS_PER_CHUNK)
        .find(|(_, p)| packet_pid(p) == VIDEO_PID && packet_has_pcr(p))
        .map(|(i, _)| i)
        .unwrap();
    modified[target * PACKET + 5] |= AF_DISCONTINUITY_FLAG;
    assert_eq!(modified[target * PACKET], SYNC_BYTE);

    let h = run_realtime(&modified);
    let hs = hints(h.log());
    let disc: Vec<usize> = hs
        .iter()
        .filter(|(_, hint)| *hint == Hint::PcrDiscontinuity)
        .map(|(i, _)| *i)
        .collect();
    assert_eq!(disc.len(), 1, "{hs:?}");
    let hint_chunk = chunk_count(&h.log()[..disc[0]]);
    assert_eq!(hint_chunk, target / PACKETS_PER_CHUNK);

    let joins = join_points(&h);
    assert_eq!(joins.len(), 2, "joiner must re-arm: {joins:?}");
    let ts = h.timescales()[&VIDEO];
    let video_reports: Vec<(usize, u64)> = h
        .log()
        .iter()
        .enumerate()
        .filter_map(|(i, l)| match l {
            BufferInput::TrackMedia { track, end, .. } if *track == VIDEO => Some((i, *end)),
            _ => None,
        })
        .collect();
    let after = video_reports.iter().find(|(i, _)| *i > disc[0]).unwrap();
    assert_eq!(after.1, pts27(joins[1], ts));
    let drops_after = h
        .samples()
        .iter()
        .filter(|(_, t, s, j)| {
            t.value() == VIDEO
                && s.pts.is_some_and(|p| p > joins[0])
                && matches!(j, Some(JoinDecision::Drop(_)))
        })
        .count();
    assert!(drops_after > 0, "samples before the new join must drop");
}

#[test]
fn set_rendered_tracks_registers_and_unregisters_difference() {
    let bytes = read_tf1_bytes();
    let mut h = Host::open(None);
    let n = 1_400;
    h.run(&bytes[..n * CHUNK], CHUNK, &realtime(n), 0);
    let ids = h.track_ids();
    let registers = |h: &Host| -> Vec<BufferInput> {
        h.log()
            .iter()
            .filter(|l| matches!(l, BufferInput::Register(_) | BufferInput::Unregister(_)))
            .cloned()
            .collect()
    };
    assert_eq!(
        registers(&h),
        [BufferInput::Register(VIDEO), BufferInput::Register(AUDIO)]
    );
    let base = h.log().len();

    h.session
        .set_rendered_tracks(&[ids[&VIDEO], ids[&OTHER_AUDIO]]);
    assert_eq!(
        h.log()[base..],
        [
            BufferInput::Unregister(AUDIO),
            BufferInput::Register(OTHER_AUDIO)
        ]
    );
    let base = h.log().len();
    h.session
        .set_rendered_tracks(&[ids[&VIDEO], ids[&OTHER_AUDIO]]);
    assert_eq!(h.log().len(), base);

    h.session
        .set_rendered_tracks(&[ids[&VIDEO], ids[&AUDIO], ids[&OTHER_AUDIO]]);
    assert_eq!(h.log()[base..], [BufferInput::Register(AUDIO)]);
}

#[test]
fn playback_position_none_until_started_then_some() {
    let bytes = read_tf1_bytes();
    let mut h = Host::open(None);
    h.tick(0);
    assert!(h.log().is_empty(), "no controller exists before data");

    let n = bytes.len().div_ceil(CHUNK);
    h.run(&bytes, CHUNK, &realtime(n), TAIL_US);
    let fed: Vec<(u64, Option<u64>)> = h
        .log()
        .iter()
        .filter_map(|l| match l {
            BufferInput::Tick { now_us, pts } => Some((*now_us, *pts)),
            _ => None,
        })
        .collect();
    assert_eq!(h.ticks[..2], [(0, None), (0, None)]);
    let given: Vec<(u64, Option<u64>)> = h.ticks.iter().skip(2).copied().collect();
    assert_eq!(fed, given);

    let start_at = h
        .events
        .iter()
        .zip(&h.times)
        .find_map(|(e, t)| matches!(e, Event::Buffer(BufferEvent::Start { .. })).then_some(*t))
        .unwrap();
    let before: Vec<_> = fed.iter().filter(|(t, _)| *t < start_at).collect();
    let after: Vec<_> = fed.iter().filter(|(t, _)| *t > start_at).collect();
    assert!(!before.is_empty() && !after.is_empty());
    assert!(before.iter().all(|(_, p)| p.is_none()));
    assert!(after.iter().all(|(_, p)| p.is_some()));
}

#[test]
fn stall_memory_roundtrip_seeds_next_session() {
    let bytes = read_tf1_bytes();
    let first = run_gap(&bytes);
    let memory = first.session.stall_memory();
    assert_ne!(memory.peak_us, 0);

    let mut seeded = Host::open(Some(memory));
    assert_eq!(seeded.session.stall_memory().peak_us, memory.peak_us);
    seeded.data(&bytes[..CHUNK], 0);
    assert_eq!(seeded.session.stall_memory().peak_us, memory.peak_us);

    let mut fresh = Host::open(None);
    fresh.data(&bytes[..CHUNK], 0);
    assert_eq!(fresh.session.stall_memory().peak_us, 0);
}

#[test]
fn vod_buffer_events_come_from_the_playout_gate_not_the_controller() {
    let bytes = read_tf1_bytes();
    let (status, mut headers) = read_tf1_headers();
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    let mut session = Session::open(URL, Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(conn, status, &headers, 0);
    let mut events = Vec::new();
    for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
        let now = i as u64 * STEP_US;
        session.on_data(conn, chunk, now);
        session.on_playback_position(now, Some(now * 27));
        session.hint(Hint::SourceError);
        while let Some(e) = session.next_event() {
            events.push(e);
        }
    }
    session.on_closed(conn, None, 10_000_000);
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(events.iter().any(|e| matches!(e, Event::Finite { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::Sample { .. })));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Ended)))
    );
    let buffer: Vec<&BufferEvent> = events
        .iter()
        .filter_map(|e| match e {
            Event::Buffer(b) => Some(b),
            _ => None,
        })
        .collect();
    assert!(matches!(buffer.first(), Some(BufferEvent::Start { .. })));
    assert_eq!(
        buffer
            .iter()
            .filter(|b| matches!(b, BufferEvent::Start { .. }))
            .count(),
        1
    );
    assert!(session.buffer_input_log().is_empty());
}

#[test]
fn on_data_zero_length_is_noop_for_controller() {
    let bytes = read_tf1_bytes();
    let mut h = Host::open(None);
    h.data(&[], 0);
    assert!(h.log().is_empty());
    h.data(&bytes[..CHUNK], 10);
    let after_first = h.log().len();
    assert_eq!(chunk_count(h.log()), 1);
    h.data(&[], 20);
    assert_eq!(h.log().len(), after_first);
    h.data(&bytes[CHUNK..CHUNK * 2], 30);
    assert_eq!(chunk_count(h.log()), 2);
}

struct ChunkVideo {
    expected_end: Option<u64>,
    last_decoded_end: Option<u64>,
}

fn video_expectations(h: &Host) -> Vec<ChunkVideo> {
    let ts = h.timescales()[&VIDEO];
    let mut per_chunk: Vec<ChunkVideo> = (0..h.chunk_marks.len())
        .map(|_| ChunkVideo {
            expected_end: None,
            last_decoded_end: None,
        })
        .collect();
    let mut max_pts: Option<i64> = None;
    for (i, t, s, j) in h.samples() {
        if t.value() != VIDEO {
            continue;
        }
        let pts = s.pts.unwrap();
        let dur = i64::from(s.duration.unwrap());
        let c = h.chunk_of_event(i);
        match j {
            Some(JoinDecision::JoinPoint(_)) if max_pts.is_none() => max_pts = Some(pts),
            Some(JoinDecision::Keep) if max_pts.is_some() => {
                max_pts = max_pts.map(|m| m.max(pts));
            }
            _ => continue,
        }
        per_chunk[c].expected_end = Some(pts27(max_pts.unwrap() + dur, ts));
        per_chunk[c].last_decoded_end = Some(pts27(pts + dur, ts));
    }
    per_chunk
}

#[test]
fn video_end_uses_max_pts_not_last_decoded() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let expect = video_expectations(&h);
    let mut reported: BTreeMap<usize, u64> = BTreeMap::new();
    let (_, join_pts) = first_join_point(&h);
    let join_27 = pts27(join_pts, h.timescales()[&VIDEO]);
    let mut seen_zero = false;
    for (c, track, end) in media_reports(h.log()) {
        if track != VIDEO {
            continue;
        }
        if !seen_zero {
            seen_zero = true;
            assert_eq!(end, join_27);
            continue;
        }
        reported.insert(c, end);
    }
    let mut differing = 0;
    for (c, e) in expect.iter().enumerate() {
        assert_eq!(reported.get(&c).copied(), e.expected_end, "chunk {c}");
        if e.expected_end.is_some() && e.expected_end != e.last_decoded_end {
            differing += 1;
        }
    }
    assert!(
        differing > 0,
        "fixture must contain reordered video so max-PTS differs from the last decoded sample"
    );
}

#[test]
fn audio_end_only_after_join_pts() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let ts_video = h.timescales()[&VIDEO];
    let ts_audio = h.timescales()[&AUDIO];
    let (join_idx, join_pts) = first_join_point(&h);
    let join_27 = pts27(join_pts, ts_video);
    let behind = h
        .samples()
        .iter()
        .filter(|(i, t, s, _)| {
            *i > join_idx
                && t.value() == AUDIO
                && pts27(s.pts.unwrap() + i64::from(s.duration.unwrap()), ts_audio) <= join_27
        })
        .count();
    assert!(
        behind > 0,
        "fixture must carry audio samples ending at or before the join point"
    );
    let audio_reports: Vec<u64> = media_reports(h.log())
        .into_iter()
        .filter(|r| r.1 == AUDIO)
        .map(|r| r.2)
        .collect();
    assert!(!audio_reports.is_empty());
    assert!(audio_reports.iter().all(|&e| e > join_27));
    let expected_first = h
        .samples()
        .iter()
        .filter(|(i, t, _, _)| *i > join_idx && t.value() == AUDIO)
        .map(|(_, _, s, _)| pts27(s.pts.unwrap() + i64::from(s.duration.unwrap()), ts_audio))
        .find(|&e| e > join_27)
        .unwrap();
    assert!(audio_reports[0] >= expected_first);
}

#[test]
fn audio_end_forgotten_when_rendered_audio_changes() {
    let bytes = read_tf1_bytes();
    let mut h = Host::open(None);
    let n = bytes.len().div_ceil(CHUNK);
    let arrivals = realtime(n);
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    let switch_at = 1_500;
    let mut switched_log_len = 0;
    let mut switched_event_idx = 0;
    for step in timeline(&arrivals, 0) {
        match step {
            Step::Tick(t) => h.tick(t),
            Step::Data(i, t) => {
                if i == switch_at {
                    let ids = h.track_ids();
                    h.session
                        .set_rendered_tracks(&[ids[&VIDEO], ids[&OTHER_AUDIO]]);
                    switched_log_len = h.log().len();
                    switched_event_idx = h.events.len();
                }
                h.data(chunks[i], t);
            }
        }
    }
    let ts = h.timescales()[&OTHER_AUDIO];
    let old_audio_after: Vec<u64> = media_reports(&h.log()[switched_log_len..])
        .into_iter()
        .filter(|r| r.1 == AUDIO)
        .map(|r| r.2)
        .collect();
    assert!(old_audio_after.is_empty());
    let mut reports = Vec::new();
    let mut chunk = 0usize;
    for l in &h.log()[switched_log_len..] {
        match l {
            BufferInput::Chunk { .. } => chunk += 1,
            BufferInput::TrackMedia { track, end, .. } if *track == OTHER_AUDIO => {
                reports.push((chunk - 1 + switch_at, *end));
            }
            _ => {}
        }
    }
    assert!(!reports.is_empty());
    let mut max_end = 0u64;
    let mut expected: BTreeMap<usize, u64> = BTreeMap::new();
    for (i, t, s, _) in h.samples() {
        if i < switched_event_idx || t.value() != OTHER_AUDIO {
            continue;
        }
        max_end = max_end.max(pts27(s.pts.unwrap() + i64::from(s.duration.unwrap()), ts));
        expected.insert(h.chunk_of_event(i), max_end);
    }
    for (c, end) in reports {
        assert_eq!(expected[&c], end, "chunk {c}");
    }
}

#[test]
fn non_rendered_audio_never_counts() {
    let bytes = read_tf1_bytes();
    let h = run_realtime(&bytes);
    let other_audio_samples = h
        .samples()
        .iter()
        .filter(|(_, t, _, _)| t.value() == OTHER_AUDIO)
        .count();
    assert!(other_audio_samples > 0);
    let tracks: Vec<u32> = media_reports(h.log()).into_iter().map(|r| r.1).collect();
    assert!(
        tracks.iter().all(|t| *t == VIDEO || *t == AUDIO),
        "{tracks:?}"
    );
}

fn rejoin_run(rejoin_at: usize) -> (Host, usize) {
    let bytes = read_tf1_bytes();
    let n = bytes.len().div_ceil(CHUNK);
    let arrivals = realtime(n);
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    let mut h = Host::open(None);
    let mut mark = 0;
    for step in timeline(&arrivals, TAIL_US) {
        match step {
            Step::Tick(t) => h.tick(t),
            Step::Data(i, t) => {
                if i == rejoin_at {
                    mark = h.log().len();
                    h.session.rejoin();
                }
                h.data(chunks[i], t);
            }
        }
    }
    (h, mark)
}

#[test]
fn rejoin_rearms_join_without_pcr_discontinuity_hint_or_controller_reset() {
    let (h, mark) = rejoin_run(1_200);
    let after = &h.log()[mark..];
    assert!(!after.iter().any(|l| matches!(
        l,
        BufferInput::Hint { .. } | BufferInput::Register(_) | BufferInput::Unregister(_)
    )));
    let joins = join_points(&h);
    assert_eq!(joins.len(), 2, "{joins:?}");
    let ts = h.timescales()[&VIDEO];
    let first_after = media_reports(after)
        .into_iter()
        .find(|r| r.1 == VIDEO)
        .unwrap();
    assert_eq!(first_after.2, pts27(joins[1], ts));
    let starts = h
        .buffer_events()
        .iter()
        .filter(|e| matches!(e, BufferEvent::Start { .. }))
        .count();
    assert_eq!(starts, 1, "controller must not be reset");
    assert!(h.samples().iter().any(|(_, t, s, j)| t.value() == VIDEO
        && s.pts.is_some_and(|p| p > joins[0])
        && matches!(j, Some(JoinDecision::Drop(_)))));
}

#[test]
fn rejoin_clears_pending_ends() {
    let bytes = read_tf1_bytes();
    let mut h = Host::open(None);
    let head = &bytes[..CHUNK * 1_200];
    for (i, ch) in head.chunks(CHUNK).enumerate() {
        h.data(ch, i as u64 * STEP_US);
    }
    let samples_before_close = h.samples().len();
    h.session.on_closed(h.conn, None, 1_300 * STEP_US);
    h.drain(1_300 * STEP_US);
    assert!(
        h.samples().len() > samples_before_close,
        "closing must flush demuxer samples that stay pending"
    );
    h.session.rejoin();
    let (status, headers) = read_tf1_headers();
    let mut conn = None;
    while let Some(a) = h.session.poll(1_300 * STEP_US) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    h.conn = conn.unwrap();
    h.session
        .on_response(h.conn, status, &headers, 1_300 * STEP_US);
    h.data(&bytes[..CHUNK], 1_400 * STEP_US);
    let tail = media_reports(h.log())
        .into_iter()
        .filter(|r| r.0 >= 1_200)
        .count();
    assert_eq!(tail, 0, "stale ends leaked into the next connection");
}
