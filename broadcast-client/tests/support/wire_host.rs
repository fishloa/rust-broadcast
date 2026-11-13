use std::io::ErrorKind;
use std::net::{ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use broadcast_client::{Action, Config, Event, Session, UdpMode};

use super::replay::{Replay, TimedEvent};

const READ_TIMEOUT: Duration = Duration::from_millis(2);
const MAX_DATAGRAM: usize = 2048;

pub struct WireHost {
    pub session: Session,
    socket: Option<UdpSocket>,
    conn: u32,
    start: Instant,
    pub events: Vec<TimedEvent>,
    pub binds: usize,
    pub closes: usize,
    pub sends: usize,
}

impl WireHost {
    pub fn open(url: &str, config: Config) -> Self {
        Self {
            session: Session::open(url, config, 0).unwrap(),
            socket: None,
            conn: 0,
            start: Instant::now(),
            events: Vec::new(),
            binds: 0,
            closes: 0,
            sends: 0,
        }
    }

    pub fn now_us(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_micros()).unwrap()
    }

    pub fn pump(&mut self) {
        let now = self.now_us();
        while let Some(a) = self.session.poll(now) {
            match a {
                Action::UdpBind {
                    conn,
                    host,
                    port,
                    mode,
                    ..
                } => {
                    assert_eq!(mode, UdpMode::Connect);
                    let peer = (host.as_str(), port)
                        .to_socket_addrs()
                        .unwrap()
                        .next()
                        .unwrap();
                    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
                    socket.connect(peer).unwrap();
                    socket.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
                    self.socket = Some(socket);
                    self.conn = conn;
                    self.binds += 1;
                    self.session.on_connected(conn);
                }
                Action::Send { bytes, .. } => {
                    self.sends += 1;
                    if let Some(s) = &self.socket {
                        let _ = s.send(&bytes);
                    }
                }
                Action::Close { .. } => {
                    self.closes += 1;
                    self.socket = None;
                }
                _ => {}
            }
        }
        let now = self.now_us();
        while let Some(event) = self.session.next_event() {
            self.events.push(TimedEvent { at_us: now, event });
        }
        let Some(socket) = &self.socket else {
            std::thread::yield_now();
            return;
        };
        let mut buf = [0u8; MAX_DATAGRAM];
        match socket.recv(&mut buf) {
            Ok(n) => {
                let now = self.now_us();
                self.session.on_data(self.conn, &buf[..n], now);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => {}
        }
    }

    pub fn run_until(&mut self, deadline: Duration, mut done: impl FnMut(&Self) -> bool) -> bool {
        let end = Instant::now() + deadline;
        while Instant::now() < end {
            self.pump();
            if done(self) {
                return true;
            }
        }
        false
    }

    pub fn replay(&self) -> Replay {
        Replay {
            events: self.events.clone(),
        }
    }

    pub fn count(&self, pred: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|t| pred(&t.event)).count()
    }
}
