mod support;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use broadcast_client::{
    AUDIO_TRACK_ID_OFFSET, Action, BufferEvent, Config, Event, PauseReason, PlayoutConfig, Session,
    SourceKind, TrackId,
};
use support::hls_host::*;
use support::renditions::{self, AUDIO_DIR, AUDIO_PLAYLIST_URL, MASTER_URL};
use support::{read_tf1_bytes, read_tf1_headers};
use transmux::JoinDecision;
use transmux::ts_hls::StreamingTsHlsSegmenter;

const TICKS_PER_SEC: u64 = 27_000_000;
const TICKS_PER_MS: u64 = 27_000;
const VIDEO: u32 = 1;
const AUDIO: u32 = 2;
const FIRST_VIDEO_PTS_TICKS: u64 = 39_960_000;
const FINITE_URL: &str = "http://example/finite.ts";
const LIVE_DIR: &str = "http://origin.test/live";
const LIVE_PLAYLIST_URL: &str = "http://origin.test/live/index.m3u8";
const LOW_PLAYLIST_URL: &str = "http://origin.test/hls/low/index.m3u8";
const HIGH_PLAYLIST_URL: &str = "http://origin.test/hls/high/index.m3u8";
const SWITCH_MASTER_URL: &str = "http://origin.test/hls/master.m3u8";
const LONG_REPEATS: i64 = 4;
const LONG_TARGET_SECS: u32 = 2;
const LONG_WINDOW: usize = 64;
const SPAN_SECS: i64 = 3;
const SEG_SECS: u64 = 2;
const MAX_WAITS: usize = 64;
const SEG_URIS: [&str; 6] = [
    "seg0.ts", "seg1.ts", "seg2.ts", "seg3.ts", "seg4.ts", "seg5.ts",
];

fn tid(v: u32) -> TrackId {
    TrackId::new(v)
}

fn to27(pts: i64, timescale: u32) -> u64 {
    u64::try_from(i128::from(pts) * i128::from(TICKS_PER_SEC) / i128::from(timescale)).unwrap()
}

fn cfg(playout: PlayoutConfig) -> Config {
    {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::Hls;
        new_config.playout = playout;
        new_config
    }
}

fn vod_lead(ms: u64) -> PlayoutConfig {
    PlayoutConfig {
        vod_start_lead_us: ms * MS,
        ..PlayoutConfig::default()
    }
}

struct Drv {
    session: Session,
    now: u64,
    chunk: usize,
    pending: Vec<(u32, String)>,
    wakes: BTreeSet<u64>,
    events: Vec<Event>,
    bodies: HashMap<String, Vec<u8>>,
}

impl Drv {
    fn new(url: &str, config: Config, bodies: HashMap<String, Vec<u8>>) -> Self {
        let session = Session::open(url, config, 0).expect("open");
        let mut d = Self {
            session,
            now: 0,
            chunk: 65_536,
            pending: Vec::new(),
            wakes: BTreeSet::new(),
            events: Vec::new(),
            bodies,
        };
        d.pump();
        d
    }

    fn pump(&mut self) {
        while let Some(a) = self.session.poll(self.now) {
            match a {
                Action::HttpRequest { conn, url, .. } => self.pending.push((conn, url)),
                Action::Close { conn } => self.pending.retain(|p| p.0 != conn),
                Action::WakeAt(t) if t > self.now => {
                    self.wakes.insert(t);
                }
                _ => {}
            }
        }
        while let Some(e) = self.session.next_event() {
            self.events.push(e);
        }
    }

    fn respond(&mut self, conn: u32, body: &[u8]) {
        self.session.on_response(conn, 200, &[], self.now);
        for c in body.chunks(self.chunk.max(1)) {
            self.session.on_data(conn, c, self.now);
        }
        self.session.on_closed(conn, None, self.now);
        self.pump();
    }

    fn is_pending(&self, url: &str) -> bool {
        self.pending.iter().any(|p| p.1 == url)
    }

    fn serve(&mut self, url: &str) {
        let i = self
            .pending
            .iter()
            .position(|p| p.1 == url)
            .unwrap_or_else(|| panic!("{url} is not pending: {:?}", self.pending));
        let (conn, _) = self.pending.remove(i);
        let body = self
            .bodies
            .get(url)
            .unwrap_or_else(|| panic!("no body for {url}"))
            .clone();
        self.respond(conn, &body);
    }

    fn serve_text(&mut self, url: &str, text: &str) {
        let i = self
            .pending
            .iter()
            .position(|p| p.1 == url)
            .unwrap_or_else(|| panic!("{url} is not pending: {:?}", self.pending));
        let (conn, _) = self.pending.remove(i);
        self.respond(conn, text.as_bytes());
    }

