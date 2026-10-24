use std::collections::{BTreeSet, HashMap, VecDeque};

use broadcast_client::{Action, Config, Event, Session, SourceKind};

pub const ORIGIN: &str = "http://origin.test";
pub const TS_PLAYLIST_URL: &str = "http://origin.test/hls/index.m3u8";
pub const TS_SEG0_URL: &str = "http://origin.test/hls/index0.ts";
pub const TS_SEG1_URL: &str = "http://origin.test/hls/index1.ts";
pub const MS: u64 = 1_000;
pub const SEC: u64 = 1_000_000;

const TS_FIXTURE_DIR: &str = "../hls-runtime/tests/fixtures/ts-hls";
const CMAF_FIXTURE_DIR: &str = "hls/cmaf-fmp4";

pub fn ts_playlist() -> Vec<u8> {
    test_fixtures::read(&format!("{TS_FIXTURE_DIR}/index.m3u8"))
}

pub fn ts_segment(n: usize) -> Vec<u8> {
    test_fixtures::read(&format!("{TS_FIXTURE_DIR}/index{n}.ts"))
}

pub fn cmaf_init() -> Vec<u8> {
    test_fixtures::read(&format!("{CMAF_FIXTURE_DIR}/init.mp4"))
}

pub fn cmaf_segment(n: usize) -> Vec<u8> {
    test_fixtures::read(&format!("{CMAF_FIXTURE_DIR}/index{n}.m4s"))
}

pub fn cmaf_playlist_text() -> String {
    "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"init.mp4\"\n\
     #EXTINF:1.000,\nindex0.m4s\n#EXTINF:1.000,\nindex1.m4s\n#EXTINF:1.000,\nindex2.m4s\n\
     #EXT-X-ENDLIST\n"
        .to_string()
}

pub fn master_text(variant_uri: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d400d,mp4a.40.2\"\n{variant_uri}\n"
    )
}

pub fn two_variant_master_text() -> String {
    "#EXTM3U\n\
     #EXT-X-STREAM-INF:BANDWIDTH=200000,RESOLUTION=320x180\nlow/index.m3u8\n\
     #EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=1280x720\nhigh/index.m3u8\n"
        .to_string()
}

pub fn live_playlist_text(media_sequence: u64, segments: &[&str]) -> String {
    let mut s = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{media_sequence}\n"
    );
    for seg in segments {
        s.push_str(&format!("#EXTINF:2.000,\n{seg}\n"));
    }
    s
}

pub fn hls_config() -> Config {
    Config {
        kind: SourceKind::Hls,
        ..Config::default()
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    Ok(Vec<u8>),
    OkHeaders(Vec<u8>, Vec<(String, String)>),
    Full(u16, Vec<u8>, Vec<(String, String)>),
    Status(u16),
    StatusHeaders(u16, Vec<(String, String)>),
    Redirect(String),
    TransportError,
    Hang,
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub conn: u32,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub byte_range: Option<(u64, u64)>,
    pub at_us: u64,
}

impl Seen {
    pub fn authorization(&self) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.as_str())
    }
}

pub struct Host {
    pub session: Session,
    pub now: u64,
    pub chunk: usize,
    pub events: Vec<Event>,
    pub seen: Vec<Seen>,
    pub closes: Vec<u32>,
    pub wakes: BTreeSet<u64>,
    routes: HashMap<String, Vec<Reply>>,
    hits: HashMap<String, usize>,
    inflight: VecDeque<Seen>,
    hung: Vec<Seen>,
}

impl Host {
    pub fn new(url: &str, config: Config) -> Self {
        let session = Session::open(url, config, 0).expect("open");
        Self {
            session,
            now: 0,
            chunk: 65_536,
            events: Vec::new(),
            seen: Vec::new(),
            closes: Vec::new(),
            wakes: BTreeSet::new(),
            routes: HashMap::new(),
            hits: HashMap::new(),
            inflight: VecDeque::new(),
            hung: Vec::new(),
        }
    }

