use std::collections::BTreeSet;

use broadcast_client::{Action, Config, Event, Session, SourceKind};
use broadcast_common::Unpackage;
use transmux::{FlvDemux, Media};

pub use test_harness::rtmp_play_peer as play_peer;

pub use play_peer::{PeerConfig, PlayPeer};

pub const MS: u64 = 1_000;
pub const SEC: u64 = 1_000_000;
pub const URL: &str = "rtmp://origin.test/live/cam1";
pub const STREAM: &str = "cam1";
pub const FIXTURE: &str = "flv/av.flv";
const FLV_HEADER_LEN: usize = 9;
const PREV_TAG_LEN: usize = 4;
const TAG_HEADER_LEN: usize = 11;

pub fn rtmp_config() -> Config {
    let mut c = Config::default();
    c.kind = SourceKind::Rtmp;
    c
}

pub fn peer_config() -> PeerConfig {
    PeerConfig {
        stream_name: STREAM.to_string(),
        ..PeerConfig::default()
    }
}

pub fn fixture_tags() -> Vec<(u8, u32, Vec<u8>)> {
    let flv = test_fixtures::read(FIXTURE);
    let mut at = FLV_HEADER_LEN + PREV_TAG_LEN;
    let mut tags = Vec::new();
    while at + TAG_HEADER_LEN <= flv.len() {
        let tag_type = flv[at];
        let size = u32::from_be_bytes([0, flv[at + 1], flv[at + 2], flv[at + 3]]) as usize;
        let ts = u32::from_be_bytes([flv[at + 7], flv[at + 4], flv[at + 5], flv[at + 6]]);
        tags.push((
            tag_type,
            ts,
            flv[at + TAG_HEADER_LEN..at + TAG_HEADER_LEN + size].to_vec(),
        ));
        at += TAG_HEADER_LEN + size + PREV_TAG_LEN;
    }
    tags
}

pub fn direct() -> Media {
    let flv = test_fixtures::read(FIXTURE);
    FlvDemux::new().unpackage(&flv[..]).expect("direct demux")
}

pub struct Connect {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

pub struct RtmpHost {
    pub session: Session,
    pub now: u64,
    pub events: Vec<Event>,
    pub connects: Vec<Connect>,
    pub closes: Vec<u32>,
    pub wakes: BTreeSet<u64>,
    pub peer: Option<PlayPeer>,
    pub conn: Option<u32>,
    pub peer_config: PeerConfig,
    pub sends: usize,
    pub fail_next_connect: bool,
    pub accept_connects: bool,
    pub silent: bool,
}

impl RtmpHost {
    pub fn new(url: &str, config: Config, peer_config: PeerConfig) -> Self {
        let session = Session::open(url, config, 0).expect("open");
        Self {
            session,
            now: 0,
            events: Vec::new(),
            connects: Vec::new(),
            closes: Vec::new(),
            wakes: BTreeSet::new(),
            peer: None,
            conn: None,
            peer_config,
            sends: 0,
            fail_next_connect: false,
            accept_connects: true,
            silent: false,
        }
    }

    pub fn collect(&mut self) {
        loop {
            let mut progressed = false;
            while let Some(action) = self.session.poll(self.now) {
                progressed = true;
                self.apply(action);
            }
            while let Some(e) = self.session.next_event() {
                progressed = true;
                self.events.push(e);
            }
            if !progressed {
                break;
            }
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::TcpConnect {
                conn,
                host,
                port,
                tls,
                ..
            } => {
                self.connects.push(Connect { host, port, tls });
                if self.fail_next_connect {
                    self.fail_next_connect = false;
                    self.session
                        .on_closed(conn, Some("connection refused".to_string()), self.now);
                } else if self.accept_connects {
                    self.conn = Some(conn);
                    self.peer = Some(PlayPeer::new(self.peer_config.clone()));
                    self.session.on_connected(conn);
                }
            }
            Action::Send { conn, bytes } => {
                self.sends += 1;
                if self.conn != Some(conn) || self.silent {
                    return;
                }
                let reply = match self.peer.as_mut() {
                    Some(p) => p.on_bytes(&bytes).expect("peer reads the client"),
                    None => return,
                };
                if !reply.is_empty() {
                    self.session.on_data(conn, &reply, self.now);
                }
            }
            Action::Close { conn } => {
                self.closes.push(conn);
                if self.conn == Some(conn) {
                    self.conn = None;
                    self.peer = None;
                }
            }
            Action::WakeAt(t) if t > self.now => {
                self.wakes.insert(t);
            }
            _ => {}
        }
    }

    pub fn advance_to(&mut self, t: u64) {
        self.now = t;
        self.collect();
    }

    pub fn send_to_session(&mut self, bytes: &[u8]) {
        let conn = self.conn.expect("a live connection");
        self.session.on_data(conn, bytes, self.now);
        self.collect();
    }

    pub fn play_all(&mut self) {
        for (tag_type, ts, body) in fixture_tags() {
            let bytes = self
                .peer
                .as_mut()
                .expect("peer")
                .media(tag_type, ts, &body)
                .expect("media");
            self.send_to_session(&bytes);
        }
    }

    pub fn peer_closes(&mut self, error: Option<&str>) {
        let conn = self.conn.take().expect("a live connection");
        self.peer = None;
        self.session
            .on_closed(conn, error.map(str::to_string), self.now);
        self.collect();
    }

    pub fn samples(&self) -> Vec<(u32, transmux::Sample)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Sample { track, sample, .. } => Some((track.value(), sample.clone())),
                _ => None,
            })
            .collect()
    }
}

pub fn connect_and_play(h: &mut RtmpHost) {
    h.collect();
    assert!(
        h.peer.as_ref().is_some_and(|p| p.is_playing()),
        "play was accepted"
    );
}

pub fn assert_samples_equal_direct(samples: &[(u32, transmux::Sample)]) {
    assert_samples_equal_direct_with(samples, 1);
}

pub fn assert_complete_samples_equal_direct(samples: &[(u32, transmux::Sample)]) {
    assert_samples_equal_direct_with(samples, 0);
}

/// A live demuxer holds each track's newest sample until the next tag of that track fixes its
/// duration; `held` is how many trailing samples per track are still held back.
pub fn assert_samples_equal_direct_with(samples: &[(u32, transmux::Sample)], held: usize) {
    let d = direct();
    for track in &d.tracks {
        let got: Vec<&transmux::Sample> = samples
            .iter()
            .filter(|(id, _)| *id == track.spec.track_id.value())
            .map(|(_, s)| s)
            .collect();
        assert_eq!(
            got.len(),
            track.samples.len() - held,
            "track {}",
            track.spec.track_id
        );
        for (a, b) in got.iter().zip(track.samples.iter()) {
            assert_eq!(a.data, b.data);
            assert_eq!(a.timing.map(|t| t.dts), b.timing.map(|t| t.dts));
            assert_eq!(a.timing.map(|t| t.pts), b.timing.map(|t| t.pts));
            assert_eq!(a.timing.map(|t| t.duration), b.timing.map(|t| t.duration));
        }
    }
}