    fn serve_all_except(&mut self, withheld: &[&str]) {
        loop {
            self.pump();
            let Some(i) = self
                .pending
                .iter()
                .position(|p| !withheld.contains(&p.1.as_str()))
            else {
                return;
            };
            let (conn, url) = self.pending.remove(i);
            let body = self
                .bodies
                .get(&url)
                .unwrap_or_else(|| panic!("unexpected request {url}"))
                .clone();
            self.respond(conn, &body);
        }
    }

    fn wait_for(&mut self, url: &str) {
        for _ in 0..MAX_WAITS {
            self.pump();
            if self.is_pending(url) {
                return;
            }
            let next = self
                .wakes
                .range((self.now + 1)..)
                .next()
                .copied()
                .unwrap_or_else(|| panic!("nothing to wait for while {url} is not pending"));
            self.wakes.remove(&next);
            self.now = next;
        }
        panic!("{url} never became pending");
    }

    fn serve_segment(&mut self, n: usize) {
        let url = format!("{LIVE_DIR}/{}", SEG_URIS[n]);
        self.wait_for(&url);
        self.serve(&url);
    }

    fn pos(&mut self, pts: Option<u64>) {
        self.session.on_playback_position(self.now, pts);
        self.pump();
    }
}

fn nb(events: &[Event]) -> Vec<Event> {
    events
        .iter()
        .filter(|e| !matches!(e, Event::Buffer(_)))
        .cloned()
        .collect()
}

fn marks(events: &[Event]) -> Vec<(usize, BufferEvent)> {
    let mut seen = 0;
    let mut out = Vec::new();
    for e in events {
        match e {
            Event::Buffer(b) => out.push((seen, b.clone())),
            _ => seen += 1,
        }
    }
    out
}

fn bufs(events: &[Event]) -> Vec<BufferEvent> {
    marks(events).into_iter().map(|(_, b)| b).collect()
}

fn names(events: &[Event]) -> Vec<&'static str> {
    bufs(events).iter().map(BufferEvent::name).collect()
}

fn ended_index(nb_events: &[Event]) -> usize {
    nb_events.iter().position(is_ended).expect("Ended event")
}

#[derive(Debug, Clone)]
struct Point {
    after: usize,
    origin: u64,
    min_end: u64,
    max_end: u64,
    limiting: TrackId,
}

impl Point {
    fn lead(&self) -> u64 {
        self.min_end.saturating_sub(self.origin)
    }
}

fn lead_points(nb_events: &[Event], rendered: &[u32]) -> Vec<Point> {
    lead_points_from(nb_events, rendered, 0)
}

