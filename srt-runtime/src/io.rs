//! Async real-socket UDP adapter over the sans-IO SRT engine.
//!
//! The sans-IO [`crate::caller::CallerHandshake`] / [`crate::listener::ListenerHandshake`]
//! engines never touch a socket — they turn state transitions into typed events
//! and byte buffers. The ARQ sender/receiver, TSBPD scheduler, and LiveCC
//! pacing controller follow the same contract: all take caller-supplied
//! `now: core::time::Duration` and never read a wall clock.
//!
//! This module is the thin tokio layer that actually moves bytes over a
//! [`tokio::net::UdpSocket`], drives the handshake to completion, and then
//! runs the data-transfer select loop that pumps socket RX → engines → socket
//! TX and ticks timers (retransmit, ACK, NAK, TSBPD release) via
//! [`tokio::time`]. The sans-IO core stays `no_std`; the adapter is pure
//! plumbing.
//!
//! # Structure
//!
//! - [`SrtListener`] — binds a UDP port and accepts incoming SRT connections,
//!   returning a [`SrtSocket`] per connected peer.
//! - [`SrtSocket`] — an established SRT connection (caller or listener role)
//!   with async [`send`](SrtSocket::send) and [`recv`](SrtSocket::recv) for
//!   application payloads.
//!
//! # Feature gate
//!
//! Only available with `features = ["tokio"]` (implies `std`). Without the
//! `tokio` feature, the crate stays `no_std`+`alloc` and nothing in this
//! module is compiled.

use std::sync::Arc;

use alloc::collections::VecDeque;
use core::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::Instant;

use crate::arq::{Receiver as ArqReceiver, Sender as ArqSender};
use crate::caller::{CallerHandshake, CallerHandshakeState};
use crate::error::{Error, Result};
use crate::handshake_sm::{HandshakeConfig, HandshakeOutput};
use crate::listener::{ListenerHandshake, ListenerHandshakeState};
use crate::livecc::{LiveCC, MaxBwConfig};
use crate::packet::misc::KeepAlivePacket;
use crate::packet::{ControlPacket, SrtPacket};
use crate::tsbpd::TsbpdScheduler;

// ===========================================================================
// Constants
// ===========================================================================

/// Maximum UDP datagram size.
const MAX_DATAGRAM: usize = 1500;
/// Maximum payloads to dequeue per send cycle.
const MAX_SEND_BATCH: usize = 8;
/// Tick interval for timer-based engine work (ACK, NAK, retransmit, TSBPD).
const TICK_INTERVAL_MS: u64 = 5;
/// Default TSBPD drift (zero when no estimate available).
const DEFAULT_DRIFT_US: u64 = 0;
/// Enable TLPKTDROP by default.
const DEFAULT_TLPKT_DROP_ENABLED: bool = true;
/// Default max bandwidth (1 Gbps).
const DEFAULT_MAX_BW: MaxBwConfig = MaxBwConfig::Set(125_000_000);
/// Handshake timeout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

// ===========================================================================
// SrtSocket — an established SRT connection
// ===========================================================================

/// An established SRT connection over UDP.
///
/// Created by [`SrtSocket::connect`] (caller role) or
/// [`SrtListener::accept`] (listener role).
#[derive(Debug)]
pub struct SrtSocket {
    udp: Arc<UdpSocket>,
    peer_addr: std::net::SocketAddr,
    peer_socket_id: u32,

    // ARQ
    sender: ArqSender,
    receiver: ArqReceiver,

    // TSBPD
    tsbpd: TsbpdScheduler,

    // LiveCC pacing
    livecc: LiveCC,

    // IO state
    send_queue: VecDeque<Vec<u8>>,
    ready_payloads: VecDeque<Vec<u8>>,
    next_message_number: u32,
    next_send_seq: u32,

    // Wall-clock epoch for `now: Duration`.
    epoch: Instant,

    // Staging: seq → payload bytes, released by TSBPD.
    staged: std::collections::BTreeMap<u32, Vec<u8>>,

    // Outbound byte buffer.
    outbound: VecDeque<Vec<u8>>,

    peer_shutdown: bool,
}

impl SrtSocket {
    /// Connect to a remote SRT peer as a Caller.
    pub async fn connect<A: tokio::net::ToSocketAddrs>(
        remote_addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        let local = "0.0.0.0:0".parse::<std::net::SocketAddr>().unwrap();
        Self::connect_from(local, remote_addr, config).await
    }