    pub fn route(&mut self, url: &str, replies: Vec<Reply>) -> &mut Self {
        self.routes.insert(url.to_string(), replies);
        self
    }

    pub fn ok(&mut self, url: &str, body: Vec<u8>) -> &mut Self {
        self.route(url, vec![Reply::Ok(body)])
    }

    pub fn hits(&self, url: &str) -> usize {
        self.hits.get(url).copied().unwrap_or(0)
    }

    pub fn hung(&self) -> &[Seen] {
        &self.hung
    }

    pub fn requests_to(&self, url: &str) -> Vec<&Seen> {
        self.seen.iter().filter(|s| s.url == url).collect()
    }

    pub fn collect(&mut self) {
        while let Some(action) = self.session.poll(self.now) {
            match action {
                Action::HttpRequest {
                    conn,
                    url,
                    headers,
                    byte_range,
                    ..
                } => {
                    let seen = Seen {
                        conn,
                        url,
                        headers,
                        byte_range,
                        at_us: self.now,
                    };
                    self.seen.push(seen.clone());
                    self.inflight.push_back(seen);
                }
                Action::Close { conn } => {
                    self.closes.push(conn);
                    self.inflight.retain(|s| s.conn != conn);
                    self.hung.retain(|s| s.conn != conn);
                }
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

    fn reply_for(&mut self, url: &str) -> Reply {
        let n = self.hits.entry(url.to_string()).or_insert(0);
        let idx = *n;
        *n += 1;
        match self.routes.get(url) {
            Some(replies) if !replies.is_empty() => replies[idx.min(replies.len() - 1)].clone(),
            _ => Reply::Status(404),
        }
    }

    fn respond(&mut self, req: Seen) {
        let reply = self.reply_for(&req.url);
        let conn = req.conn;
        match reply {
            Reply::Ok(body) => {
                self.session.on_response(conn, 200, &[]);
                for chunk in body.chunks(self.chunk.max(1)) {
                    self.session.on_data(conn, chunk, self.now);
                }
                self.session.on_closed(conn, None, self.now);
            }
            Reply::OkHeaders(body, headers) => {
                self.session.on_response(conn, 200, &headers);
                for chunk in body.chunks(self.chunk.max(1)) {
                    self.session.on_data(conn, chunk, self.now);
                }
                self.session.on_closed(conn, None, self.now);
            }
            Reply::Full(code, body, headers) => {
                self.session.on_response(conn, code, &headers);
                for chunk in body.chunks(self.chunk.max(1)) {
                    self.session.on_data(conn, chunk, self.now);
                }
                self.session.on_closed(conn, None, self.now);
            }
            Reply::Status(code) => {
                self.session.on_response(conn, code, &[]);
                self.session.on_closed(conn, None, self.now);
            }
            Reply::StatusHeaders(code, headers) => {
                self.session.on_response(conn, code, &headers);
                self.session.on_closed(conn, None, self.now);
            }
            Reply::Redirect(loc) => {
                self.session
                    .on_response(conn, 302, &[("Location".to_string(), loc)]);
            }
            Reply::TransportError => {
                self.session
                    .on_closed(conn, Some("reset".to_string()), self.now);
            }
            Reply::Hang => self.hung.push(req),
        }
    }

    pub fn step(&mut self) -> bool {
        self.collect();
        if let Some(req) = self.inflight.pop_front() {
            self.respond(req);
            return true;
        }
        if let Some(t) = self.wakes.range((self.now + 1)..).next().copied() {
            self.wakes.remove(&t);
            self.now = t;
            return true;
        }
        false
    }

    pub fn run_until(&mut self, stop: impl Fn(&[Event]) -> bool, max_steps: usize) {
        for _ in 0..max_steps {
            self.collect();
            if stop(&self.events) {
                return;
            }
            if !self.step() {
                self.collect();
                return;
            }
        }
        self.collect();
    }

    pub fn run_to_end(&mut self) {
        self.run_until(|evs| evs.iter().any(is_ended), 10_000);
    }

    pub fn samples(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, Event::Sample { .. }))
            .count()
    }
}

pub fn is_ended(e: &Event) -> bool {
    matches!(e, Event::Status(broadcast_client::Status::Ended))
}

pub fn drain_actions(session: &mut Session, now: u64) -> Vec<Action> {
    let mut out = Vec::new();
    while let Some(a) = session.poll(now) {
        out.push(a);
    }
    out
}

pub fn drain_events(session: &mut Session) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(e) = session.next_event() {
        out.push(e);
    }
    out
}