fn lead_points_from(nb_events: &[Event], rendered: &[u32], from: usize) -> Vec<Point> {
    let rendered: BTreeSet<TrackId> = rendered.iter().map(|v| tid(*v)).collect();
    let mut kind: BTreeMap<TrackId, (bool, u32)> = BTreeMap::new();
    let mut first: BTreeMap<TrackId, u64> = BTreeMap::new();
    let mut max_pts: BTreeMap<TrackId, i64> = BTreeMap::new();
    let mut end: BTreeMap<TrackId, u64> = BTreeMap::new();
    let mut out = Vec::new();
    for (i, e) in nb_events.iter().enumerate() {
        match e {
            Event::Tracks(specs) => {
                for s in specs {
                    kind.insert(
                        s.track_id,
                        (s.config.audio_sample_rate().is_none(), s.timescale),
                    );
                }
            }
            Event::Sample {
                track,
                sample,
                join,
            } if rendered.contains(track) && i >= from => {
                if matches!(join, Some(JoinDecision::Drop(_))) {
                    continue;
                }
                let (is_video, ts) = kind[track];
                let pts = sample.pts.unwrap();
                let dur = i64::from(sample.duration.unwrap_or(0));
                first.entry(*track).or_insert_with(|| to27(pts, ts));
                let m = max_pts.entry(*track).or_insert(pts);
                *m = (*m).max(pts);
                let new_end = if is_video {
                    to27(*m + dur, ts)
                } else {
                    to27(pts + dur, ts)
                };
                let slot = end.entry(*track).or_insert(0);
                *slot = if is_video {
                    new_end
                } else {
                    (*slot).max(new_end)
                };
                if rendered.iter().all(|t| end.contains_key(t)) {
                    let video_first = rendered
                        .iter()
                        .filter(|t| kind[t].0)
                        .map(|t| first[t])
                        .min();
                    let origin = video_first
                        .unwrap_or_else(|| rendered.iter().map(|t| first[t]).min().unwrap());
                    let (limiting, min_end) = rendered
                        .iter()
                        .map(|t| (*t, end[t]))
                        .min_by_key(|(_, e)| *e)
                        .unwrap();
                    let max_end = rendered.iter().map(|t| end[t]).max().unwrap();
                    out.push(Point {
                        after: i + 1,
                        origin,
                        min_end,
                        max_end,
                        limiting,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn first_point_at_lead(points: &[Point], lead_ticks: u64) -> &Point {
    points
        .iter()
        .find(|p| p.lead() >= lead_ticks)
        .expect("lead is reached")
}

fn first_video_pts_after(nb_events: &[Event], from: usize) -> u64 {
    let mut ts = BTreeMap::new();
    for e in &nb_events[..from] {
        if let Event::Tracks(specs) = e {
            for s in specs {
                ts.insert(s.track_id, s.timescale);
            }
        }
    }
    for e in &nb_events[from..] {
        if let Event::Tracks(specs) = e {
            for s in specs {
                ts.insert(s.track_id, s.timescale);
            }
        }
        if let Event::Sample {
            track,
            sample,
            join,
        } = e
            && track.value() == VIDEO
            && !matches!(join, Some(JoinDecision::Drop(_)))
        {
            return to27(sample.pts.unwrap(), ts[track]);
        }
    }
    panic!("no video sample after {from}");
}

fn seg1_url() -> &'static str {
    TS_SEG1_URL
}

fn partial_vod(playout: PlayoutConfig) -> Drv {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(playout), ts_bodies());
    d.serve_all_except(&[seg1_url()]);
    d
}

fn finish_vod(d: &mut Drv) {
    d.serve_all_except(&[]);
}

#[test]
fn hls_vod_start_after_vod_lead() {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(PlayoutConfig::default()), ts_bodies());
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    let expect = first_point_at_lead(&points, SEC * 27);
    let before_expect = points
        .iter()
        .rev()
        .find(|p| p.after < expect.after)
        .unwrap();
    assert!(before_expect.lead() < SEC * 27);
    assert!(
        expect.after < ended_index(&list),
        "the lead is met before the stream ends"
    );
    let m = marks(&d.events);
    assert_eq!(
        m,
        vec![(
            expect.after,
            BufferEvent::Start {
                at_27mhz: expect.origin
            }
        )]
    );
}

#[test]
fn hls_vod_shorter_than_lead_starts_at_end() {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(vod_lead(10_000)), ts_bodies());
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    assert!(points.iter().all(|p| p.lead() < 10 * TICKS_PER_SEC));
    let origin = points[0].origin;
    assert_eq!(
        marks(&d.events),
        vec![(ended_index(&list), BufferEvent::Start { at_27mhz: origin })]
    );
}

#[test]
fn hls_vod_pause_when_position_reaches_end_of_buffered_media_before_end() {
    let mut d = partial_vod(PlayoutConfig::default());
    let list = nb(&d.events);
    assert!(!list.iter().any(is_ended));
    let last = lead_points(&list, &[VIDEO, AUDIO]).pop().unwrap();
    assert_eq!(names(&d.events), vec!["start"]);
    d.pos(Some(last.origin));
    d.pos(Some(last.min_end - 1));
    assert_eq!(names(&d.events), vec!["start"]);
    d.pos(Some(last.min_end));
    assert_eq!(
        bufs(&d.events).last().unwrap(),
        &BufferEvent::Pause {
            hold_at_27mhz: last.min_end,
            reason: PauseReason::DataArriving,
            starving_track: Some(last.limiting.value())
        }
    );
    assert_eq!(names(&d.events), vec!["start", "pause"]);
}

#[test]
fn hls_vod_never_pauses_after_end() {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(PlayoutConfig::default()), ts_bodies());
    d.serve_all_except(&[]);
    let last = lead_points(&nb(&d.events), &[VIDEO, AUDIO]).pop().unwrap();
    d.pos(Some(last.origin));
    d.pos(Some(last.min_end));
    d.pos(Some(last.min_end + 5 * TICKS_PER_SEC));
    d.pos(Some(u64::MAX / 4));
    assert_eq!(names(&d.events), vec!["start"]);
}

#[test]
fn hls_vod_resume_when_more_media_arrives_to_lead() {
    let playout = PlayoutConfig {
        resume_lead_us: Some(500 * MS),
        ..PlayoutConfig::default()
    };
    let mut d = partial_vod(playout);
    let hold = lead_points(&nb(&d.events), &[VIDEO, AUDIO])
        .pop()
        .unwrap()
        .min_end;
    d.pos(Some(hold));
    assert_eq!(names(&d.events), vec!["start", "pause"]);
    let paused_at = nb(&d.events).len();
    finish_vod(&mut d);
    let list = nb(&d.events);
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    let expect = points
        .iter()
        .find(|p| p.after > paused_at && p.min_end.saturating_sub(hold) >= 500 * TICKS_PER_MS)
        .expect("resume lead reached by the delivered media");
    assert!(expect.after < ended_index(&list));
    let m = marks(&d.events);
    assert_eq!(m.len(), 3);
    assert_eq!(m[2], (expect.after, BufferEvent::Resume { at_27mhz: hold }));
}

#[test]
fn hls_vod_resume_immediately_when_end_arrives_while_paused() {
    let playout = PlayoutConfig {
        resume_lead_us: Some(60 * SEC),
        ..PlayoutConfig::default()
    };
    let mut d = partial_vod(playout);
    let hold = lead_points(&nb(&d.events), &[VIDEO, AUDIO])
        .pop()
        .unwrap()
        .min_end;
    d.pos(Some(hold));
    finish_vod(&mut d);
    let list = nb(&d.events);
    let m = marks(&d.events);
    assert_eq!(m.len(), 3);
    assert_eq!(
        m[2],
        (ended_index(&list), BufferEvent::Resume { at_27mhz: hold })
    );
}

fn long_stream() -> HashMap<String, Vec<u8>> {
    let events = oracle_events(TS_PLAYLIST_URL, &ts_bodies());
    let mut specs = Vec::new();
    let mut samples = Vec::new();
    for e in events {
        match e {
            Event::Tracks(t) => specs = t,
            Event::Sample { track, sample, .. } => samples.push((track, sample)),
            _ => {}
        }
    }
    let scale = |t: TrackId| i64::from(specs.iter().find(|s| s.track_id == t).unwrap().timescale);
    let mut all = Vec::new();
    for k in 0..LONG_REPEATS {
        for (t, s) in &samples {
            let mut s = s.clone();
            let off = k * SPAN_SECS * scale(*t);
            s.dts = s.dts.map(|d| d + off);
            s.pts = s.pts.map(|p| p + off);
            all.push((*t, s));
        }
    }
    all.sort_by_key(|(t, s)| i128::from(s.dts.unwrap()) * 90_000 / i128::from(scale(*t)));
    let mut seg = StreamingTsHlsSegmenter::new(specs, LONG_TARGET_SECS, LONG_WINDOW).unwrap();
    for (t, s) in all {
        seg.push(t, s).unwrap();
    }
    seg.finish().unwrap();
    let mut m = HashMap::new();
    for s in seg.take_ready() {
        m.insert(format!("{LIVE_DIR}/{}", s.uri), s.bytes);
    }
    assert_eq!(m.len(), SEG_URIS.len());
    m
}

fn live_playlist(count: usize) -> String {
    live_playlist_text(0, &SEG_URIS[..count])
}

fn live_drv(playout: PlayoutConfig, count: usize) -> Drv {
    let mut d = Drv::new(LIVE_PLAYLIST_URL, cfg(playout), long_stream());
    d.serve_text(LIVE_PLAYLIST_URL, &live_playlist(count));
    d.serve_all_except(&[LIVE_PLAYLIST_URL]);
    d
}

fn live_lead(ms: u64) -> PlayoutConfig {
    PlayoutConfig {
        live_start_lead_us: ms * MS,
        ..PlayoutConfig::default()
    }
}

#[test]
fn hls_live_uses_live_lead_not_vod_lead() {
    let playout = PlayoutConfig {
        vod_start_lead_us: SEC,
        live_start_lead_us: 3 * SEC,
        ..PlayoutConfig::default()
    };
    let d = live_drv(playout, 2);
    let list = nb(&d.events);
    assert!(!list.iter().any(is_ended));
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    let vod_point = first_point_at_lead(&points, SEC * 27);
    let live_point = first_point_at_lead(&points, 3 * TICKS_PER_SEC);
    assert!(vod_point.after < live_point.after);
    assert_eq!(
        marks(&d.events),
        vec![(
            live_point.after,
            BufferEvent::Start {
                at_27mhz: live_point.origin
            }
        )]
    );
}

#[test]
fn hls_live_start_lead_is_capped_by_three_target_durations() {
    let mut d = live_drv(live_lead(60_000), 3);
    assert!(bufs(&d.events).is_empty());
    d.wait_for(LIVE_PLAYLIST_URL);
    d.serve_text(LIVE_PLAYLIST_URL, &live_playlist(4));
    d.serve_segment(3);
    let list = nb(&d.events);
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    let expect = first_point_at_lead(&points, 3 * SEG_SECS * TICKS_PER_SEC);
    assert_eq!(
        marks(&d.events),
        vec![(
            expect.after,
            BufferEvent::Start {
                at_27mhz: expect.origin
            }
        )]
    );
}

#[test]
fn hls_live_pause_and_resume_cycle_with_delivery_gap() {
    let mut d = live_drv(live_lead(3_000), 2);
    let list = nb(&d.events);
    let p1 = lead_points(&list, &[VIDEO, AUDIO]).pop().unwrap();
    assert_eq!(names(&d.events), vec!["start"]);
    d.pos(Some(p1.origin + TICKS_PER_SEC));
    assert_eq!(names(&d.events), vec!["start"]);
    d.pos(Some(p1.min_end));
    assert_eq!(names(&d.events), vec!["start", "pause"]);

    d.wait_for(LIVE_PLAYLIST_URL);
    d.serve_text(LIVE_PLAYLIST_URL, &live_playlist(2));
    d.wait_for(LIVE_PLAYLIST_URL);
    d.serve_text(LIVE_PLAYLIST_URL, &live_playlist(2));
    assert_eq!(names(&d.events), vec!["start", "pause"]);

    d.wait_for(LIVE_PLAYLIST_URL);
    d.serve_text(LIVE_PLAYLIST_URL, &live_playlist(3));
    d.serve_segment(2);
    let after_seg2 = lead_points(&nb(&d.events), &[VIDEO, AUDIO]).pop().unwrap();
    assert!(after_seg2.min_end - p1.min_end < 3 * TICKS_PER_SEC);
    assert_eq!(names(&d.events), vec!["start", "pause"]);

    d.wait_for(LIVE_PLAYLIST_URL);
    d.serve_text(LIVE_PLAYLIST_URL, &live_playlist(4));
    d.serve_segment(3);
    let list = nb(&d.events);
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    let resume_point = points
        .iter()
        .find(|p| p.after > after_seg2.after && p.min_end - p1.min_end >= 3 * TICKS_PER_SEC)
        .expect("resume lead reached");
    let m = marks(&d.events);
    assert_eq!(names(&d.events), vec!["start", "pause", "resume"]);
    assert_eq!(
        m[2],
        (
            resume_point.after,
            BufferEvent::Resume {
                at_27mhz: p1.min_end
            }
        )
    );
}

fn finite_session(config: Config) -> Session {
    Session::open(FINITE_URL, config, 0).unwrap()
}

fn feed_finite(session: &mut Session, chunk: usize, positions: bool) -> Vec<Event> {
    let bytes = test_fixtures::read("ts/h264_aac.ts");
    let headers = vec![("Content-Length".to_string(), bytes.len().to_string())];
    let conn = match session.poll(0).expect("request") {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(conn, 200, &headers, 0);
    let mut out = Vec::new();
    for (i, c) in bytes.chunks(chunk).enumerate() {
        let now = i as u64 * 1_000;
        session.on_data(conn, c, now);
        if positions {
            session.on_playback_position(now, None);
        }
        while let Some(e) = session.next_event() {
            out.push(e);
        }
    }
    session.on_closed(conn, None, 10 * SEC);
    while let Some(e) = session.next_event() {
        out.push(e);
    }
    out
}

#[test]
fn finite_ts_vod_gate_events_not_buffering_runtime() {
    let mut s = finite_session(Config::default());
    let events = feed_finite(&mut s, 1316, true);
    let list = nb(&events);
    assert!(list.iter().any(|e| matches!(e, Event::Finite { .. })));
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    assert!(points.len() > 10);
    let expect = first_point_at_lead(&points, SEC * 27);
    assert!(expect.after < ended_index(&list));
    assert_eq!(
        marks(&events),
        vec![(
            expect.after,
            BufferEvent::Start {
                at_27mhz: expect.origin
            }
        )]
    );
    assert!(s.buffer_input_log().is_empty());
}

#[test]
fn live_ts_http_still_uses_buffering_runtime() {
    let bytes = read_tf1_bytes();
    let (status, headers) = read_tf1_headers();
    let mut s = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match s.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    s.on_response(conn, status, &headers, 0);
    let mut events = Vec::new();
    for (i, c) in bytes.chunks(1316).take(2_500).enumerate() {
        let now = i as u64 * 3_200;
        s.on_data(conn, c, now);
        s.on_playback_position(now, None);
        while let Some(e) = s.next_event() {
            events.push(e);
        }
    }
    assert!(!events.iter().any(|e| matches!(e, Event::Finite { .. })));
    assert!(names(&events).contains(&"start"));
    let log = s.buffer_input_log();
    assert!(
        log.iter()
            .any(|i| matches!(i, broadcast_client::BufferInput::Chunk { .. }))
    );
    assert!(
        log.iter()
            .any(|i| matches!(i, broadcast_client::BufferInput::TrackMedia { .. }))
    );
}

struct Renditions {
    d: Drv,
    video_end_url: Vec<String>,
    audio_urls: Vec<String>,
    audio_track: u32,
}

fn rendition_bodies() -> (HashMap<String, Vec<u8>>, Vec<String>, Vec<String>, u32) {
    let f = renditions::fixture();
    let mut bodies = HashMap::new();
    bodies.insert(
        MASTER_URL.to_string(),
        "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nv/index.m3u8\n"
            .as_bytes()
            .to_vec(),
    );
    let video = f
        .video
        .bodies(renditions::VIDEO_DIR, renditions::VIDEO_PLAYLIST_URL);
    let audio = f.audio.bodies(AUDIO_DIR, AUDIO_PLAYLIST_URL);
    let video_urls: Vec<String> = video.keys().cloned().collect();
    let audio_urls: Vec<String> = audio.keys().cloned().collect();
    bodies.extend(video);
    bodies.extend(audio);
    (
        bodies,
        video_urls,
        audio_urls,
        f.audio_stream_track_id.value() + AUDIO_TRACK_ID_OFFSET,
    )
}

fn renditions_drv(playout: PlayoutConfig) -> Renditions {
    let (bodies, video_end_url, audio_urls, audio_track) = rendition_bodies();
    let mut d = Drv::new(MASTER_URL, cfg(playout), bodies);
    d.session
        .set_rendered_tracks(&[tid(VIDEO), tid(audio_track)]);
    Renditions {
        d,
        video_end_url,
        audio_urls,
        audio_track,
    }
}

fn audio_segment_urls(r: &Renditions) -> Vec<String> {
    let mut v: Vec<String> = r
        .audio_urls
        .iter()
        .filter(|u| u.as_str() != AUDIO_PLAYLIST_URL)
        .cloned()
        .collect();
    v.sort();
    v
}

#[test]
fn rendition_audio_ends_join_the_gate_min() {
    let mut r = renditions_drv(PlayoutConfig::default());
    let audio_segs = audio_segment_urls(&r);
    let withheld: Vec<&str> = audio_segs.iter().map(String::as_str).collect();
    r.d.serve_all_except(&withheld);
    assert!(r.video_end_url.len() > 1);
    let video_only = nb(&r.d.events);
    assert!(
        video_only
            .iter()
            .filter(|e| matches!(e, Event::Sample { track, .. } if track.value() == VIDEO))
            .count()
            > 50
    );
    assert!(bufs(&r.d.events).is_empty(), "audio has not reported yet");

    r.d.serve(&audio_segs[0]);
    let list = nb(&r.d.events);
    let rendered = [VIDEO, r.audio_track];
    let points = lead_points(&list, &rendered);
    let expect = first_point_at_lead(&points, SEC * 27);
    let max_based = points
        .iter()
        .find(|p| p.max_end.saturating_sub(p.origin) >= SEC * 27)
        .unwrap();
    assert!(
        max_based.after < expect.after,
        "min over tracks is later than max"
    );
    let video_done = list
        .iter()
        .rposition(|e| matches!(e, Event::Sample { track, .. } if track.value() == VIDEO))
        .unwrap();
    assert!(
        expect.after > video_done,
        "start waits for the lagging audio rendition"
    );
    assert_eq!(
        marks(&r.d.events),
        vec![(
            expect.after,
            BufferEvent::Start {
                at_27mhz: expect.origin
            }
        )]
    );
}

#[test]
fn starving_track_is_the_track_with_smallest_end() {
    let mut r = renditions_drv(PlayoutConfig::default());
    let audio_segs = audio_segment_urls(&r);
    let withheld: Vec<&str> = audio_segs[1..].iter().map(String::as_str).collect();
    r.d.serve_all_except(&withheld);
    let list = nb(&r.d.events);
    assert!(!list.iter().any(is_ended));
    let last = lead_points(&list, &[VIDEO, r.audio_track]).pop().unwrap();
    assert_eq!(last.limiting.value(), r.audio_track, "audio lags the video");
    assert_eq!(names(&r.d.events), vec!["start"]);
    r.d.pos(Some(last.min_end));
    assert_eq!(
        bufs(&r.d.events).last().unwrap(),
        &BufferEvent::Pause {
            hold_at_27mhz: last.min_end,
            reason: PauseReason::DataArriving,
            starving_track: Some(r.audio_track)
        }
    );
}

#[test]
fn position_none_never_pauses() {
    let mut d = partial_vod(PlayoutConfig::default());
    let last = lead_points(&nb(&d.events), &[VIDEO, AUDIO]).pop().unwrap();
    for _ in 0..5 {
        d.pos(None);
    }
    assert_eq!(names(&d.events), vec!["start"]);
    d.pos(Some(last.min_end));
    assert_eq!(names(&d.events), vec!["start", "pause"]);
}

#[test]
fn start_at_is_origin_pts_27mhz() {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(PlayoutConfig::default()), ts_bodies());
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let first_video = first_video_pts_after(&list, 0);
    assert_eq!(first_video, FIRST_VIDEO_PTS_TICKS);
    assert_eq!(
        bufs(&d.events),
        vec![BufferEvent::Start {
            at_27mhz: FIRST_VIDEO_PTS_TICKS
        }]
    );

    let mut c = Drv::new(
        CMAF_PLAYLIST_URL,
        cfg(PlayoutConfig::default()),
        cmaf_bodies(),
    );
    c.serve_all_except(&[]);
    let list = nb(&c.events);
    let first_video = first_video_pts_after(&list, 0);
    assert_ne!(first_video, 0);
    assert_eq!(
        bufs(&c.events),
        vec![BufferEvent::Start {
            at_27mhz: first_video
        }]
    );
}

#[test]
fn set_rendered_tracks_audio_only_uses_audio_origin() {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(PlayoutConfig::default()), ts_bodies());
    d.session.set_rendered_tracks(&[tid(AUDIO)]);
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let points = lead_points(&list, &[AUDIO]);
    let expect = first_point_at_lead(&points, SEC * 27);
    assert_ne!(expect.origin, FIRST_VIDEO_PTS_TICKS);
    assert_eq!(
        marks(&d.events),
        vec![(
            expect.after,
            BufferEvent::Start {
                at_27mhz: expect.origin
            }
        )]
    );
}

#[test]
fn rejoin_resets_origin_and_restarts() {
    let mut d = partial_vod(vod_lead(500));
    assert_eq!(names(&d.events), vec!["start"]);
    d.session.rejoin();
    d.pump();
    let from = nb(&d.events).len();
    finish_vod(&mut d);
    let list = nb(&d.events);
    let origin = first_video_pts_after(&list, from);
    let points = lead_points_from(&list, &[VIDEO, AUDIO], from);
    let expect = first_point_at_lead(&points, 500 * TICKS_PER_MS);
    let m = marks(&d.events);
    assert_eq!(names(&d.events), vec!["start", "start"]);
    assert_eq!(
        m[1],
        (expect.after, BufferEvent::Start { at_27mhz: origin })
    );
    assert_ne!(origin, FIRST_VIDEO_PTS_TICKS);
}

#[test]
fn discontinuity_new_timeline_resets_and_starts_again() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.000000,\nindex0.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:1.000000,\nindex1.ts\n#EXT-X-ENDLIST\n";
    let mut bodies = ts_bodies();
    bodies.insert(TS_PLAYLIST_URL.to_string(), playlist.as_bytes().to_vec());
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(vod_lead(500)), bodies);
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let disc = list
        .iter()
        .position(|e| matches!(e, Event::Discontinuity))
        .expect("discontinuity event");
    let origin = first_video_pts_after(&list, disc);
    let points = lead_points_from(&list, &[VIDEO, AUDIO], disc);
    let expect = first_point_at_lead(&points, 500 * TICKS_PER_MS);
    let m = marks(&d.events);
    assert_eq!(names(&d.events), vec!["start", "start"]);
    assert!(m[0].0 <= disc);
    assert_eq!(
        m[1],
        (expect.after, BufferEvent::Start { at_27mhz: origin })
    );
}

