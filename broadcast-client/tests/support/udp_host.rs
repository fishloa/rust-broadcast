use broadcast_client::{Action, Config, Event, Session, UdpMode};
use broadcast_common::Serialize;
use rtp_packet::RtpPacket;

use super::replay::{Replay, TimedEvent};

pub const TS_PACKET: usize = 188;
pub const MP2T_PAYLOAD_TYPE: u8 = 33;
pub const DATAGRAM_STEP_US: u64 = 1_000;
const RTP_TS_PER_PACKET: u32 = 90;

pub struct UdpHost {
    pub session: Session,
    pub conn: Option<u32>,
    pub now: u64,
    pub events: Vec<TimedEvent>,
    pub binds: Vec<Action>,
    pub closes: Vec<u32>,
    pub wake: Option<u64>,
    pub auto_connect: bool,
}

impl UdpHost {
    pub fn open(url: &str, config: Config) -> Self {
        let session = Session::open(url, config, 0).unwrap();
        let mut host = Self {
            session,
            conn: None,
            now: 0,
            events: Vec::new(),
            binds: Vec::new(),
            closes: Vec::new(),
            wake: None,
            auto_connect: true,
        };
        host.settle();
        host
    }

    pub fn settle(&mut self) {
        while let Some(a) = self.session.poll(self.now) {
            match a {
                Action::UdpBind { conn, .. } => {
                    self.conn = Some(conn);
                    self.binds.push(a);
                    if self.auto_connect {
                        self.session.on_connected(conn);
                    }
                }
                Action::Close { conn } => self.closes.push(conn),
                Action::WakeAt(at) => self.wake = Some(at),
                _ => {}
            }
        }
        while let Some(event) = self.session.next_event() {
            self.events.push(TimedEvent {
                at_us: self.now,
                event,
            });
        }
    }

    pub fn datagram(&mut self, bytes: &[u8]) {
        self.now += DATAGRAM_STEP_US;
        let conn = self.conn.expect("a bound socket");
        self.session.on_data(conn, bytes, self.now);
        self.settle();
    }

    pub fn advance_to(&mut self, at_us: u64) {
        self.now = self.now.max(at_us);
        self.settle();
    }

    pub fn close_socket(&mut self) {
        let conn = self.conn.expect("a bound socket");
        self.session.on_closed(conn, None, self.now);
        self.settle();
    }

    pub fn replay(&self) -> Replay {
        Replay {
            events: self.events.clone(),
        }
    }

    pub fn count(&self, pred: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|t| pred(&t.event)).count()
    }

    pub fn bind_modes(&self) -> Vec<UdpMode> {
        self.binds
            .iter()
            .filter_map(|a| match a {
                Action::UdpBind { mode, .. } => Some(*mode),
                _ => None,
            })
            .collect()
    }
}

pub fn ts_datagrams(ts: &[u8], packets_per_datagram: usize) -> Vec<Vec<u8>> {
    let whole = &ts[..ts.len() / TS_PACKET * TS_PACKET];
    whole
        .chunks(packets_per_datagram * TS_PACKET)
        .map(<[u8]>::to_vec)
        .collect()
}

pub fn rtp_datagram(seq: u16, ssrc: u32, payload_type: u8, payload: &[u8]) -> Vec<u8> {
    let pkt = RtpPacket {
        marker: false,
        payload_type,
        sequence_number: seq,
        timestamp: u32::from(seq) * RTP_TS_PER_PACKET,
        ssrc,
        csrc: Vec::new(),
        extension: None,
        padding: None,
        payload,
    };
    let mut buf = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut buf).unwrap();
    buf
}

pub fn rtp_datagrams(ts: &[u8], per: usize, first_seq: u16, ssrc: u32) -> Vec<Vec<u8>> {
    ts_datagrams(ts, per)
        .iter()
        .enumerate()
        .map(|(i, payload)| {
            rtp_datagram(
                first_seq.wrapping_add(i as u16),
                ssrc,
                MP2T_PAYLOAD_TYPE,
                payload,
            )
        })
        .collect()
}
