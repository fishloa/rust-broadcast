use broadcast_client::{Action, Config, Event, Session};

const TS_PACKET: usize = 188;
const DEFAULT_STEP_US: u64 = 10_000;

pub struct Host {
    pub session: Session,
    pub conn: u32,
    pub now: u64,
    pub log: Vec<Event>,
}

impl Host {
    pub fn open(config: Config) -> Self {
        let session = Session::open("http://h/live.ts", config, 0).unwrap();
        let mut host = Self {
            session,
            conn: 0,
            now: 0,
            log: Vec::new(),
        };
        host.settle();
        host
    }

    pub fn settle(&mut self) {
        while let Some(a) = self.session.poll(self.now) {
            if let Action::HttpRequest { conn, .. } = a {
                self.conn = conn;
                self.session.on_response(
                    conn,
                    200,
                    &[("Content-Type".to_string(), "video/MP2T".to_string())],
                    self.now,
                );
            }
        }
        while let Some(e) = self.session.next_event() {
            self.log.push(e);
        }
    }

    pub fn feed(&mut self, bytes: &[u8], chunk: usize) {
        let bytes = &bytes[..bytes.len() / TS_PACKET * TS_PACKET];
        for part in bytes.chunks(chunk) {
            self.now += DEFAULT_STEP_US;
            self.session.on_data(self.conn, part, self.now);
            self.settle();
        }
    }

    pub fn reconnect(&mut self) {
        self.session.reconnect(self.now);
        self.settle();
    }

    pub fn tick(&mut self, pts_27mhz: Option<u64>) {
        self.session.on_playback_position(self.now, pts_27mhz);
        self.settle();
    }

    pub fn announced(&self) -> Option<Vec<u32>> {
        self.log.iter().rev().find_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
    }

    pub fn samples_on(&self, track: u32) -> usize {
        self.log
            .iter()
            .filter(|e| matches!(e, Event::Sample { track: t, .. } if t.value() == track))
            .count()
    }

    pub fn mark(&self) -> usize {
        self.log.len()
    }
}