#[test]
fn hls_variant_switch_resets_gate() {
    let mut bodies = HashMap::new();
    bodies.insert(
        SWITCH_MASTER_URL.to_string(),
        two_variant_master_text().into_bytes(),
    );
    for (playlist, dir) in [
        (LOW_PLAYLIST_URL, "http://origin.test/hls/low"),
        (HIGH_PLAYLIST_URL, "http://origin.test/hls/high"),
    ] {
        bodies.insert(playlist.to_string(), ts_playlist());
        bodies.insert(format!("{dir}/index0.ts"), ts_segment(0));
        bodies.insert(format!("{dir}/index1.ts"), ts_segment(1));
    }
    let mut d = Drv::new(SWITCH_MASTER_URL, cfg(vod_lead(500)), bodies);
    d.serve(SWITCH_MASTER_URL);
    d.serve_all_except(&["http://origin.test/hls/high/index1.ts"]);
    assert_eq!(names(&d.events), vec!["start"]);
    let from = nb(&d.events).len();
    d.session.select_variant(0).expect("select");
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let disc = from
        + list[from..]
            .iter()
            .position(|e| matches!(e, Event::Discontinuity))
            .expect("the switch announces a new timeline");
    let origin = first_video_pts_after(&list, disc);
    assert_eq!(origin, FIRST_VIDEO_PTS_TICKS);
    let points = lead_points_from(&list, &[VIDEO, AUDIO], disc);
    let expect = first_point_at_lead(&points, 500 * TICKS_PER_MS);
    let m = marks(&d.events);
    assert_eq!(names(&d.events), vec!["start", "start"]);
    assert_eq!(
        m[1],
        (expect.after, BufferEvent::Start { at_27mhz: origin })
    );
}

