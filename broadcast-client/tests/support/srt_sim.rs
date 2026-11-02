use broadcast_client::{Action, Config, Event, Session, SrtEntropySeed};

use super::replay::{Replay, TimedEvent};
use super::srt_peer::SrtPeer;

pub const TICK_US: u64 = 1_000;
pub const ONE_WAY_US: u64 = 2_000;
pub const SEED: [u8; 32] = [0x5A; 32];
const SEQ_MASK: u32 = 0x7FFF_FFFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fate {
    Deliver,
    Drop,
    Duplicate,
    DelayUs(u64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    ToPeer,
    ToSession,
}

pub fn config_with_seed(mut config: Config) -> Config {
    config.srt.entropy_seed = Some(SrtEntropySeed::new(SEED));
    config
}

pub struct SrtSim {
    pub session: Session,
    pub peers: Vec<SrtPeer>,
    pub now: u64,
    pub conn: Option<u32>,
    pub events: Vec<TimedEvent>,
    pub binds: Vec<Action>,
    pub closes: Vec<u32>,
    pub from_session: Vec<(u64, Vec<u8>)>,
    pub peer_listening: bool,
    pub last_wake: Option<u64>,
    pub fate: Box<dyn FnMut(u32, bool) -> Fate>,
    inflight: Vec<(u64, Dir, Vec<u8>)>,
    connected_to: usize,
}

impl SrtSim {
    pub fn open(url: &str, config: Config, peer: SrtPeer) -> Self {
        Self::open_with_peers(url, config, vec![peer])
    }

    pub fn open_with_peers(url: &str, config: Config, peers: Vec<SrtPeer>) -> Self {
        let session = Session::open(url, config, 0).unwrap();
        let mut sim = Self {
            session,
            peers,
            now: 0,
            conn: None,
            events: Vec::new(),
            binds: Vec::new(),
            closes: Vec::new(),
            from_session: Vec::new(),
            peer_listening: true,
            last_wake: None,
            fate: Box::new(|_, _| Fate::Deliver),
            inflight: Vec::new(),
            connected_to: 0,
        };
        sim.settle();
        sim
    }

    pub fn peer(&mut self) -> &mut SrtPeer {
        let i = self
            .connected_to
            .saturating_sub(1)
            .min(self.peers.len() - 1);
        &mut self.peers[i]
    }

    pub fn binds_count(&self) -> usize {
        self.binds.len()
    }

    fn settle(&mut self) {
        loop {
            let mut progressed = false;
            while let Some(a) = self.session.poll(self.now) {
                progressed = true;
                match a {
                    Action::UdpBind { conn, .. } => {
                        self.conn = Some(conn);
                        self.connected_to = self.binds.len() + 1;
                        self.binds.push(a);
                        self.session.on_connected(conn);
                    }
                    Action::Send { bytes, .. } => {
                        self.from_session.push((self.now, bytes.clone()));
                        if self.peer_listening {
                            self.inflight
                                .push((self.now + ONE_WAY_US, Dir::ToPeer, bytes));
                        }
                    }
                    Action::Close { conn } => self.closes.push(conn),
                    Action::WakeAt(at) => self.last_wake = Some(at),
                    _ => {}
                }
            }
            while let Some(event) = self.session.next_event() {
                progressed = true;
                self.events.push(TimedEvent {
                    at_us: self.now,
                    event,
                });
            }
            if !progressed {
                break;
            }
        }
    }

    fn deliver_due(&mut self) {
        let now = self.now;
        let mut due: Vec<(u64, Dir, Vec<u8>)> = Vec::new();
        self.inflight.retain(|item| {
            if item.0 <= now {
                due.push(item.clone());
                false
            } else {
                true
            }
        });
        due.sort_by_key(|d| d.0);
        for (_, dir, bytes) in due {
            match dir {
                Dir::ToPeer => {
                    let now = self.now;
                    let replies = self.peer().on_datagram(&bytes, now);
                    for r in replies {
                        self.queue_for_session(r, false);
                    }
                }
                Dir::ToSession => {
                    if let Some(conn) = self.conn {
                        self.session.on_data(conn, &bytes, self.now);
                    }
                }
            }
            self.settle();
        }
    }

    fn data_seq(bytes: &[u8]) -> Option<u32> {
        (bytes.len() >= 16 && bytes[0] & 0x80 == 0)
            .then(|| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & SEQ_MASK)
    }

    pub fn queue_for_session(&mut self, bytes: Vec<u8>, first: bool) {
        let fate = match Self::data_seq(&bytes) {
            Some(seq) => (self.fate)(seq, !first),
            None => Fate::Deliver,
        };
        let at = self.now + ONE_WAY_US;
        match fate {
            Fate::Deliver => self.inflight.push((at, Dir::ToSession, bytes)),
            Fate::Drop => {}
            Fate::Duplicate => {
                self.inflight.push((at, Dir::ToSession, bytes.clone()));
                self.inflight.push((at, Dir::ToSession, bytes));
            }
            Fate::DelayUs(d) => self.inflight.push((at + d, Dir::ToSession, bytes)),
        }
    }

    pub fn send_data(&mut self, payload: &[u8]) {
        let now = self.now;
        let d = self.peer().data(payload, now);
        self.queue_for_session(d, true);
    }

    pub fn raw_to_session(&mut self, bytes: Vec<u8>) {
        let at = self.now + ONE_WAY_US;
        self.inflight.push((at, Dir::ToSession, bytes));
    }

    pub fn advance_to(&mut self, to_us: u64) {
        while self.now < to_us {
            self.now = (self.now + TICK_US).min(to_us);
            self.deliver_due();
            self.settle();
        }
    }

    pub fn advance_by(&mut self, us: u64) {
        self.advance_to(self.now + us);
    }

    pub fn replay(&self) -> Replay {
        Replay {
            events: self.events.clone(),
        }
    }

    pub fn count(&self, pred: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|t| pred(&t.event)).count()
    }

    pub fn connect(&mut self) {
        self.advance_by(100 * TICK_US);
    }
}
