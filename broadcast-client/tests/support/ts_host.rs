use broadcast_client::{Action, BufferEvent, Config, Event, Session};
use broadcast_common::stage::Timestamp;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}
const TS_PACKET: usize = 188;
const DEFAULT_STEP_US: u64 = 10_000;
const TICKS_PER_US: u64 = 27;

pub struct Host {
    pub session: Session,
    pub conn: ConnId,
    pub now: u64,
    pub log: Vec<Event>,
    playing: Option<(u64, u64)>,
    held: Option<u64>,
}

impl Host {
    pub fn open(config: Config) -> Self {
        let session = Session::open("http://h/live.ts", config, ts(0)).unwrap();
        let mut host = Self {
            session,
            conn: 0,
            now: 0,
            log: Vec::new(),
            playing: None,
            held: None,
        };
        host.settle();
        host
    }

    pub fn settle(&mut self) {
        while let Some(a) = self.session.poll(ts(self.now)) {
            if let Action::HttpRequest { conn, .. } = a {
                self.conn = conn;
                self.session.on_response(
                    conn,
                    200,
                    &[("Content-Type".to_string(), "video/MP2T".to_string())],
                    ts(self.now),
                );
            }
        }
        while let Some(e) = self.session.next_event() {
            match &e {
                Event::Buffer(
                    BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz },
                ) => {
                    self.playing = Some((*at_27mhz, self.now));
                    self.held = None;
                }
                Event::Buffer(BufferEvent::Pause { hold_at_27mhz, .. }) => {
                    self.playing = None;
                    self.held = Some(*hold_at_27mhz);
                }
                _ => {}
            }
            self.log.push(e);
        }
    }

    pub fn position(&self) -> Option<u64> {
        match self.playing {
            Some((base, since)) => Some(base + (self.now - since) * TICKS_PER_US),
            None => self.held,
        }
    }

    pub fn feed_playing(&mut self, bytes: &[u8], chunk: usize) {
        let bytes = &bytes[..bytes.len() / TS_PACKET * TS_PACKET];
        for part in bytes.chunks(chunk) {
            self.now += DEFAULT_STEP_US;
            self.session.on_data(self.conn, part, ts(self.now));
            self.settle();
            self.tick(self.position());
        }
    }

    pub fn feed(&mut self, bytes: &[u8], chunk: usize) {
        let bytes = &bytes[..bytes.len() / TS_PACKET * TS_PACKET];
        for part in bytes.chunks(chunk) {
            self.now += DEFAULT_STEP_US;
            self.session.on_data(self.conn, part, ts(self.now));
            self.settle();
        }
    }

    #[cfg(feature = "test-seams")]
    pub fn reconnect(&mut self) {
        self.session.reconnect(ts(self.now));
        self.settle();
    }

    pub fn tick(&mut self, pts_27mhz: Option<u64>) {
        self.session.on_playback_position(ts(self.now), pts_27mhz);
        self.settle();
    }

    pub fn announced(&self) -> Option<Vec<u32>> {
        self.log.iter().rev().find_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
    }

    #[cfg(feature = "test-seams")]
    pub fn samples_on(&self, track: u32) -> usize {
        self.log
            .iter()
            .filter(|e| matches!(e, Event::Sample { track: t, .. } if t.value() == track))
            .count()
    }

    #[cfg(feature = "test-seams")]
    pub fn mark(&self) -> usize {
        self.log.len()
    }
}
