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
//! runs a **background driver task** per connection that pumps socket RX →
//! engines → socket TX and, crucially, ticks the timers (retransmit, ACK,
//! NAK, TSBPD release) on a fixed [`tokio::time::interval`] — so loss recovery
//! keeps making progress even when the application is neither sending nor
//! receiving at that instant. The sans-IO core stays `no_std`; the adapter is
//! pure plumbing.
//!
//! # Why a background task
//!
//! SRT reliability is bidirectional and continuous: a receiver that detects a
//! gap emits a NAK, and the *sender* must react to that NAK by retransmitting
//! — long after the application handed it the original payload. A purely
//! pull-based `send`/`recv` (one that only advances the protocol while the
//! app is blocked inside a call) deadlocks the moment a fire-and-forget sender
//! stops calling `send`: the inbound NAK is never drained and the lost packet
//! is never resent. The driver task decouples protocol progress from
//! application call timing: [`SrtSocket::send`] enqueues a payload and returns
//! immediately, [`SrtSocket::recv`] awaits a delivered payload, and the task
//! in between runs the select loop (RX / app-send / periodic tick) forever
//! until the peer shuts down or the [`SrtSocket`] is dropped.
//!
//! # Structure
//!
//! - [`SrtListener`] — binds a UDP port and accepts incoming SRT connections,
//!   returning a [`SrtSocket`] per connected peer.
//! - [`SrtSocket`] — a handle to an established SRT connection (caller or
//!   listener role) with async [`send`](SrtSocket::send) and
//!   [`recv`](SrtSocket::recv) for application payloads; the actual protocol
//!   runs on the background driver task the handle owns.
//!
//! # Feature gate
//!
//! Only available with `features = ["tokio"]` (implies `std`). Without the
//! `tokio` feature, the crate stays `no_std`+`alloc` and nothing in this
//! module is compiled.

use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;

use alloc::collections::VecDeque;
use core::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::arq::{Receiver as ArqReceiver, Sender as ArqSender};
use crate::caller::{CallerHandshake, CallerHandshakeState};
use crate::error::{Error, Result};
use crate::handshake_sm::{HandshakeConfig, HandshakeOutput, derive_cookie};
use crate::listener::{ListenerHandshake, ListenerHandshakeState};
use crate::livecc::{LiveCC, MaxBwConfig};
use crate::packet::misc::KeepAlivePacket;
use crate::packet::{ControlPacket, DataPacket, SrtPacket};
use crate::tsbpd::TsbpdScheduler;

// ===========================================================================
// Constants
// ===========================================================================

/// Maximum UDP datagram size.
const MAX_DATAGRAM: usize = 1500;
/// Interval on which the driver task ticks the timer-based engine work (ACK,
/// NAK, retransmit, TSBPD release). Small enough that loss recovery is prompt
/// on a low-latency link, large enough not to busy-spin.
const TICK_INTERVAL_MS: u64 = 2;
/// Default TSBPD drift (zero when no estimate available).
const DEFAULT_DRIFT_US: u64 = 0;
/// TLPKTDROP (`draft-sharabayko-srt-01` §4.6) is **disabled** in this
/// adapter: it exists to *discard* packets that could not be recovered in
/// time, which is the exact opposite of the reliable, lossless in-order
/// delivery `SrtSocket::recv` promises. With it off, the TSBPD scheduler
/// waits for the ARQ layer's NAK-driven retransmission instead of skipping a
/// gap — so every payload is delivered, in order. (A live/latency-bounded
/// mode that re-enables drop is a future follow-up.)
const DEFAULT_TLPKT_DROP_ENABLED: bool = false;
/// Default max bandwidth (1 Gbps).
const DEFAULT_MAX_BW: MaxBwConfig = MaxBwConfig::Set(125_000_000);
/// Handshake timeout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

// ===========================================================================
// Outbound queue
// ===========================================================================