#[test]
fn config_knobs_change_leads() {
    let default = PlayoutConfig::default();
    assert_eq!(default.vod_start_lead_us, SEC);
    assert_eq!(default.live_start_lead_us, 6 * SEC);
    assert_eq!(default.resume_lead_us, None);
    assert_eq!(default.pause_lead_us, 0);

    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(vod_lead(2_000)), ts_bodies());
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let points = lead_points(&list, &[VIDEO, AUDIO]);
    let two = first_point_at_lead(&points, 2 * TICKS_PER_SEC);
    let one = first_point_at_lead(&points, TICKS_PER_SEC);
    assert!(one.after < two.after);
    assert_eq!(
        marks(&d.events),
        vec![(
            two.after,
            BufferEvent::Start {
                at_27mhz: two.origin
            }
        )]
    );

    let playout = PlayoutConfig {
        pause_lead_us: 500 * MS,
        ..PlayoutConfig::default()
    };
    let mut p = partial_vod(playout);
    let last = lead_points(&nb(&p.events), &[VIDEO, AUDIO]).pop().unwrap();
    let threshold = last.min_end - 500 * TICKS_PER_MS;
    p.pos(Some(threshold - 1));
    assert_eq!(names(&p.events), vec!["start"]);
    p.pos(Some(threshold));
    assert_eq!(
        bufs(&p.events).last().unwrap(),
        &BufferEvent::Pause {
            hold_at_27mhz: threshold,
            reason: PauseReason::DataArriving,
            starving_track: Some(last.limiting.value())
        }
    );

    let resume = PlayoutConfig {
        resume_lead_us: Some(2 * SEC),
        ..PlayoutConfig::default()
    };
    let mut r = partial_vod(resume);
    let hold = lead_points(&nb(&r.events), &[VIDEO, AUDIO])
        .pop()
        .unwrap()
        .min_end;
    r.pos(Some(hold));
    finish_vod(&mut r);
    let list = nb(&r.events);
    let m = marks(&r.events);
    assert_eq!(
        m[2].0,
        ended_index(&list),
        "2 s of extra media never arrives, only the end resumes"
    );
}