    /// Connect from a specific local address.
    pub async fn connect_from<A: tokio::net::ToSocketAddrs>(
        local_addr: std::net::SocketAddr,
        remote_addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        let socket = UdpSocket::bind(local_addr)
            .await
            .map_err(|e| io_err("bind", e))?;
        let peer = resolve_one(remote_addr).await?;
        let socket = Arc::new(socket);

        let own_socket_id = config.initial_seq_number;
        let mut hs = CallerHandshake::new(own_socket_id, config.clone());

        // Send INDUCTION.
        let induction = hs.start().map_err(|_| Error::InvalidField {
            what: "caller start",
            reason: "start failed",
        })?;
        socket
            .send_to(&induction, peer)
            .await
            .map_err(|e| io_err("send induction", e))?;

        let mut buf = [0u8; MAX_DATAGRAM];
        let _peer_initial_seq = 0u32;

        loop {
            match hs.state() {
                CallerHandshakeState::Connected => break,
                CallerHandshakeState::Rejected | CallerHandshakeState::TimedOut => {
                    return Err(Error::InvalidField {
                        what: "hs state",
                        reason: "rejected or timed out",
                    });
                }
                _ => {}
            }

            let n = tokio::time::timeout(HANDSHAKE_TIMEOUT, socket.recv_from(&mut buf)).await;

            match n {
                Ok(Ok((len, _src))) => {
                    let bytes = &buf[..len];
                    let outcomes = hs.feed_bytes(bytes).map_err(|_| Error::InvalidField {
                        what: "hs feed",
                        reason: "feed failed",
                    })?;

                    for outcome in outcomes {
                        match outcome {
                            HandshakeOutput::Send(bytes) => {
                                socket
                                    .send_to(&bytes, peer)
                                    .await
                                    .map_err(|e| io_err("send hs", e))?;
                            }
                            HandshakeOutput::Connected(_params) => {
                                // Extract the peer's ISN from the bytes we
                                // just fed (the listener's handshake response
                                // carries its initial_seq_number).
                                let peer_isn = extract_isn_from_bytes(bytes).unwrap_or(0);
                                let epoch = Instant::now();
                                let tsbpd_delay_ms = u64::from(config.latency_ms);
                                let tsbpd_time_base = 0u64;
                                let conn = SrtSocket::new(
                                    socket,
                                    peer,
                                    config.initial_seq_number,
                                    peer_isn,
                                    tsbpd_time_base,
                                    tsbpd_delay_ms,
                                    epoch,
                                );
                                return Ok(conn);
                            }
                            HandshakeOutput::Rejected(_) => {
                                return Err(Error::InvalidField {
                                    what: "hs rejected",
                                    reason: "peer rejected",
                                });
                            }
                            HandshakeOutput::TimedOut => {
                                return Err(Error::InvalidField {
                                    what: "hs timeout",
                                    reason: "caller timed out",
                                });
                            }
                        }
                    }
                }
                Ok(Err(e)) => return Err(io_err("recv hs", e)),
                Err(_) => {
                    // Tick retransmit.
                    for outcome in hs.tick() {
                        match outcome {
                            HandshakeOutput::Send(bytes) => {
                                socket
                                    .send_to(&bytes, peer)
                                    .await
                                    .map_err(|e| io_err("retransmit", e))?;
                            }
                            HandshakeOutput::TimedOut => {
                                return Err(Error::InvalidField {
                                    what: "hs timeout",
                                    reason: "retransmit exhausted",
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Err(Error::InvalidField {
            what: "handshake",
            reason: "unreachable",
        })
    }

    fn new(
        udp: Arc<UdpSocket>,
        peer_addr: std::net::SocketAddr,
        our_initial_seq: u32,
        peer_initial_seq: u32,
        tsbpd_time_base: u64,
        tsbpd_delay_ms: u64,
        epoch: Instant,
    ) -> Self {
        SrtSocket {
            udp,
            peer_addr,
            peer_socket_id: peer_initial_seq,
            sender: ArqSender::new(peer_initial_seq),
            receiver: ArqReceiver::new(peer_initial_seq, peer_initial_seq),
            tsbpd: TsbpdScheduler::new(
                peer_initial_seq,
                tsbpd_time_base,
                tsbpd_delay_ms,
                DEFAULT_DRIFT_US,
                DEFAULT_TLPKT_DROP_ENABLED,
                None,
            ),
            livecc: LiveCC::new(DEFAULT_MAX_BW),
            send_queue: VecDeque::new(),
            ready_payloads: VecDeque::new(),
            next_message_number: 1,
            next_send_seq: our_initial_seq,
            epoch,
            staged: std::collections::BTreeMap::new(),
            outbound: VecDeque::new(),
            peer_shutdown: false,
        }
    }

    /// Send a payload to the peer.
    pub async fn send(&mut self, payload: &[u8]) -> Result<()> {
        self.send_queue.push_back(payload.to_vec());
        self.drive_send().await
    }

    /// The peer's socket address.
    pub fn peer_addr(&self) -> std::net::SocketAddr {
        self.peer_addr
    }

    /// Receive the next payload, waiting until one is available.
    /// Returns `None` if the peer has cleanly shut down.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            if let Some(payload) = self.ready_payloads.pop_front() {
                return Ok(Some(payload));
            }
            if self.peer_shutdown && self.ready_payloads.is_empty() {
                return Ok(None);
            }
            self.drive_recv().await?;
        }
    }

    async fn drive_recv(&mut self) -> Result<()> {
        let mut buf = [0u8; MAX_DATAGRAM];

        loop {
            if !self.ready_payloads.is_empty() || self.peer_shutdown {
                return Ok(());
            }

            let n = tokio::time::timeout(
                Duration::from_millis(TICK_INTERVAL_MS),
                self.udp.recv_from(&mut buf),
            )
            .await;

            match n {
                Ok(Ok((len, src))) if src == self.peer_addr => {
                    self.ingress(&buf[..len])?;
                }
                Ok(Ok((_len, _src))) => {
                    // Datagram from another peer; ignore.
                }
                Ok(Err(e)) => return Err(io_err("recv", e)),
                Err(_) => {
                    self.tick_engines();
                }
            }

            self.flush_outbound().await?;
        }
    }

    async fn drive_send(&mut self) -> Result<()> {
        self.tick_engines();

        let batch: Vec<Vec<u8>> = self
            .send_queue
            .drain(..MAX_SEND_BATCH.min(self.send_queue.len()))
            .collect();

        for payload in &batch {
            self.send_one(payload);
        }

        self.flush_outbound().await?;
        Ok(())
    }

    fn send_one(&mut self, payload: &[u8]) {
        let now = self.elapsed();
        self.livecc.on_data_packet(payload.len() as u64);
        let bytes = self
            .sender
            .on_data(self.next_send_seq, self.next_message_number, payload, now);
        self.next_send_seq = self.next_send_seq.wrapping_add(1);
        self.next_message_number = self.next_message_number.wrapping_add(1);
        self.outbound.push_back(bytes);
    }

    fn ingress(&mut self, bytes: &[u8]) -> Result<()> {
        let now = self.elapsed();
        let packet = SrtPacket::parse(bytes)?;

        match packet {
            SrtPacket::Data(d) => {
                let outcome = self.receiver.feed_data(d.seq_number, now);
                if let Some(nak_bytes) = outcome.nak {
                    self.outbound.push_back(nak_bytes);
                }

                self.staged
                    .entry(d.seq_number)
                    .or_insert_with(|| d.data.to_vec());

                let tsbpd_out = self.tsbpd.feed_data(d.seq_number, d.timestamp, now);

                for &seq in &tsbpd_out.delivered {
                    if let Some(payload) = self.staged.remove(&seq) {
                        self.ready_payloads.push_back(payload);
                    }
                }

                for &seq in &outcome.delivered {
                    if let Some(payload) = self.staged.remove(&seq) {
                        self.ready_payloads.push_back(payload);
                    }
                }
            }
            SrtPacket::Control(ref c) => match c {
                ControlPacket::Ack(ack) => {
                    if let Some(ackack_bytes) = self.sender.on_ack(ack, now) {
                        self.outbound.push_back(ackack_bytes);
                    }
                }
                ControlPacket::Nak(nak) => {
                    self.sender.on_nak(nak);
                }
                ControlPacket::AckAck(ackack) => {
                    self.receiver.on_ackack(ackack, now);
                }
                ControlPacket::KeepAlive(_) => {
                    let pkt = ControlPacket::KeepAlive(KeepAlivePacket {
                        timestamp: self.elapsed_us(),
                        dest_socket_id: self.peer_socket_id,
                    });
                    let mut buf = vec![0u8; pkt.serialized_len()];
                    let _ = pkt.serialize_into(&mut buf);
                    self.outbound.push_back(buf);
                }
                ControlPacket::Shutdown(_) => {
                    self.peer_shutdown = true;
                }
                _ => {}
            },
        }

        Ok(())
    }

    fn tick_engines(&mut self) {
        let now = self.elapsed();

        for bytes in self.sender.tick(now) {
            self.outbound.push_back(bytes);
        }

        for bytes in self.receiver.tick(now) {
            self.outbound.push_back(bytes);
        }

        let tsbpd_out = self.tsbpd.tick(now);
        for &seq in &tsbpd_out.delivered {
            if let Some(payload) = self.staged.remove(&seq) {
                self.ready_payloads.push_back(payload);
            }
        }
    }

    fn elapsed(&self) -> Duration {
        Instant::now().duration_since(self.epoch)
    }

    fn elapsed_us(&self) -> u32 {
        self.elapsed().as_micros().min(u128::from(u32::MAX)) as u32
    }

    async fn flush_outbound(&mut self) -> Result<()> {
        while let Some(bytes) = self.outbound.pop_front() {
            let period = self.livecc.on_ack_received();
            if period > Duration::ZERO {
                tokio::time::sleep(period).await;
            }
            self.udp
                .send_to(&bytes, self.peer_addr)
                .await
                .map_err(|e| io_err("send", e))?;
        }
        Ok(())
    }
}

// ===========================================================================
// SrtListener
// ===========================================================================

/// An SRT listener that accepts incoming Caller connections.
#[derive(Debug)]
pub struct SrtListener {
    udp: Arc<UdpSocket>,
    config: HandshakeConfig,
    next_socket_id: u32,
    pending: std::collections::HashMap<std::net::SocketAddr, PendingListener>,
    outbound_queue: std::collections::HashMap<std::net::SocketAddr, VecDeque<Vec<u8>>>,
}

#[derive(Debug)]
struct PendingListener {
    handshake: ListenerHandshake,
    params: Option<HandshakeOutput>,
    /// The peer's ISN, extracted from the INDUCTION handshake packet.
    peer_initial_seq: u32,
}

impl SrtListener {
    /// Bind an SRT listener on `addr`.
    pub async fn bind<A: tokio::net::ToSocketAddrs>(
        addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        let socket = UdpSocket::bind(addr).await.map_err(|e| io_err("bind", e))?;
        Ok(SrtListener {
            udp: Arc::new(socket),
            config,
            next_socket_id: 1,
            pending: std::collections::HashMap::new(),
            outbound_queue: std::collections::HashMap::new(),
        })
    }

    /// The local socket address the listener is bound to.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.udp.local_addr().map_err(|e| io_err("local_addr", e))
    }

    /// Accept the next incoming SRT connection.
    pub async fn accept(&mut self) -> Result<SrtSocket> {
        let mut buf = [0u8; MAX_DATAGRAM];

        loop {
            if let Some(conn) = self.drain_completed() {
                return conn;
            }

            let n = tokio::time::timeout(Duration::from_millis(100), self.udp.recv_from(&mut buf))
                .await;

            match n {
                Ok(Ok((len, src))) => {
                    let _ = self.handle_datagram(src, &buf[..len]);
                    self.flush_for_peer(src).await?;
                }
                Ok(Err(e)) => return Err(io_err("recv_from", e)),
                Err(_) => {
                    self.tick_pending();
                    self.flush_all().await?;
                }
            }
        }
    }

    fn handle_datagram(&mut self, src: std::net::SocketAddr, bytes: &[u8]) -> Result<()> {
        let packet = SrtPacket::parse(bytes).map_err(|_| Error::InvalidField {
            what: "parse",
            reason: "non-SRT datagram",
        })?;

        let ctrl = match packet {
            SrtPacket::Control(c) => c,
            _ => return Ok(()),
        };

        let is_new = !self.pending.contains_key(&src);

        if is_new {
            let peer_isn = match &ctrl {
                ControlPacket::Handshake(hp) => hp.initial_seq_number,
                _ => return Ok(()),
            };

            let own_socket_id = self.next_socket_id;
            self.next_socket_id = self.next_socket_id.wrapping_add(1);
            let syn_cookie = 0xC0FF_EE42;
            let hs = ListenerHandshake::new(own_socket_id, syn_cookie, self.config.clone());
            self.pending.insert(
                src,
                PendingListener {
                    handshake: hs,
                    params: None,
                    peer_initial_seq: peer_isn,
                },
            );
        }

        let entry = self.pending.get_mut(&src).ok_or(Error::InvalidField {
            what: "pending",
            reason: "no pending entry",
        })?;

        let outcomes = entry
            .handshake
            .feed(&ctrl)
            .map_err(|_| Error::InvalidField {
                what: "listener feed",
                reason: "feed failed",
            })?;

        for outcome in outcomes {
            match outcome {
                HandshakeOutput::Send(bytes) => {
                    self.outbound_queue.entry(src).or_default().push_back(bytes);
                }
                HandshakeOutput::Connected(_) => {
                    entry.params = Some(HandshakeOutput::Connected(
                        entry.handshake.negotiated().unwrap().clone(),
                    ));
                }
                HandshakeOutput::Rejected(_) => {
                    self.pending.remove(&src);
                    return Err(Error::InvalidField {
                        what: "hs rejected",
                        reason: "peer rejected",
                    });
                }
                HandshakeOutput::TimedOut => {
                    self.pending.remove(&src);
                    return Err(Error::InvalidField {
                        what: "hs timeout",
                        reason: "listener",
                    });
                }
            }
        }

        Ok(())
    }

    fn tick_pending(&mut self) {
        let mut to_remove = Vec::new();
        for (addr, entry) in self.pending.iter_mut() {
            for outcome in entry.handshake.tick() {
                match outcome {
                    HandshakeOutput::Send(bytes) => {
                        self.outbound_queue
                            .entry(*addr)
                            .or_default()
                            .push_back(bytes);
                    }
                    HandshakeOutput::TimedOut => {
                        to_remove.push(*addr);
                    }
                    _ => {}
                }
            }
        }
        for addr in to_remove {
            self.pending.remove(&addr);
        }
    }

    fn drain_completed(&mut self) -> Option<Result<SrtSocket>> {
        let addr = self
            .pending
            .iter()
            .find(|(_, p)| {
                p.params.is_some()
                    && matches!(p.handshake.state(), ListenerHandshakeState::Connected)
            })
            .map(|(addr, _)| *addr)?;

        let entry = self.pending.remove(&addr)?;
        let peer_initial_seq = entry.peer_initial_seq;
        let our_initial_seq = self.config.initial_seq_number;
        let tsbpd_delay_ms = u64::from(self.config.latency_ms);
        let tsbpd_time_base = 0;
        let epoch = Instant::now();

        // Share the listener's Arc<UdpSocket> with the connection.
        let conn = SrtSocket::new(
            Arc::clone(&self.udp),
            addr,
            our_initial_seq,
            peer_initial_seq,
            tsbpd_time_base,
            tsbpd_delay_ms,
            epoch,
        );
        Some(Ok(conn))
    }

    async fn flush_for_peer(&mut self, addr: std::net::SocketAddr) -> Result<()> {
        if let Some(queue) = self.outbound_queue.get_mut(&addr) {
            while let Some(bytes) = queue.pop_front() {
                self.udp
                    .send_to(&bytes, addr)
                    .await
                    .map_err(|e| io_err("send_to", e))?;
            }
        }
        Ok(())
    }

    async fn flush_all(&mut self) -> Result<()> {
        let addrs: Vec<std::net::SocketAddr> = self.outbound_queue.keys().copied().collect();
        for addr in addrs {
            self.flush_for_peer(addr).await?;
        }
        Ok(())
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn io_err(context: &'static str, e: std::io::Error) -> Error {
    // The Error type uses `&'static str` for reasons, so we can't embed
    // dynamic error info. Log the context only.
    let _ = e;
    Error::InvalidField {
        what: context,
        reason: "io error",
    }
}

async fn resolve_one<A: tokio::net::ToSocketAddrs>(addr: A) -> Result<std::net::SocketAddr> {
    let mut addrs = tokio::net::lookup_host(addr)
        .await
        .map_err(|e| io_err("resolve", e))?;
    addrs.next().ok_or(Error::InvalidField {
        what: "resolve",
        reason: "no addrs",
    })
}

/// Extract the `initial_seq_number` from a handshake control packet's bytes.
/// Returns `None` if the bytes don't parse as a handshake control packet.
fn extract_isn_from_bytes(bytes: &[u8]) -> Option<u32> {
    let pkt = SrtPacket::parse(bytes).ok()?;
    match pkt {
        SrtPacket::Control(ControlPacket::Handshake(hp)) => Some(hp.initial_seq_number),
        _ => None,
    }
}
