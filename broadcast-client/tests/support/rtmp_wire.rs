use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use broadcast_client::{Action, ClientIdentity, Config, Event, Session};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ServerConfig, ServerConnection, StreamOwned};

use super::rtmp_host::{PeerConfig, PlayPeer, fixture_tags};
use super::tls_pki::{Ca, client_config};

const READ_TIMEOUT: Duration = Duration::from_millis(5);
const SERVER_READ_TIMEOUT: Duration = Duration::from_millis(10);
const READ_CHUNK: usize = 64 * 1024;
const TLS_SERVER_NAME: &str = "localhost";

trait Io: Read + Write {}
impl<T: Read + Write> Io for T {}

#[derive(Debug, Clone, Copy)]
pub enum Serve {
    All,
    DropAfter(usize),
}

#[derive(Default)]
pub struct Log {
    pub accepted: usize,
    pub failed_before_data: usize,
    pub client_certs: Vec<Option<Vec<u8>>>,
    pub acks: Vec<Vec<u32>>,
}

pub struct PeerServer {
    pub addr: SocketAddr,
    pub log: Arc<Mutex<Log>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PeerServer {
    pub fn start(
        config: PeerConfig,
        scenarios: Vec<Serve>,
        tls: Option<Arc<ServerConfig>>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Log::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log2, stop2) = (log.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut served = 0usize;
            while !stop2.load(Ordering::Relaxed) {
                let Ok((tcp, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                tcp.set_nonblocking(false).unwrap();
                tcp.set_read_timeout(Some(SERVER_READ_TIMEOUT)).unwrap();
                log2.lock().unwrap().accepted += 1;
                let scenario = scenarios
                    .get(served)
                    .or(scenarios.last())
                    .copied()
                    .unwrap_or(Serve::All);
                served += 1;
                let mut certs = None;
                let io: Box<dyn Io> = match &tls {
                    Some(cfg) => {
                        let conn = ServerConnection::new(cfg.clone()).unwrap();
                        let mut s = StreamOwned::new(conn, tcp);
                        if s.conn.complete_io(&mut s.sock).is_err() {
                            log2.lock().unwrap().failed_before_data += 1;
                            continue;
                        }
                        certs = Some(
                            s.conn
                                .peer_certificates()
                                .and_then(|c| c.first())
                                .map(|c| c.to_vec()),
                        );
                        Box::new(s)
                    }
                    None => Box::new(tcp),
                };
                if let Some(c) = certs {
                    log2.lock().unwrap().client_certs.push(c);
                }
                let acks = serve(io, PlayPeer::new(config.clone()), scenario, &stop2);
                log2.lock().unwrap().acks.push(acks);
            }
        });
        Self {
            addr,
            log,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for PeerServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(mut io: Box<dyn Io>, mut peer: PlayPeer, scenario: Serve, stop: &AtomicBool) -> Vec<u32> {
    let mut buf = vec![0u8; READ_CHUNK];
    let mut streamed = false;
    while !stop.load(Ordering::Relaxed) {
        match io.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let out = peer.on_bytes(&buf[..n]).expect("peer reads the client");
                if io.write_all(&out).is_err() {
                    break;
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        if peer.is_playing() && !streamed {
            streamed = true;
            let limit = match scenario {
                Serve::All => usize::MAX,
                Serve::DropAfter(n) => n,
            };
            for (i, (t, ts, b)) in fixture_tags().into_iter().enumerate() {
                if i >= limit {
                    return peer.acks();
                }
                let bytes = peer.media(t, ts, &b).unwrap();
                if io.write_all(&bytes).is_err() {
                    return peer.acks();
                }
            }
            let eof = peer.stream_eof().unwrap();
            let _ = io.write_all(&eof);
            let _ = io.flush();
        }
    }
    peer.acks()
}

enum Wire {
    Plain(TcpStream),
    Tls(Box<StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Wire {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Wire::Plain(s) => s.read(buf),
            Wire::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Wire::Plain(s) => s.write(buf),
            Wire::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Wire::Plain(s) => s.flush(),
            Wire::Tls(s) => s.flush(),
        }
    }
}

pub struct WireHost {
    pub session: Session,
    pub events: Vec<Event>,
    pub connects: usize,
    pub offered_identity: usize,
    pub failed_connects: usize,
    conn: Option<(u32, Wire)>,
    start: Instant,
    server_ca: Option<Ca>,
    port: u16,
}

impl WireHost {
    pub fn open(url: &str, config: Config, port: u16, server_ca: Option<Ca>) -> Self {
        Self {
            session: Session::open(url, config, 0).unwrap(),
            events: Vec::new(),
            connects: 0,
            offered_identity: 0,
            failed_connects: 0,
            conn: None,
            start: Instant::now(),
            server_ca,
            port,
        }
    }

    pub fn now_us(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_micros()).unwrap()
    }

    fn connect(&self, tls: bool, identity: Option<&ClientIdentity>) -> Result<Wire, String> {
        let tcp = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        tcp.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
        if !tls {
            return Ok(Wire::Plain(tcp));
        }
        let ca = self.server_ca.as_ref().expect("a server CA for TLS");
        let cfg: Arc<ClientConfig> = client_config(ca, identity);
        let tls_conn =
            rustls::ClientConnection::new(cfg, ServerName::try_from(TLS_SERVER_NAME).unwrap())
                .map_err(|e| e.to_string())?;
        let mut stream = Box::new(StreamOwned::new(tls_conn, tcp));
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|e| e.to_string())?;
        Ok(Wire::Tls(stream))
    }

    pub fn pump(&mut self) {
        let now = self.now_us();
        let mut actions = Vec::new();
        while let Some(a) = self.session.poll(now) {
            actions.push(a);
        }
        for action in actions {
            match action {
                Action::TcpConnect {
                    conn,
                    tls,
                    identity,
                    ..
                } => {
                    self.connects += 1;
                    self.offered_identity += usize::from(identity.is_some());
                    match self.connect(tls, identity.as_deref()) {
                        Ok(w) => {
                            self.conn = Some((conn, w));
                            self.session.on_connected(conn);
                        }
                        Err(e) => {
                            self.failed_connects += 1;
                            self.session.on_closed(conn, Some(e), self.now_us());
                        }
                    }
                }
                Action::Send { conn, bytes } => {
                    if let Some((c, w)) = self.conn.as_mut()
                        && *c == conn
                        && w.write_all(&bytes).and_then(|_| w.flush()).is_err()
                    {
                        self.close_with_error(conn, "write failed");
                    }
                }
                Action::Close { conn } if self.conn.as_ref().is_some_and(|(c, _)| *c == conn) => {
                    self.conn = None;
                }
                _ => {}
            }
        }
        self.read_once();
        while let Some(e) = self.session.next_event() {
            self.events.push(e);
        }
    }

    fn close_with_error(&mut self, conn: u32, why: &str) {
        self.conn = None;
        self.session
            .on_closed(conn, Some(why.to_string()), self.now_us());
    }

    fn read_once(&mut self) {
        let Some((conn, wire)) = self.conn.as_mut() else {
            return;
        };
        let conn = *conn;
        let mut buf = vec![0u8; READ_CHUNK];
        match wire.read(&mut buf) {
            Ok(0) => {
                self.conn = None;
                self.session.on_closed(conn, None, self.now_us());
            }
            Ok(n) => {
                let now = self.now_us();
                self.session.on_data(conn, &buf[..n], now);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => self.close_with_error(conn, &e.to_string()),
        }
    }

    pub fn run_until(&mut self, deadline: Duration, stop: impl Fn(&WireHost) -> bool) {
        let end = Instant::now() + deadline;
        while Instant::now() < end && !stop(self) {
            self.pump();
            if self.conn.is_none() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        self.pump();
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