fn sample_marks(events: &[Event]) -> Vec<(usize, BufferEvent)> {
    let mut samples = 0;
    let mut out = Vec::new();
    for e in events {
        match e {
            Event::Buffer(b) => out.push((samples, b.clone())),
            Event::Sample { .. } => samples += 1,
            _ => {}
        }
    }
    out
}

type Marks = Vec<(usize, BufferEvent)>;

fn run_chunked(chunk: usize) -> (Marks, Marks) {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(PlayoutConfig::default()), ts_bodies());
    d.chunk = chunk;
    d.serve_all_except(&[]);
    let hls = sample_marks(&d.events);
    let mut s = finite_session(Config::default());
    let events = feed_finite(&mut s, chunk, false);
    (hls, sample_marks(&events))
}

#[test]
fn gate_events_deterministic_across_chunking() {
    let (hls_small, ts_small) = run_chunked(1);
    let (hls_big, ts_big) = run_chunked(65_536);
    assert_eq!(hls_small.len(), 1);
    assert_eq!(ts_small.len(), 1);
    assert_eq!(hls_small, hls_big);
    assert_eq!(ts_small, ts_big);
}

#[test]
fn no_events_before_first_sample() {
    let mut d = Drv::new(TS_PLAYLIST_URL, cfg(PlayoutConfig::default()), ts_bodies());
    d.session.set_rendered_tracks(&[tid(VIDEO), tid(AUDIO)]);
    d.serve(TS_PLAYLIST_URL);
    d.pos(Some(0));
    d.pos(None);
    assert!(bufs(&d.events).is_empty());
    assert!(!d.events.iter().any(|e| matches!(e, Event::Sample { .. })));
    d.serve_all_except(&[]);
    let list = nb(&d.events);
    let first_sample = list
        .iter()
        .position(|e| matches!(e, Event::Sample { .. }))
        .unwrap();
    let m = marks(&d.events);
    assert_eq!(m.len(), 1);
    assert!(m[0].0 > first_sample);
}

#[test]
fn start_emitted_once_until_pause() {
    let playout = PlayoutConfig {
        resume_lead_us: Some(500 * MS),
        ..PlayoutConfig::default()
    };
    let mut d = partial_vod(playout);
    let hold = lead_points(&nb(&d.events), &[VIDEO, AUDIO])
        .pop()
        .unwrap()
        .min_end;
    for step in 0..4 {
        d.pos(Some(hold - (4 - step) * TICKS_PER_SEC / 100));
    }
    assert_eq!(names(&d.events), vec!["start"]);
    d.pos(Some(hold));
    finish_vod(&mut d);
    d.pos(Some(hold));
    assert_eq!(names(&d.events), vec!["start", "pause", "resume"]);
    assert!(d.events.iter().any(is_ended));
}