pub fn http_requests(actions: &[Action]) -> Vec<(u32, String)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::HttpRequest { conn, url, .. } => Some((*conn, url.clone())),
            _ => None,
        })
        .collect()
}

pub fn oracle_events(playlist_url: &str, bodies: &HashMap<String, Vec<u8>>) -> Vec<Event> {
    use hls_runtime::client::{Action as HlsAction, HlsClient, Output};
    let mut client = HlsClient::new(playlist_url);
    let mut out = Vec::new();
    for _ in 0..10_000 {
        while let Some(o) = client.next_output() {
            match o {
                Output::Tracks(t) => out.push(Event::Tracks(t)),
                Output::Samples { track_id, samples } => {
                    for sample in samples {
                        out.push(Event::Sample {
                            track: track_id,
                            sample,
                            join: None,
                        });
                    }
                }
                Output::DisplayChanged {
                    track_id,
                    pts,
                    display,
                } => out.push(Event::DisplayChanged {
                    track: track_id,
                    pts,
                    display,
                }),
                Output::Discontinuity => out.push(Event::Discontinuity),
                Output::EndOfStream => return out,
                _ => {}
            }
        }
        match client.poll() {
            Some(a @ HlsAction::FetchPlaylist { .. }) => {
                let url = a.playlist_request_url().expect("playlist url");
                client
                    .on_playlist(bodies.get(&url).expect("oracle playlist body"))
                    .expect("oracle playlist");
            }
            Some(HlsAction::FetchResource { id, url, .. }) => {
                client
                    .on_resource(id, bodies.get(&url).expect("oracle resource body"))
                    .expect("oracle resource");
            }
            Some(_) => {}
            None => break,
        }
    }
    out
}

pub fn data_events(events: &[Event]) -> Vec<Event> {
    events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Tracks(_)
                    | Event::Sample { .. }
                    | Event::DisplayChanged { .. }
                    | Event::Discontinuity
            )
        })
        .cloned()
        .collect()
}

pub fn statuses(events: &[Event]) -> Vec<broadcast_client::Status> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(s) => Some(*s),
            _ => None,
        })
        .collect()
}

pub fn ts_bodies() -> HashMap<String, Vec<u8>> {
    let mut m = HashMap::new();
    m.insert(TS_PLAYLIST_URL.to_string(), ts_playlist());
    m.insert(TS_SEG0_URL.to_string(), ts_segment(0));
    m.insert(TS_SEG1_URL.to_string(), ts_segment(1));
    m
}

pub fn host_with(bodies: &HashMap<String, Vec<u8>>, url: &str, config: Config) -> Host {
    let mut h = Host::new(url, config);
    for (u, b) in bodies {
        h.ok(u, b.clone());
    }
    h
}

pub const CMAF_BASE: &str = "http://origin.test/c";
pub const CMAF_PLAYLIST_URL: &str = "http://origin.test/c/index.m3u8";

pub fn cmaf_bodies() -> HashMap<String, Vec<u8>> {
    let mut m = HashMap::new();
    m.insert(
        CMAF_PLAYLIST_URL.to_string(),
        cmaf_playlist_text().into_bytes(),
    );
    m.insert(format!("{CMAF_BASE}/init.mp4"), cmaf_init());
    for i in 0..3 {
        m.insert(format!("{CMAF_BASE}/index{i}.m4s"), cmaf_segment(i));
    }
    m
}