/// One datagram queued for transmission, plus whether LiveCC packet pacing
/// (`PKT_SND_PERIOD`, `draft-sharabayko-srt-01` §5.1) applies to it.
///
/// §5.1.2 paces DATA packets only — ACK/NAK/ACKACK/Keep-Alive control
/// feedback must never be throttled behind the pacing delay, or loss
/// recovery (which rides on that same control traffic) stalls along with
/// it.
#[derive(Debug)]
struct OutboundPacket {
    bytes: Vec<u8>,
    /// `true` for an (original or retransmitted) DATA packet — the only
    /// kind LiveCC pacing applies to.
    is_data: bool,
}

impl OutboundPacket {
    fn data(bytes: Vec<u8>) -> Self {
        OutboundPacket {
            bytes,
            is_data: true,
        }
    }

    fn control(bytes: Vec<u8>) -> Self {
        OutboundPacket {
            bytes,
            is_data: false,
        }
    }
}

// ===========================================================================
// SrtSocket — a handle to an established SRT connection
// ===========================================================================

/// A handle to an established SRT connection over UDP.
///
/// Created by [`SrtSocket::connect`] (caller role) or
/// [`SrtListener::accept`] (listener role). The protocol itself runs on a
/// background [`tokio`] task the handle owns; [`send`](Self::send) enqueues an
/// application payload and [`recv`](Self::recv) awaits a delivered one.
/// Dropping the handle aborts the driver task.
#[derive(Debug)]
pub struct SrtSocket {
    peer_addr: std::net::SocketAddr,
    /// Application payloads flowing to the driver task for transmission.
    to_driver: mpsc::UnboundedSender<Vec<u8>>,
    /// TSBPD/ARQ-delivered payloads flowing back from the driver task.
    from_driver: mpsc::UnboundedReceiver<Vec<u8>>,
    /// The driver task; aborted on drop.
    driver: Option<tokio::task::JoinHandle<()>>,
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
                            HandshakeOutput::Connected(params) => {
                                // The peer's ISN (seeds ARQ/TSBPD sequence
                                // tracking) is carried in the handshake bytes
                                // we just fed; the peer's SRT Socket ID (the
                                // wire `dest_socket_id` for every outgoing
                                // packet) is the negotiated
                                // `params.peer_socket_id` — the two are
                                // unrelated values (§3).
                                let peer_isn = extract_isn_from_bytes(bytes).unwrap_or(0);
                                let epoch = Instant::now();
                                let tsbpd_delay_ms = u64::from(config.latency_ms);
                                let tsbpd_time_base = 0u64;
                                let conn = SrtSocket::spawn(
                                    socket,
                                    peer,
                                    config.initial_seq_number,
                                    peer_isn,
                                    params.peer_socket_id,
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

    /// Build the engine state, spawn its background driver task, and return
    /// the [`SrtSocket`] handle wired to it.
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        udp: Arc<UdpSocket>,
        peer_addr: std::net::SocketAddr,
        our_initial_seq: u32,
        peer_initial_seq: u32,
        peer_socket_id: u32,
        tsbpd_time_base: u64,
        tsbpd_delay_ms: u64,
        epoch: Instant,
    ) -> Self {
        let (to_driver, app_out) = mpsc::unbounded_channel::<Vec<u8>>();
        let (deliver, from_driver) = mpsc::unbounded_channel::<Vec<u8>>();

        let driver = Driver {
            udp,
            peer_addr,
            peer_socket_id,
            // `dest_socket_id` on every outgoing DATA/ACKACK/NAK/ACK packet
            // must be the peer's negotiated SRT Socket ID, not its ISN — the
            // two are unrelated values (§3).
            sender: ArqSender::new(peer_socket_id),
            receiver: ArqReceiver::new(peer_socket_id, peer_initial_seq),
            tsbpd: TsbpdScheduler::new(
                peer_initial_seq,
                tsbpd_time_base,
                tsbpd_delay_ms,
                DEFAULT_DRIFT_US,
                DEFAULT_TLPKT_DROP_ENABLED,
                None,
            ),
            livecc: LiveCC::new(DEFAULT_MAX_BW),
            next_message_number: 1,
            next_send_seq: our_initial_seq,
            epoch,
            staged: std::collections::BTreeMap::new(),
            outbound: VecDeque::new(),
            deliver,
            peer_shutdown: false,
        };

        let handle = tokio::spawn(driver.run(app_out));

        SrtSocket {
            peer_addr,
            to_driver,
            from_driver,
            driver: Some(handle),
        }
    }

    /// Enqueue a payload for transmission to the peer.
    ///
    /// Returns immediately once the payload is handed to the driver task —
    /// actual transmission, ACK/NAK handling, and retransmission all happen
    /// on that task. Fails only if the driver task has stopped (peer shut
    /// down or connection error).
    pub async fn send(&mut self, payload: &[u8]) -> Result<()> {
        self.to_driver
            .send(payload.to_vec())
            .map_err(|_| Error::Io {
                kind: std::io::ErrorKind::BrokenPipe,
                context: "send",
            })
    }

    /// The peer's socket address.
    pub fn peer_addr(&self) -> std::net::SocketAddr {
        self.peer_addr
    }

    /// Receive the next payload, waiting until one is available.
    /// Returns `None` once the peer has shut down (or the driver task has
    /// stopped) and no further payloads will arrive.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(self.from_driver.recv().await)
    }
}

impl Drop for SrtSocket {
    fn drop(&mut self) {
        if let Some(handle) = self.driver.take() {
            handle.abort();
        }
    }
}

// ===========================================================================
// Driver — the per-connection background task
// ===========================================================================

/// The engine state driven by one connection's background task. Owns the
/// socket, the sans-IO ARQ/TSBPD/LiveCC engines, and the outbound queue; runs
/// the RX / app-send / periodic-tick select loop in [`Driver::run`].
struct Driver {
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

    next_message_number: u32,
    next_send_seq: u32,

    // Wall-clock epoch for `now: Duration`.
    epoch: Instant,

    // Staging: seq → payload bytes, released to `deliver` by TSBPD/ARQ.
    staged: std::collections::BTreeMap<u32, Vec<u8>>,

    // Outbound datagram queue (data paced, control not).
    outbound: VecDeque<OutboundPacket>,

    // Delivered payloads flowing back to the application handle.
    deliver: mpsc::UnboundedSender<Vec<u8>>,

    peer_shutdown: bool,
}

impl Driver {
    /// The select loop: socket RX, application-send, and a periodic engine
    /// tick — the tick arm is what keeps retransmit/ACK/NAK progressing when
    /// neither peer is actively sending application data.
    async fn run(mut self, mut app_out: mpsc::UnboundedReceiver<Vec<u8>>) {
        // Clone the `Arc` so the RX future borrows a *local*, leaving the
        // other select arms free to borrow `self` mutably.
        let udp = Arc::clone(&self.udp);
        let mut buf = [0u8; MAX_DATAGRAM];
        let mut ticker = tokio::time::interval(Duration::from_millis(TICK_INTERVAL_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut app_open = true;

        loop {
            tokio::select! {
                // Application handed us a payload to send.
                maybe = app_out.recv(), if app_open => {
                    match maybe {
                        Some(payload) => self.send_one(&payload),
                        None => app_open = false, // handle dropped its sender.
                    }
                }
                // A datagram arrived from the network.
                r = udp.recv_from(&mut buf) => {
                    match r {
                        Ok((len, src)) if src == self.peer_addr => {
                            // A malformed/foreign datagram is ignored, not fatal.
                            let _ = self.ingress(&buf[..len]);
                        }
                        Ok(_) => {} // datagram from another peer; ignore.
                        Err(_) => break, // socket error — end the task.
                    }
                }
                // Periodic timers: retransmit, ACK, NAK, TSBPD release.
                _ = ticker.tick() => {
                    self.tick_engines();
                }
            }

            if self.flush_outbound().await.is_err() {
                break;
            }
            if self.peer_shutdown {
                break;
            }
        }
        // Dropping `self.deliver` here closes the channel, so the handle's
        // `recv` returns `None` (clean shutdown / task ended).
    }

    fn send_one(&mut self, payload: &[u8]) {
        let now = self.elapsed();
        self.livecc.on_data_packet(payload.len() as u64);
        let bytes = self
            .sender
            .on_data(self.next_send_seq, self.next_message_number, payload, now);
        self.next_send_seq = self.next_send_seq.wrapping_add(1);
        self.next_message_number = self.next_message_number.wrapping_add(1);
        self.outbound.push_back(OutboundPacket::data(bytes));
    }

    fn ingress(&mut self, bytes: &[u8]) -> Result<()> {
        let now = self.elapsed();
        let packet = SrtPacket::parse(bytes)?;

        match packet {
            SrtPacket::Data(d) => {
                // The ARQ receiver drives *reliability* only: loss detection
                // and the resulting NAK (rules 4, 14) plus the ACK point.
                // Application delivery is the TSBPD scheduler's job — it is
                // the single in-order delivery authority (see below), so its
                // `outcome.delivered` is intentionally NOT used to deliver
                // here. Running both cursors over one `staged` map races
                // them and reorders retransmitted packets.
                let outcome = self.receiver.feed_data(d.seq_number, now);
                if let Some(nak_bytes) = outcome.nak {
                    self.outbound.push_back(OutboundPacket::control(nak_bytes));
                }

                self.staged
                    .entry(d.seq_number)
                    .or_insert_with(|| d.data.to_vec());

                // TSBPD is the sole delivery cursor: it releases packets in
                // strict sequence order, waiting for a NAK-recovered gap to
                // be filled rather than skipping it (TLPKTDROP disabled — see
                // `DEFAULT_TLPKT_DROP_ENABLED`).
                let tsbpd_out = self.tsbpd.feed_data(d.seq_number, d.timestamp, now);
                for &seq in &tsbpd_out.delivered {
                    if let Some(payload) = self.staged.remove(&seq) {
                        let _ = self.deliver.send(payload);
                    }
                }
            }
            SrtPacket::Control(ref c) => match c {
                ControlPacket::Ack(ack) => {
                    if let Some(ackack_bytes) = self.sender.on_ack(ack, now) {
                        self.outbound
                            .push_back(OutboundPacket::control(ackack_bytes));
                    }
                }
                ControlPacket::Nak(nak) => {
                    // Record the reported loss; the next `tick_engines`
                    // (or this cycle's, if a tick fired) drains the
                    // retransmit queue — retransmits are queued ahead of any
                    // new first-time data (rules 5, 15, 16).
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
                    self.outbound.push_back(OutboundPacket::control(buf));
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

        // Retransmitted DATA packets first (rules 5, 15, 16, 18): they are
        // drained from the NAK-populated loss list and queued *before* any
        // new first-time data appended later this cycle, reproducing the
        // sans-IO engine's "loss list before first transmission" priority
        // (see `arq::sender`'s module doc). Feed LiveCC the same as a
        // first-time send (`specs/rules/srt-livecc.md` §5.1.2, L3216-3217:
        // "original or retransmitted") and tag them `is_data` so pacing
        // applies.
        for bytes in self.sender.tick(now) {
            if let Ok(dp) = DataPacket::parse(&bytes) {
                self.livecc.on_data_packet(dp.data.len() as u64);
            }
            self.outbound.push_back(OutboundPacket::data(bytes));
        }

        // Periodic ACK/NAK (rules 11, 12, 21, 22): control feedback, never
        // paced.
        for bytes in self.receiver.tick(now) {
            self.outbound.push_back(OutboundPacket::control(bytes));
        }

        let tsbpd_out = self.tsbpd.tick(now);
        for &seq in &tsbpd_out.delivered {
            if let Some(payload) = self.staged.remove(&seq) {
                let _ = self.deliver.send(payload);
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
        while let Some(item) = self.outbound.pop_front() {
            // `specs/rules/srt-livecc.md` §5.1.2: `PKT_SND_PERIOD` paces DATA
            // packets only — control feedback (ACK/NAK/ACKACK/Keep-Alive)
            // must go out immediately, or loss recovery (which rides on
            // that same control traffic) would be throttled right along
            // with the data it is meant to unblock.
            if item.is_data {
                let period = self.livecc.on_ack_received();
                if period > Duration::ZERO {
                    tokio::time::sleep(period).await;
                }
            }
            self.udp
                .send_to(&item.bytes, self.peer_addr)
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
    /// Per-listener secret input to [`derive_cookie`] (`draft-sharabayko-srt-01`
    /// §4.3.1.1: "a cookie that is crafted based on host, port and current
    /// time"). Generated once at [`SrtListener::bind`] so every SYN Cookie
    /// this listener hands out is per-instance, not a fixed shared value a
    /// remote peer could pre-compute and replay against a different listener.
    cookie_secret: u64,
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
            cookie_secret: random_u64(),
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
            // §4.3.1.1: "a cookie that is crafted based on host, port and
            // current time with 1 minute accuracy" — `derive_cookie` mixes
            // exactly those inputs (`crate::handshake_sm`'s existing,
            // documented derivation), keyed by this listener's own secret so
            // two listeners never hand out the same cookie for the same
            // peer/time bucket.
            let peer_key = addr_to_u64(&src);
            let time_bucket = unix_time_bucket();
            let syn_cookie = derive_cookie(peer_key, time_bucket, self.cookie_secret);
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
        // The peer's negotiated SRT Socket ID (distinct from its ISN above)
        // — `drain_completed` only reaches entries filtered to
        // `ListenerHandshakeState::Connected`, so `negotiated()` is always
        // `Some` here.
        let peer_socket_id = entry
            .handshake
            .negotiated()
            .expect("filtered to Connected state")
            .peer_socket_id;
        let our_initial_seq = self.config.initial_seq_number;
        let tsbpd_delay_ms = u64::from(self.config.latency_ms);
        let tsbpd_time_base = 0;
        let epoch = Instant::now();

        // Share the listener's Arc<UdpSocket> with the connection's driver.
        let conn = SrtSocket::spawn(
            Arc::clone(&self.udp),
            addr,
            our_initial_seq,
            peer_initial_seq,
            peer_socket_id,
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

/// Maps an OS I/O failure to a structured [`Error::Io`], preserving the
/// `std::io::ErrorKind` (bind failures are then distinguishable from
/// mid-connection resets, etc.) and the call site that failed. `std::io::Error`
/// itself is not `Clone`/`Eq` (this crate's [`Error`] derives both), so only
/// its `kind()` is kept — see the `S4` release-audit finding.
fn io_err(context: &'static str, e: std::io::Error) -> Error {
    Error::Io {
        kind: e.kind(),
        context,
    }
}

/// Mixes a [`std::net::SocketAddr`] into a `u64` for use as `derive_cookie`'s
/// `peer_key` input (§4.3.1.1: the cookie is "crafted based on host,
/// port..."). Not a spec-defined algorithm — any stable, well-distributed
/// mix of the peer's address is sufficient here.
fn addr_to_u64(addr: &std::net::SocketAddr) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    addr.hash(&mut hasher);
    hasher.finish()
}

/// The current UNIX time, bucketed to 1-minute accuracy — the `time_bucket`
/// input `derive_cookie` expects (§4.3.1.1: "...and current time with 1
/// minute accuracy").
fn unix_time_bucket() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (secs / 60) as u32
}

/// A per-process/per-listener random `u64`, used as `derive_cookie`'s
/// `secret` input. Sourced from `std::collections::hash_map::RandomState`
/// (the standard library's own OS-seeded randomness, already used internally
/// for `HashMap` DoS resistance) rather than pulling in a `rand` dependency
/// for one seed value.
fn random_u64() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
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
