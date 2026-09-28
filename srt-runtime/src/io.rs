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

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use alloc::collections::VecDeque;
use core::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::arq::seq::{seq_diff, seq_in_closed_range, seq_next};
use crate::arq::{Receiver as ArqReceiver, Sender as ArqSender};
use crate::caller::{CallerHandshake, CallerHandshakeState};
use crate::error::{Error, Result};
use crate::handshake_sm::{self, HandshakeConfig, HandshakeOutput, RejectionReason, derive_cookie};
use crate::listener::{ListenerHandshake, ListenerHandshakeState};
use crate::livecc::{LiveCC, MaxBwConfig};
use crate::packet::data::next_message_number;
use crate::packet::misc::KeepAlivePacket;
use crate::packet::{
    ControlPacket, DataPacket, DropReqPacket, EncryptionField, EncryptionKeyField,
    HandshakeExtensionFlags, HandshakeExtensions, HandshakePacket, SEQ_NUMBER_MASK, SrtPacket,
};
use crate::tsbpd::{TickOutcome, TsbpdScheduler};

// ===========================================================================
// Constants
// ===========================================================================

/// Maximum UDP datagram size.
const MAX_DATAGRAM: usize = 1500;

/// One [`RouteTable`] entry: the accepted connection's ingress channel, plus
/// the source address it was accepted from. libsrt demultiplexes its
/// multiplexed listener socket by Destination Socket ID, not source address
/// (draft §4.3.1; `CRcvQueue::worker` dispatches on it in `srtcore/core.cpp`)
/// — the address is kept here only to *check* it against the datagram that
/// claims this ID (see [`spawn_listener_routing_pump`]), so a spoofed ID from
/// the wrong peer cannot be injected into someone else's session.
#[derive(Debug, Clone)]
struct RouteEntry {
    addr: std::net::SocketAddr,
    tx: mpsc::Sender<Vec<u8>>,
    /// Shared with that connection's [`Driver`]/[`SrtSocket`], so a drop
    /// here (channel full) is counted where [`SrtSocket::stats`] can see it.
    stats: Arc<Counters>,
}

/// Shared per-socket demultiplexing table (issue #1029): maps an accepted
/// connection's own Socket ID to its [`RouteEntry`]. A [`SrtListener`]'s
/// single bound socket is used by every connection it has accepted, so —
/// unlike a [`SrtSocket::connect`]'s own dedicated, unshared socket — the
/// listener needs exactly one task reading the socket and routing each
/// datagram, never one `recv_from` call per connection racing another's on
/// the same socket (see the module doc on [`SrtListener`]).
type RouteTable = Arc<std::sync::Mutex<std::collections::HashMap<u32, RouteEntry>>>;

/// RAII guard that removes this connection's entry from the listener's shared
/// [`RouteTable`] when dropped. A stale route was a real leak: dropping a
/// listener-accepted [`SrtSocket`] runs `Drop::drop` → `handle.abort()`,
/// which cancels the driver task at its next await point and drops its
/// future (and everything the future owns) without ever reaching the
/// explicit cleanup that used to sit at the tail of [`Driver::run`]'s loop —
/// that code only ran on a *normal* return, never on abort. A `Drop` impl
/// runs either way, since it fires when this guard's owner (the future) is
/// dropped, not only when the function body finishes.
struct RouteCleanup(Option<(u32, RouteTable)>);

impl Drop for RouteCleanup {
    fn drop(&mut self) {
        if let Some((id, routes)) = self.0.take() {
            routes.lock().unwrap().remove(&id);
        }
    }
}

/// Aborts the wrapped background task when dropped. A [`SrtSocket::connect`]ed
/// caller's dedicated-socket forwarder task (`spawn_dedicated_socket_forwarder`)
/// holds its own `Arc<UdpSocket>` clone and loops on `recv_from` forever —
/// dropping the `Driver`'s own reference alone does not stop it, so without
/// this guard the forwarder keeps the local port bound indefinitely after the
/// `SrtSocket` is dropped, and a later attempt to bind that same port fails
/// with `AddrInUse` (item 3). `None` for a listener-accepted connection,
/// which has no per-connection forwarder to abort.
struct TaskGuard(Option<tokio::task::JoinHandle<()>>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

/// Capacity of a per-connection inbound-datagram channel (the dedicated
/// forwarder for a [`SrtSocket::connect`]ed caller, or a [`SrtListener`]'s
/// per-accepted-connection route) — bounded so a peer that floods faster than
/// the driver task's tick loop can drain does not grow this queue (and this
/// process's memory) without bound; excess datagrams are dropped, not
/// buffered (item 4).
const RX_CHANNEL_CAPACITY: usize = 4096;
/// Capacity of a [`SrtListener`]'s candidate-new-connection queue (datagrams
/// the routing pump could not match to any accepted connection). Bounded for
/// the same reason as [`RX_CHANNEL_CAPACITY`] — a handshake flood must not
/// grow this queue without bound even though [`MAX_PENDING`] already caps how
/// many of those candidates get to hold engine state.
const UNROUTED_CHANNEL_CAPACITY: usize = 1024;
/// Capacity of the delivered-payload channel from a connection's driver task
/// to its [`SrtSocket`] handle. Bounded so a slow/stalled application that
/// never calls [`SrtSocket::recv`] cannot make TSBPD-released payloads pile
/// up in memory without bound purely from network input — matching this
/// adapter's live (TLPKTDROP) delivery philosophy of dropping rather than
/// buffering forever (see [`DEFAULT_TLPKT_DROP_ENABLED`]).
const DELIVER_CHANNEL_CAPACITY: usize = 4096;

/// Interval on which the driver task ticks the timer-based engine work (ACK,
/// NAK, retransmit, TSBPD release). Small enough that loss recovery is prompt
/// on a low-latency link, large enough not to busy-spin.
const TICK_INTERVAL_MS: u64 = 2;
/// Default TSBPD drift (zero when no estimate available).
const DEFAULT_DRIFT_US: u64 = 0;
/// TLPKTDROP (`draft-sharabayko-srt-01` §4.6) is **enabled** in this
/// adapter: the default mode is live (latency-bounded) delivery, where a
/// packet that could not be recovered inside its play-time window is
/// discarded rather than stalling everything behind it. With it on, the
/// TSBPD scheduler releases later packets past an unrecoverable gap once the
/// next available packet reaches its play time (rule 21) instead of waiting
/// forever for a retransmission that may never come — one loss must not
/// freeze delivery or grow memory.
const DEFAULT_TLPKT_DROP_ENABLED: bool = true;
/// Default max bandwidth (1 Gbps).
const DEFAULT_MAX_BW: MaxBwConfig = MaxBwConfig::Set(125_000_000);
/// Handshake timeout.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Peer idle timeout: the longest silence — any packet, not just DATA — from
/// a connected peer before the connection is torn down. Matches libsrt's
/// `SRTO_PEERIDLETIMEO` default of 5000 ms (SRT socket options reference,
/// `srtcore/core.cpp`'s `CSrtConfig` default). Without this, an accepted
/// connection whose peer vanished without a Shutdown (crashed, network path
/// gone) never ends on its own — `Driver::run` would poll its dead `rx`
/// forever, so nothing ever removes its route-table entry or drops its
/// `SrtSocket` handle.
const PEER_IDLE_TIMEOUT: Duration = Duration::from_millis(5000);
/// Upper bound on concurrently in-progress listener handshakes. Every new
/// source address that sends an INDUCTION allocates a [`PendingListener`]
/// entry; without a cap, a flood of (possibly spoofed) handshake packets from
/// many sources exhausts memory before any of them times out.
const MAX_PENDING: usize = 1024;
/// How often [`SrtListener::accept`] drives pending-handshake expiry and
/// retransmits, independent of receive activity.
const PENDING_TICK_INTERVAL: Duration = Duration::from_millis(100);

// ===========================================================================
// Outbound queue
// ===========================================================================
//
// Two separate queues, not one queue tagged per-item: §5.1.2 paces DATA
// packets only — ACK/NAK/ACKACK/Keep-Alive control feedback must never be
// throttled behind the pacing delay, or loss recovery (which rides on that
// same control traffic) stalls along with it. A single FIFO queue with an
// `is_data` tag on each item got this half right (pacing never delayed a
// control packet that had already reached the *front*) but not the other
// half: `flush_outbound` stopped at the first not-yet-due DATA packet and
// never looked past it, so a control packet enqueued *behind* one — the
// common case, since `tick_engines` queues retransmits before this cycle's
// ACK/NAK — sat there just as throttled as if pacing applied to it too. Two
// queues make that structurally impossible: control is always drained in
// full before data pacing is even considered.

/// Per-connection counters for datagrams a bounded internal channel dropped
/// because it was full (item 4) — this adapter's own internal backpressure,
/// distinct from wire-level loss ARQ/TLPKTDROP already account for. Shared
/// (via `Arc`) between a [`Driver`] and the [`SrtSocket`] handle so
/// [`SrtSocket::stats`] reads live counts, not a snapshot frozen at spawn.
#[derive(Debug, Default)]
struct Counters {
    rx_dropped: AtomicU64,
    deliver_dropped: AtomicU64,
}

/// A snapshot of one connection's `Counters`, returned by
/// [`SrtSocket::stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SocketStats {
    /// Inbound datagrams dropped because the per-connection ingress channel
    /// was full (the peer sent faster than this connection's driver task
    /// could drain).
    pub rx_dropped: u64,
    /// TSBPD-released payloads dropped because the application wasn't
    /// calling [`SrtSocket::recv`] fast enough to keep the delivery channel
    /// from filling.
    pub deliver_dropped: u64,
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
    from_driver: mpsc::Receiver<Vec<u8>>,
    /// The driver task; aborted on drop.
    driver: Option<tokio::task::JoinHandle<()>>,
    stats: Arc<Counters>,
}

impl SrtSocket {
    /// Connect to a remote SRT peer as a Caller.
    ///
    /// # Encryption
    /// A [`HandshakeConfig`] with `crypto` set is refused with
    /// [`Error::InvalidField`] before any network I/O: this adapter's data
    /// path does not encrypt, so an encrypted connection must not be
    /// silently downgraded to plaintext. The sans-IO engines still negotiate
    /// keys; the adapter will support encryption once its data path does.
    pub async fn connect<A: tokio::net::ToSocketAddrs>(
        remote_addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        refuse_crypto(&config)?;
        let local = "0.0.0.0:0".parse::<std::net::SocketAddr>().unwrap();
        Self::connect_from(local, remote_addr, config).await
    }

    /// Connect from a specific local address.
    ///
    /// # Encryption
    /// Like [`Self::connect`], refuses a `crypto`-enabled
    /// [`HandshakeConfig`] with [`Error::InvalidField`] before any I/O.
    pub async fn connect_from<A: tokio::net::ToSocketAddrs>(
        local_addr: std::net::SocketAddr,
        remote_addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        refuse_crypto(&config)?;
        let socket = UdpSocket::bind(local_addr)
            .await
            .map_err(|e| io_err("bind", e))?;
        let peer = resolve_one(remote_addr).await?;
        let socket = Arc::new(socket);

        // Both this Caller's own Socket ID and its ISN must be freshly
        // generated per connection, not carried over from `config` (whose
        // `initial_seq_number` a caller may leave at its `0` default) —
        // `draft-sharabayko-srt-01` §3/§4.3.1.1 expects both random, and a
        // fixed or predictable pair lets an off-path party that knows the
        // 4-tuple guess a live connection's wire identifiers outright.
        let own_socket_id = random_socket_id();
        let mut config = config;
        config.initial_seq_number = random_isn();
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
                Ok(Ok((len, src))) => {
                    // A packet from anyone but the peer we're connecting to
                    // (some unrelated traffic that happened to land on this
                    // freshly bound port) must not fail the connect — ignore
                    // it and keep waiting for the real peer (item 5).
                    if src != peer {
                        continue;
                    }
                    let bytes = &buf[..len];
                    // Likewise, a packet that isn't a well-formed Handshake
                    // control packet right now (a stray Keep-Alive, a
                    // resent/duplicate reply, garbage) must not fail the
                    // connect either — only a genuine handshake rejection or
                    // timeout does that.
                    let Ok(outcomes) = hs.feed_bytes(bytes) else {
                        continue;
                    };

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
                                //
                                // `require_peer_isn` errors (rather than
                                // defaulting to 0) if the very bytes that just
                                // drove the handshake state machine to
                                // `Connected` fail to re-parse as a Handshake
                                // control packet — an internal inconsistency
                                // between this extraction shim and
                                // `CallerHandshake`. A silent `0` fallback
                                // would be indistinguishable from a genuine
                                // ISN of 0 and would seed ARQ/TSBPD sequence
                                // tracking wrong for the life of the
                                // connection, surfacing only much later as
                                // inexplicable loss/reordering.
                                let peer_isn = require_peer_isn(bytes)?;
                                let epoch = Instant::now();
                                let tsbpd_delay_ms = u64::from(config.latency_ms);
                                let tsbpd_time_base = 0u64;
                                // This socket is exclusively this connection's
                                // own (bound fresh in `connect_from`, never
                                // shared) — no demux table needed, just a
                                // forwarder task unifying it with the same
                                // `Driver::rx` path a listener-accepted
                                // connection uses (issue #1029).
                                let stats = Arc::new(Counters::default());
                                let (rx, forwarder) = spawn_dedicated_socket_forwarder(
                                    Arc::clone(&socket),
                                    peer,
                                    Arc::clone(&stats),
                                );
                                let conn = SrtSocket::spawn(
                                    socket,
                                    peer,
                                    rx,
                                    stats,
                                    RouteCleanup(None),
                                    TaskGuard(Some(forwarder)),
                                    config.initial_seq_number,
                                    peer_isn,
                                    params.peer_socket_id,
                                    tsbpd_time_base,
                                    tsbpd_delay_ms,
                                    epoch,
                                    config.max_flow_window_size,
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
        rx: mpsc::Receiver<Vec<u8>>,
        stats: Arc<Counters>,
        route_cleanup: RouteCleanup,
        forwarder_guard: TaskGuard,
        our_initial_seq: u32,
        peer_initial_seq: u32,
        peer_socket_id: u32,
        tsbpd_time_base: u64,
        tsbpd_delay_ms: u64,
        epoch: Instant,
        max_flow_window: u32,
    ) -> Self {
        let (to_driver, app_out) = mpsc::unbounded_channel::<Vec<u8>>();
        let (deliver, from_driver) = mpsc::channel(DELIVER_CHANNEL_CAPACITY);

        let driver = Driver {
            udp,
            peer_addr,
            peer_socket_id,
            stats: Arc::clone(&stats),
            rx,
            _route_cleanup: route_cleanup,
            _forwarder_guard: forwarder_guard,
            last_rx_at: Instant::now(),
            // `dest_socket_id` on every outgoing DATA/ACKACK/NAK/ACK packet
            // must be the peer's negotiated SRT Socket ID, not its ISN — the
            // two are unrelated values (§3).
            sender: ArqSender::new(peer_socket_id),
            receiver: ArqReceiver::new(peer_socket_id, peer_initial_seq, max_flow_window),
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
            max_flow_window,
            outbound_data: VecDeque::new(),
            outbound_control: VecDeque::new(),
            next_data_send_at: None,
            deliver,
            peer_shutdown: false,
        };

        let handle = tokio::spawn(driver.run(app_out));

        SrtSocket {
            peer_addr,
            to_driver,
            from_driver,
            driver: Some(handle),
            stats,
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

    /// A snapshot of this connection's internal-backpressure drop counters
    /// (item 4) — datagrams or delivered payloads this adapter's own bounded
    /// channels dropped because they were full, not wire-level loss.
    pub fn stats(&self) -> SocketStats {
        SocketStats {
            rx_dropped: self.stats.rx_dropped.load(Ordering::Relaxed),
            deliver_dropped: self.stats.deliver_dropped.load(Ordering::Relaxed),
        }
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
    /// Shared with the [`SrtSocket`] handle (for [`SrtSocket::stats`]) and
    /// whichever task feeds `rx` (the dedicated forwarder, or the listener's
    /// routing pump via its `RouteEntry`), so a drop on either side of that
    /// channel is counted in the one place the application can read it.
    stats: Arc<Counters>,

    // Inbound datagrams for *this* connection only (issue #1029). Fed by
    // exactly one upstream demultiplexer: a per-connection forwarder task
    // reading a dedicated socket ([`SrtSocket::connect_from`]), or the
    // [`SrtListener`]'s single shared-socket routing pump. Never
    // `udp.recv_from` directly — that would race every other task reading
    // the same socket for whichever datagram arrives next, handing some of
    // this connection's own traffic to a different connection (or the
    // listener's own accept loop) at random. Bounded (item 4): a peer that
    // floods faster than this task's tick loop drains gets its excess
    // datagrams dropped, not buffered without bound.
    rx: mpsc::Receiver<Vec<u8>>,
    // For a listener-accepted connection: removes this connection's entry
    // from the shared route table on drop — including on `SrtSocket::Drop`'s
    // task abort, not only a normal `run()` return (see [`RouteCleanup`]).
    // Holds `None` for a `SrtSocket::connect`ed connection (no shared table).
    _route_cleanup: RouteCleanup,
    // For a `SrtSocket::connect`ed connection: aborts the dedicated-socket
    // forwarder task on drop, so it stops holding the local UDP port open
    // (see [`TaskGuard`]). `None` for a listener-accepted connection (no
    // per-connection forwarder — the listener's routing pump outlives it).
    _forwarder_guard: TaskGuard,
    // Absolute time the last datagram (of any kind, valid or not) was
    // received from the peer — the basis for the peer-idle timeout below.
    last_rx_at: Instant,

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

    // Negotiated maximum flow window (§3.2.1): the cap on how far ahead of
    // the delivery cursor a packet may be staged.
    max_flow_window: u32,

    // Outbound datagram queues — separate so control can never queue behind
    // a not-yet-due paced DATA packet (see the module doc above).
    outbound_data: VecDeque<Vec<u8>>,
    outbound_control: VecDeque<Vec<u8>>,

    // Token-bucket pacing schedule (issue #1061): the earliest time the next
    // DATA packet may go out. `None` means no DATA packet is currently
    // pacing-blocked (the queue is empty of DATA, or was last drained with
    // room to spare). Advanced by `LiveCC::on_ack_received()`'s period each
    // time a DATA packet is actually sent, *from whichever is later of the
    // previous schedule slot or now* — so a burst that arrives after this
    // connection was otherwise idle (RX/tick-bound) sends immediately
    // instead of being penalized for the gap, while a sustained flood is
    // still throttled to the configured rate. This replaces a real
    // `tokio::time::sleep` per DATA packet in `flush_outbound`, which — even
    // at a sub-millisecond computed period — actually blocked for
    // tokio's real timer resolution (>= ~1 ms), capping throughput at
    // roughly 1000 pkt/s regardless of the configured MAX_BW, and blocked
    // the same `run()` iteration's RX processing while it slept.
    next_data_send_at: Option<Instant>,

    // Delivered payloads flowing back to the application handle. Bounded
    // (item 4): see [`DELIVER_CHANNEL_CAPACITY`].
    deliver: mpsc::Sender<Vec<u8>>,

    peer_shutdown: bool,
}

impl Driver {
    /// The select loop: socket RX, application-send, and a periodic engine
    /// tick — the tick arm is what keeps retransmit/ACK/NAK progressing when
    /// neither peer is actively sending application data.
    async fn run(mut self, mut app_out: mpsc::UnboundedReceiver<Vec<u8>>) {
        let mut ticker = tokio::time::interval(Duration::from_millis(TICK_INTERVAL_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut app_open = true;

        // Set when the application handle is gone (its `to_driver` sender was
        // dropped): the loop makes one final flush pass and then exits, so a
        // dropped [`SrtSocket`] does not leave a driver task parked forever on
        // the socket/timer arms — which would keep a current-thread runtime
        // from shutting down. (`SrtSocket::Drop` also aborts the task; this is
        // the cooperative path that does not rely on abort-during-shutdown.)
        let mut shutting_down = false;

        loop {
            // How long until the DATA packet currently at the front of the
            // outbound queue (if any) is allowed to go out. `None` disables
            // this `select!` arm entirely — pacing never blocks RX/app/tick
            // when nothing is pacing-throttled (issue #1061).
            let pacing_wait = self.next_paced_send_wait();

            tokio::select! {
                // Application handed us a payload to send.
                maybe = app_out.recv(), if app_open => {
                    match maybe {
                        Some(payload) => self.send_one(&payload),
                        None => {
                            // Handle dropped its sender — no more app data
                            // will ever arrive. Stop polling this (now
                            // permanently `Ready(None)`) arm and tear down.
                            app_open = false;
                            shutting_down = true;
                        }
                    }
                }
                // A datagram for this connection, already demultiplexed by
                // the upstream forwarder/router (issue #1029) — never a
                // shared `udp.recv_from` racing another task for the same
                // socket.
                maybe = self.rx.recv() => {
                    match maybe {
                        Some(bytes) => {
                            // A malformed datagram is ignored, not fatal.
                            let _ = self.ingress(&bytes);
                        }
                        None => break, // upstream demux/forwarder is gone.
                    }
                }
                // Periodic timers: retransmit, ACK, NAK, TSBPD release.
                _ = ticker.tick() => {
                    self.tick_engines();
                }
                // The pacing schedule caught up to the front of the queue —
                // wake up to flush it. A plain `select!` arm, not a serial
                // `.await` after the fact, so RX/app/tick keep being
                // serviced for the whole wait rather than only after it.
                _ = tokio::time::sleep(pacing_wait.unwrap_or(Duration::ZERO)), if pacing_wait.is_some() => {}
            }

            if self.flush_outbound().await.is_err() {
                break;
            }
            if Instant::now().duration_since(self.last_rx_at) >= PEER_IDLE_TIMEOUT {
                // libsrt's default `SRTO_PEERIDLETIMEO`: no packet at all
                // (not just DATA) from the peer for this long means the
                // connection is broken, not merely quiet — a live SRT peer
                // always sends at least periodic Keep-Alives.
                break;
            }
            if self.peer_shutdown || shutting_down {
                break;
            }
        }
        // The route-table and forwarder-task cleanup this used to do inline
        // now happens in `_route_cleanup`/`_forwarder_guard`'s `Drop` impls,
        // which run here too (falling off the end of this function drops
        // `self`) as well as on `SrtSocket::Drop`'s task abort — the case
        // this inline code used to miss entirely.
        //
        // Dropping `self.deliver` here closes the channel, so the handle's
        // `recv` returns `None` (clean shutdown / task ended).
    }

    fn send_one(&mut self, payload: &[u8]) {
        let now = self.elapsed();
        self.livecc.on_data_packet(payload.len() as u64);
        let bytes = self
            .sender
            .on_data(self.next_send_seq, self.next_message_number, payload, now);
        // Wrap at the wire field's own bit width (31 bits for the sequence
        // number, 26 for the message number — `draft-sharabayko-srt-01`
        // §3.1), not at `u32::MAX`: a plain `wrapping_add(1)` let both
        // counters walk past their field width (~20 h of continuous sending
        // for the sequence number, or immediately with a high initial value;
        // ~67 million messages for the message number), after which every
        // subsequent `DataPacket::serialize_into` returned
        // `Error::FieldTooWide` and panicked the `.expect("buffer sized from
        // serialized_len")` call sites in `arq::sender` that assumed only a
        // too-small buffer could fail (issue #1062).
        self.next_send_seq = seq_next(self.next_send_seq);
        self.next_message_number = next_message_number(self.next_message_number);
        self.outbound_data.push_back(bytes);
    }

    fn ingress(&mut self, bytes: &[u8]) -> Result<()> {
        // Any datagram from the peer — even one that fails to parse below —
        // is evidence the peer is still there, resetting the idle timeout.
        self.last_rx_at = Instant::now();
        let now = self.elapsed();
        let packet = SrtPacket::parse(bytes)?;

        match packet {
            SrtPacket::Data(d) => {
                // The adapter never decrypts, so a ciphertext packet must not
                // reach the application (§3.1 `KK`, §6): drop it before ARQ
                // or TSBPD see it (acknowledging data we can never deliver
                // would be worse than dropping it).
                if d.key_flag != EncryptionKeyField::NotEncrypted {
                    return Ok(());
                }

                // Flow-window overflow guard (§3.2.1): a packet further
                // ahead of the delivery cursor than the negotiated maximum
                // flow window can never be released in this connection's
                // lifetime — staging it would let an unrecoverable gap grow
                // memory without bound, so drop it instead.
                if seq_diff(d.seq_number, self.tsbpd.next_release()) > self.max_flow_window as i32 {
                    return Ok(());
                }

                // The ARQ receiver drives *reliability* only: loss detection
                // and the resulting NAK (rules 4, 14) plus the ACK point.
                // Application delivery is the TSBPD scheduler's job — it is
                // the single in-order delivery authority (see below), so its
                // `outcome.delivered` is intentionally NOT used to deliver
                // here. Running both cursors over one `staged` map races
                // them and reorders retransmitted packets.
                let outcome = self.receiver.feed_data(d.seq_number, now);
                if let Some(nak_bytes) = outcome.nak {
                    self.outbound_control.push_back(nak_bytes);
                }

                self.staged
                    .entry(d.seq_number)
                    .or_insert_with(|| d.data.to_vec());

                // TSBPD is the sole delivery cursor: it releases packets in
                // strict sequence order, waiting for a NAK-recovered gap to
                // be filled — until the next available packet reaches its
                // play time, at which point live mode skips the unrecoverable
                // gap and delivers that packet (see
                // `DEFAULT_TLPKT_DROP_ENABLED`).
                let tsbpd_out = self.tsbpd.feed_data(d.seq_number, d.timestamp, now);
                self.release(&tsbpd_out);
            }
            SrtPacket::Control(ref c) => match c {
                ControlPacket::Ack(ack) => {
                    if let Some(ackack_bytes) = self.sender.on_ack(ack, now) {
                        self.outbound_control.push_back(ackack_bytes);
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
                ControlPacket::DropReq(d) => {
                    self.handle_dropreq(d, now);
                }
                ControlPacket::KeepAlive(_) => {
                    let pkt = ControlPacket::KeepAlive(KeepAlivePacket {
                        timestamp: self.elapsed_us(),
                        dest_socket_id: self.peer_socket_id,
                        libsrt_pad: true,
                    });
                    let mut buf = vec![0u8; pkt.serialized_len()];
                    let _ = pkt.serialize_into(&mut buf);
                    self.outbound_control.push_back(buf);
                }
                ControlPacket::Shutdown(_) => {
                    self.peer_shutdown = true;
                }
                _ => {}
            },
        }

        Ok(())
    }

    /// Handle a DROPREQ (§3.2.9): the peer announces it will never deliver
    /// sequence numbers `first..=last`. Tell the ARQ receiver to stop NAKing
    /// them and advance its ack point, tell the TSBPD scheduler to skip them
    /// for delivery purposes, discard anything already staged inside the
    /// range (the sender has given up on the message they belonged to), then
    /// drive one release pass so whatever the gap was blocking becomes
    /// deliverable immediately.
    fn handle_dropreq(&mut self, d: &DropReqPacket, now: Duration) {
        let first = d.first_seq & SEQ_NUMBER_MASK;
        let last = d.last_seq & SEQ_NUMBER_MASK;
        if seq_diff(last, first) < 0 {
            // `last` precedes `first` — malformed range; nothing sane to skip.
            return;
        }
        self.receiver.skip_range(first, last);
        self.tsbpd.skip_range(first, last);
        self.staged
            .retain(|&s, _| !seq_in_closed_range(s, first, last));

        let tsbpd_out = self.tsbpd.tick(now);
        self.release(&tsbpd_out);
    }

    /// Apply one TSBPD outcome to `staged` and the application channel:
    /// deliver the released payloads, and purge anything the scheduler dropped
    /// as too-late/skipped so a lost packet cannot leak its staged bytes.
    fn release(&mut self, outcome: &TickOutcome) {
        for &seq in &outcome.delivered {
            if let Some(payload) = self.staged.remove(&seq)
                && self.deliver.try_send(payload).is_err()
            {
                // Full (app isn't calling `recv` fast enough) or the handle
                // is gone; either way, drop rather than buffer without bound
                // (item 4) and count it.
                self.stats.deliver_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        for &seq in &outcome.dropped {
            self.staged.remove(&seq);
        }
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
            self.outbound_data.push_back(bytes);
        }

        // Periodic ACK/NAK (rules 11, 12, 21, 22): control feedback, never
        // paced.
        for bytes in self.receiver.tick(now) {
            self.outbound_control.push_back(bytes);
        }

        let tsbpd_out = self.tsbpd.tick(now);
        self.release(&tsbpd_out);
    }

    fn elapsed(&self) -> Duration {
        Instant::now().duration_since(self.epoch)
    }

    /// The Keep-Alive/ACKACK/etc. control-packet wire `Timestamp` — wraps
    /// modulo `2^32` past ~71.58 minutes, matching `arq::duration_to_wire_us`
    /// (issue #1063; a clamp here made every control packet after that point
    /// carry the exact same wire timestamp forever).
    fn elapsed_us(&self) -> u32 {
        crate::arq::duration_to_wire_us(self.elapsed())
    }

    /// How long until the DATA packet at the front of the outbound data
    /// queue (if any) is next allowed to go out — `None` if there is nothing
    /// pacing-blocked right now (the data queue is empty, or its front
    /// packet's slot has already arrived). Drives the `run()` loop's pacing
    /// `select!` arm (issue #1061): by construction, `flush_outbound` always
    /// drains everything currently due before returning, so a `Some` here is
    /// always a genuine future deadline, never one already past. Never
    /// consults `outbound_control` — control is never pacing-blocked (item 6:
    /// two separate queues, not one tagged queue, so control can never queue
    /// behind a not-yet-due DATA packet either).
    fn next_paced_send_wait(&self) -> Option<Duration> {
        if self.outbound_data.is_empty() {
            return None;
        }
        let deadline = self.next_data_send_at?;
        Some(deadline.saturating_duration_since(Instant::now()))
    }

    /// Send every outbound item that is due right now, in order, without
    /// blocking on the pacing delay itself (issue #1061).
    ///
    /// `specs/rules/srt-livecc.md` §5.1.2: `PKT_SND_PERIOD` paces DATA
    /// packets only — control feedback (ACK/NAK/ACKACK/Keep-Alive) must go
    /// out immediately, or loss recovery (which rides on that same control
    /// traffic) would be throttled right along with the data it is meant to
    /// unblock.
    ///
    /// DATA packets are scheduled against `next_data_send_at`, a token-bucket
    /// style deadline rather than a per-packet `tokio::time::sleep`: a real
    /// sleep, even for a sub-millisecond computed period, actually blocks for
    /// tokio's real timer resolution (documented as coarse — at least
    /// roughly a millisecond on this host) — capping throughput at roughly
    /// 1000 pkt/s regardless of the configured `MAX_BW`, and (since the old
    /// code awaited it serially inside this function, called from every
    /// `run()` iteration) stalling RX/app-send processing for that same
    /// window. The schedule advances from whichever is later of the
    /// previous deadline or now, so a connection that was genuinely idle
    /// (or RX/tick-bound, not pacing-bound) for a while does not have to
    /// "pay back" that gap — it sends immediately and only then resumes
    /// pacing at the configured rate — while a sustained flood is still
    /// throttled to it.
    async fn flush_outbound(&mut self) -> Result<()> {
        // Control first, and all of it, unconditionally — never paced, and
        // (being its own queue) never stuck behind a not-yet-due DATA packet
        // either (item 6).
        while let Some(bytes) = self.outbound_control.pop_front() {
            self.udp
                .send_to(&bytes, self.peer_addr)
                .await
                .map_err(|e| io_err("send", e))?;
        }
        while self.outbound_data.front().is_some() {
            let now = Instant::now();
            if let Some(deadline) = self.next_data_send_at
                && now < deadline
            {
                // Not yet due — leave it (and everything behind it, to
                // preserve send order) queued for a later pass.
                break;
            }
            let bytes = self
                .outbound_data
                .pop_front()
                .expect("front() just matched");
            let period = self.livecc.on_ack_received();
            let base = self.next_data_send_at.filter(|&t| t > now).unwrap_or(now);
            self.next_data_send_at = Some(base + period);
            self.udp
                .send_to(&bytes, self.peer_addr)
                .await
                .map_err(|e| io_err("send", e))?;
        }
        Ok(())
    }
}

/// Spawn the forwarder task for a connection with its own **dedicated,
/// unshared** socket (a [`SrtSocket::connect`]ed caller): reads that socket
/// directly (no demux table needed — nothing else ever reads it) and
/// forwards only datagrams from `peer`, matching the filtering
/// `Driver::run`'s old direct-`recv_from` arm used to do inline (issue
/// #1029; see the module doc on [`SrtListener`] for the shared-socket case,
/// which needs actual demultiplexing, not just this filter).
///
/// Returns the receiver the [`Driver`] reads plus the task's own
/// [`tokio::task::JoinHandle`] — the caller must abort it (via
/// [`TaskGuard`]) once the connection ends, or this loop keeps the socket's
/// local port bound forever (item 3): dropping the `Driver`'s `Arc<UdpSocket>`
/// reference alone does not stop this task, which holds its own clone and
/// stays parked in `recv_from` waiting for a datagram that may never come.
fn spawn_dedicated_socket_forwarder(
    udp: Arc<UdpSocket>,
    peer: std::net::SocketAddr,
    stats: Arc<Counters>,
) -> (mpsc::Receiver<Vec<u8>>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
    let handle = tokio::spawn(async move {
        let mut buf = [0u8; MAX_DATAGRAM];
        loop {
            match udp.recv_from(&mut buf).await {
                Ok((len, src)) if src == peer => {
                    if tx.try_send(buf[..len].to_vec()).is_err() {
                        if tx.is_closed() {
                            break; // Driver is gone.
                        }
                        // Full — the driver isn't draining fast enough; drop
                        // this datagram rather than buffer without bound
                        // (item 4) and count it.
                        stats.rx_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(_) => {} // datagram from another peer; ignore.
                Err(_) => break,
            }
        }
    });
    (rx, handle)
}

// ===========================================================================
// SrtListener
// ===========================================================================

/// An SRT listener that accepts incoming Caller connections.
///
/// # One socket, many connections (issue #1029)
///
/// Every connection this listener accepts shares its single bound socket —
/// unlike [`SrtSocket::connect`], which binds a fresh, unshared socket per
/// connection. `recv_from` on a shared socket delivers each datagram to
/// whichever waiting caller happens to be polled next, *not* whichever
/// caller the datagram is actually addressed to: if both this listener's own
/// `accept` loop and one or more accepted connections' driver tasks each
/// called `recv_from` on the same socket, a datagram belonging to one
/// connection could be handed to a completely different one (which would
/// then just discard it as foreign), silently losing it for its rightful
/// recipient. So exactly one task ever calls `recv_from` on `udp`: the
/// routing pump spawned once in [`SrtListener::bind`]. It demultiplexes the
/// way libsrt does (item 2, draft §4.3.1): a Destination Socket ID of `0`
/// (every handshake packet this crate's Caller ever sends, per the
/// `dest_socket_id` fix in `caller.rs`) always means "not yet an established
/// connection" and goes to `unrouted_rx` (`accept`'s own input); any other ID
/// must match an entry in `routes` *and* that entry's accepted source
/// address, or the datagram is dropped — a stray packet naming an ID that was
/// never accepted (or accepted from a different address) has nowhere
/// legitimate to go.
#[derive(Debug)]
pub struct SrtListener {
    udp: Arc<UdpSocket>,
    config: HandshakeConfig,
    /// Per-listener secret input to [`derive_cookie`] (`draft-sharabayko-srt-01`
    /// §4.3.1.1: "a cookie that is crafted based on host, port and current
    /// time"). Generated once at [`SrtListener::bind`] so every SYN Cookie
    /// this listener hands out is per-instance, not a fixed shared value a
    /// remote peer could pre-compute and replay against a different listener.
    cookie_secret: u64,
    pending: std::collections::HashMap<std::net::SocketAddr, PendingListener>,
    outbound_queue: std::collections::HashMap<std::net::SocketAddr, VecDeque<Vec<u8>>>,
    /// Shared demux table the routing pump task consults for every inbound
    /// datagram; `drain_completed` registers a new entry for each freshly
    /// accepted connection.
    routes: RouteTable,
    /// Datagrams the routing pump could not match to any entry in `routes`
    /// — candidate new-connection (handshake) traffic, `accept`'s own input.
    /// Bounded (item 4): see [`UNROUTED_CHANNEL_CAPACITY`].
    unrouted_rx: mpsc::Receiver<(std::net::SocketAddr, Vec<u8>)>,
    /// Shared with the routing pump task, which increments it whenever
    /// `unrouted_rx` was full (item 4) — a routed connection's own channel
    /// being full is counted on *that* connection's [`SrtSocket::stats`]
    /// instead (via its `RouteEntry`'s shared [`Counters`]).
    unrouted_dropped: Arc<AtomicU64>,
}

#[derive(Debug)]
struct PendingListener {
    handshake: ListenerHandshake,
    params: Option<HandshakeOutput>,
    /// The peer's ISN, extracted from the INDUCTION handshake packet.
    peer_initial_seq: u32,
    /// This listener's own ISN for this connection, generated once when the
    /// entry was created and echoed back here (rather than re-read from
    /// `self.config` at completion) so it is guaranteed to be the exact
    /// value the [`ListenerHandshake`] was constructed with — see
    /// [`SrtListener::handle_datagram`].
    our_initial_seq: u32,
}

impl SrtListener {
    /// Bind an SRT listener on `addr`.
    ///
    /// # Encryption
    /// A [`HandshakeConfig`] with `crypto` set is refused with
    /// [`Error::InvalidField`] before any network I/O, and a peer whose
    /// CONCLUSION requests encryption (a Key Material extension or a
    /// non-zero `Encryption Field`) is rejected — this adapter's data path
    /// does not encrypt, so an encrypted connection must never be accepted.
    pub async fn bind<A: tokio::net::ToSocketAddrs>(
        addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        refuse_crypto(&config)?;
        let socket = Arc::new(UdpSocket::bind(addr).await.map_err(|e| io_err("bind", e))?);
        let routes: RouteTable = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let (unrouted_tx, unrouted_rx) = mpsc::channel(UNROUTED_CHANNEL_CAPACITY);
        let unrouted_dropped = Arc::new(AtomicU64::new(0));
        spawn_listener_routing_pump(
            Arc::clone(&socket),
            Arc::clone(&routes),
            unrouted_tx,
            Arc::clone(&unrouted_dropped),
        );
        Ok(SrtListener {
            udp: socket,
            config,
            cookie_secret: random_u64(),
            pending: std::collections::HashMap::new(),
            outbound_queue: std::collections::HashMap::new(),
            routes,
            unrouted_rx,
            unrouted_dropped,
        })
    }

    /// Datagrams dropped because the candidate-new-connection queue was full
    /// (item 4) — a handshake flood outrunning [`SrtListener::accept`].
    pub fn unrouted_dropped(&self) -> u64 {
        self.unrouted_dropped.load(Ordering::Relaxed)
    }

    /// The local socket address the listener is bound to.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.udp.local_addr().map_err(|e| io_err("local_addr", e))
    }

    /// Whether this listener has its own crypto config (feature-gated: with
    /// the `crypto` feature compiled out there is no such field, and the
    /// adapter can never accept an encrypted connection).
    fn local_crypto_enabled(&self) -> bool {
        #[cfg(feature = "crypto")]
        {
            self.config.crypto.is_some()
        }
        #[cfg(not(feature = "crypto"))]
        {
            false
        }
    }

    /// Accept the next incoming SRT connection.
    pub async fn accept(&mut self) -> Result<SrtSocket> {
        // Pending-handshake expiry runs on its own timer, independent of
        // receive activity: with the previous `timeout(100ms, recv_from)`
        // pattern, `tick_pending` only ran while the socket sat idle, so a
        // steady trickle of traffic from *other* sources kept every stalled
        // entry alive forever.
        let mut ticker = tokio::time::interval(PENDING_TICK_INTERVAL);

        loop {
            if let Some(conn) = self.drain_completed() {
                return conn;
            }

            tokio::select! {
                // Candidate new-connection traffic the routing pump could
                // not match to an already-accepted connection (issue #1029)
                // — never a direct `udp.recv_from` racing every accepted
                // connection's own driver task for the same socket.
                r = self.unrouted_rx.recv() => {
                    match r {
                        Some((src, bytes)) => {
                            let _ = self.handle_datagram(src, &bytes);
                            self.flush_for_peer(src).await?;
                        }
                        None => {
                            return Err(Error::Io {
                                kind: std::io::ErrorKind::BrokenPipe,
                                context: "listener routing pump ended",
                            });
                        }
                    }
                }
                _ = ticker.tick() => {
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

        // A peer that asks for encryption is refused, never accepted: this
        // adapter's data path never applies the negotiated SEK (§6.1.5), so
        // accepting would silently exchange plaintext (or deliver a peer's
        // ciphertext to the application as if it were TS). With the `crypto`
        // feature the sans-IO engine already rejects a Key Material
        // *extension* (`(None, Some(km))` → `REJ_UNSECURE`, see
        // `listener.rs`); this adapter-level check covers a non-zero
        // `Encryption Field` (§3.2.1, Table 2) on any handshake and the
        // `KMREQ` extension-flag bit on a CONCLUSION (on an INDUCTION that
        // 16-bit field is the legacy UDT socket-type value, not the Table 3
        // bitmask — see `handshake_sm::INDUCTION_LEGACY_SOCKET_TYPE`).
        if !self.local_crypto_enabled()
            && let ControlPacket::Handshake(hp) = &ctrl
        {
            let asks_cipher = hp.encryption_field != EncryptionField::NoEncryption;
            let asks_kmreq = hp.handshake_type == crate::packet::HandshakeType::Conclusion
                && hp.extension_field.kmreq();
            if asks_cipher || asks_kmreq {
                // Refuse before allocating handshake state for the
                // source; `own_socket_id` is 0 because no socket was ever
                // assigned to this refused connection.
                let rejection = build_encryption_rejection(hp, 0, &self.config)?;
                self.outbound_queue
                    .entry(src)
                    .or_default()
                    .push_back(rejection);
                self.pending.remove(&src);
                return Ok(());
            }
        }

        let is_new = !self.pending.contains_key(&src);

        if is_new {
            // Pending state is bounded: a new source can only be admitted
            // after expired entries have been reaped, and even then only if
            // room remains — otherwise the source is ignored (§4.3.1.1's SYN
            // Cookie keeps unverified callers from allocating resources in
            // the first place; this caps what a verified-looking flood can
            // hold).
            if self.pending.len() >= MAX_PENDING {
                self.tick_pending();
                if self.pending.len() >= MAX_PENDING {
                    return Ok(());
                }
            }

            let peer_isn = match &ctrl {
                ControlPacket::Handshake(hp) => hp.initial_seq_number,
                _ => return Ok(()),
            };

            // Both this connection's own Socket ID and its ISN must be
            // freshly generated, not a predictable incrementing counter
            // (Socket ID) or the listener-wide `config.initial_seq_number`
            // reused for every accepted peer (ISN) — `draft-sharabayko-srt-01`
            // §3/§4.3.1.1 expects both random per connection, and either a
            // fixed or a sequential value lets an off-path party that
            // observed one accepted connection predict the next one's wire
            // identifiers.
            let own_socket_id = random_socket_id();
            let our_initial_seq = random_isn();
            let mut per_conn_config = self.config.clone();
            per_conn_config.initial_seq_number = our_initial_seq;
            // §4.3.1.1: "a cookie that is crafted based on host, port and
            // current time with 1 minute accuracy" — `derive_cookie` mixes
            // exactly those inputs (`crate::handshake_sm`'s existing,
            // documented derivation), keyed by this listener's own secret so
            // two listeners never hand out the same cookie for the same
            // peer/time bucket.
            let peer_key = addr_to_u64(&src);
            let time_bucket = unix_time_bucket();
            let syn_cookie = derive_cookie(peer_key, time_bucket, self.cookie_secret);
            let hs = ListenerHandshake::new(own_socket_id, syn_cookie, per_conn_config);
            self.pending.insert(
                src,
                PendingListener {
                    handshake: hs,
                    params: None,
                    peer_initial_seq: peer_isn,
                    our_initial_seq,
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
            // A timed-out peer will never see another packet: drop its queued
            // handshake bytes too, so `outbound_queue` cannot grow per-source
            // without bound under a flood.
            self.outbound_queue.remove(&addr);
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
        // `drain_completed` only reaches entries filtered to
        // `ListenerHandshakeState::Connected`, so `negotiated()` is always
        // `Some` here.
        let negotiated = entry
            .handshake
            .negotiated()
            .expect("filtered to Connected state");
        // The peer's negotiated SRT Socket ID (distinct from its ISN above).
        let peer_socket_id = negotiated.peer_socket_id;
        // This listener's own Socket ID for this connection — the value a
        // real peer's packets carry as `dest_socket_id` from here on, and so
        // the key the routing pump demultiplexes by (item 2; see
        // [`spawn_listener_routing_pump`]).
        let own_socket_id = negotiated.own_socket_id;
        // The exact ISN this listener generated for `entry` at INDUCTION
        // time (`handle_datagram`) — not `self.config.initial_seq_number`,
        // which is the listener-wide template and no longer carries the
        // per-connection value once randomized.
        let our_initial_seq = entry.our_initial_seq;
        let tsbpd_delay_ms = u64::from(self.config.latency_ms);
        let tsbpd_time_base = 0;
        let epoch = Instant::now();

        // Register this connection's own route in the shared demux table
        // (issue #1029) *before* handing off to its driver task — the
        // routing pump must never see a datagram naming this connection's
        // Socket ID land in `unrouted_rx` again once this connection exists.
        let (tx, rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
        let stats = Arc::new(Counters::default());
        self.routes.lock().unwrap().insert(
            own_socket_id,
            RouteEntry {
                addr,
                tx,
                stats: Arc::clone(&stats),
            },
        );

        // Share the listener's Arc<UdpSocket> with the connection's driver.
        let conn = SrtSocket::spawn(
            Arc::clone(&self.udp),
            addr,
            rx,
            stats,
            RouteCleanup(Some((own_socket_id, Arc::clone(&self.routes)))),
            TaskGuard(None),
            our_initial_seq,
            peer_initial_seq,
            peer_socket_id,
            tsbpd_time_base,
            tsbpd_delay_ms,
            epoch,
            self.config.max_flow_window_size,
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

/// Spawn the single task that ever calls `recv_from` on a [`SrtListener`]'s
/// shared socket, demultiplexing every datagram the way libsrt does (item 2,
/// draft §4.3.1; libsrt's `CRcvQueue::worker` dispatches on Destination
/// Socket ID the same way): Destination Socket ID `0` always means "not yet
/// an established connection" and goes to `unrouted_tx` (`accept`'s own
/// input — every handshake packet this crate's Caller sends carries `0`
/// there, see `caller.rs`); any other ID must match a `routes` entry *and*
/// that entry's accepted source address, or the datagram is dropped — an ID
/// nothing accepted, or one accepted from a different address, has no
/// legitimate connection to reach. This is what makes it safe for the
/// listener's `accept` loop and every accepted connection's driver task to
/// run concurrently without racing each other for the same socket's
/// `recv_from`.
fn spawn_listener_routing_pump(
    udp: Arc<UdpSocket>,
    routes: RouteTable,
    unrouted_tx: mpsc::Sender<(std::net::SocketAddr, Vec<u8>)>,
    unrouted_dropped: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let mut buf = [0u8; MAX_DATAGRAM];
        loop {
            let (len, src) = match udp.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(_) => break, // socket error — end the pump.
            };
            let bytes = &buf[..len];
            match crate::packet::peek_dest_socket_id(bytes) {
                Some(0) | None => {
                    // `0` (candidate new connection) or too short to carry a
                    // full header at all — either way, not a routable
                    // established-connection packet; hand it to `accept`,
                    // which parses (and rejects malformed bytes) properly.
                    if unrouted_tx.try_send((src, bytes.to_vec())).is_err() {
                        if unrouted_tx.is_closed() {
                            // The listener itself is gone (dropped without a
                            // final `accept`); nothing more this pump can do.
                            break;
                        }
                        unrouted_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Some(id) => {
                    let route = routes.lock().unwrap().get(&id).cloned();
                    // No `route` entry (no connection was ever accepted with
                    // this ID), or one exists but from a different source
                    // address, is never routed — spoofed-source packets
                    // included.
                    if let Some(entry) = route
                        && entry.addr == src
                        && entry.tx.try_send(bytes.to_vec()).is_err()
                        && !entry.tx.is_closed()
                    {
                        // A closed channel would instead mean the
                        // connection's driver has already exited (it
                        // removes its own entry on the way out — see
                        // `RouteCleanup`), a benign race, not counted; a
                        // still-open-but-full channel is this connection's
                        // own backpressure (item 4).
                        entry.stats.rx_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    });
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Builds a Handshake rejection packet (`Handshake Type` = `1000 + code`,
/// §4.3 Table 7) refusing an encryption request with
/// [`RejectionReason::Unsecure`] ("password required or unexpected", code
/// 1011) — the same wire shape `listener.rs` emits for its own rejections.
/// The adapter needs it because a peer can advertise encryption via the
/// `Encryption Field` / `KMREQ` flag bits without ever sending a Key Material
/// extension block for the engine to notice.
fn build_encryption_rejection(
    hp: &HandshakePacket<'_>,
    own_socket_id: u32,
    config: &HandshakeConfig,
) -> Result<Vec<u8>> {
    let hp_out = HandshakePacket {
        timestamp: 0,
        dest_socket_id: hp.srt_socket_id,
        version: handshake_sm::HANDSHAKE_VERSION_5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0),
        initial_seq_number: 0,
        mtu: config.mtu,
        max_flow_window_size: config.max_flow_window_size,
        handshake_type: RejectionReason::Unsecure.to_handshake_type(),
        srt_socket_id: own_socket_id,
        syn_cookie: 0,
        peer_ip: config.local_ip,
        extensions: HandshakeExtensions(&[]),
    };
    handshake_sm::build_bytes(hp_out)
}

/// Refuses a [`HandshakeConfig`] that asks for payload encryption.
///
/// The sans-IO handshake engines implement the §6.1.5 Key Material exchange,
/// but this adapter's data path never applies the negotiated SEK — it would
/// send and receive plaintext while the caller believed the stream was
/// encrypted (and deliver a peer's ciphertext as if it were TS). Until the
/// data path is wired, an encrypted connection is refused up front instead.
fn refuse_crypto(config: &HandshakeConfig) -> Result<()> {
    #[cfg(feature = "crypto")]
    if config.crypto.is_some() {
        return Err(Error::InvalidField {
            what: "crypto",
            reason: "payload encryption is not yet supported by the tokio adapter; \
                     the sans-IO engines negotiate keys but io.rs would send plaintext",
        });
    }
    #[cfg(not(feature = "crypto"))]
    let _ = config;
    Ok(())
}

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

/// A random `u64` from the OS randomness source (`getrandom`), used as
/// `derive_cookie`'s `secret` input and to draw the per-connection Socket ID
/// and ISN (SRT-W11). `getrandom::getrandom` never returns a partially-filled
/// buffer — any failure is a `panic`, matching `HandshakeConfig`'s own
/// no-fallible-construction convention elsewhere in this adapter.
fn random_u64() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("OS randomness source");
    u64::from_ne_bytes(bytes)
}

/// The largest SRT Socket ID this adapter will ever generate.
///
/// `draft-sharabayko-srt-01` §3/§4.3.1.1 gives the wire field only as "32
/// bits" (this crate's own `specs/ietf_draft_sharabayko_srt_01.txt`, e.g. its
/// Handshake Packet description — no transcription in this crate's `docs/`
/// narrows that range further; there is no `docs/` directory at all yet). A
/// real libsrt peer, though, only ever *allocates* an ID in `1..=`
/// [`MAX_SRT_SOCKET_ID`]: libsrt's `CUDTUnited::generateSocketID`
/// (`srtcore/core.cpp`) masks a freshly drawn 32-bit value down with `&
/// 0x3FFFFFFF` before use — bit 30 (`0x40000000`) is libsrt's own "this ID
/// names a socket group, not a plain socket" marker, and bit 31 would make
/// the ID negative when read back as libsrt's signed C `int`. Generating
/// outside that range is wire-legal per the draft but not a value any real
/// SRT implementation would ever send or expect, so this adapter matches
/// libsrt's allocation range rather than exercising the full 32 bits.
const MAX_SRT_SOCKET_ID: u32 = 0x3FFF_FFFF;

/// A random SRT Socket ID for a new connection, drawn from the same
/// OS-seeded source as [`random_u64`], in `1..=`[`MAX_SRT_SOCKET_ID`] (see
/// its doc). Never `0` — that value marks "no socket assigned" elsewhere in
/// this adapter (see [`build_encryption_rejection`]'s `own_socket_id`
/// argument for a refused, pre-handshake peer) and is reserved for that
/// meaning here too — the `% MAX_SRT_SOCKET_ID` reduction followed by `+ 1`
/// makes both guarantees (never `0`, never above the cap) hold by
/// construction, with no retry loop needed.
fn random_socket_id() -> u32 {
    (random_u64() as u32 % MAX_SRT_SOCKET_ID) + 1
}

/// A random Initial Sequence Number in the legal 31-bit SRT sequence-number
/// range (`SEQ_NUMBER_MASK`, §3) for a new connection. Every new Caller
/// connect and every new accepted Listener connection must draw a fresh
/// value here rather than reuse a fixed/default `HandshakeConfig::initial_seq_number`
/// (SRT-W11): `draft-sharabayko-srt-01` §3/§4.3.1.1 expects the ISN
/// unpredictable per connection, same as the Socket ID above.
fn random_isn() -> u32 {
    (random_u64() as u32) & SEQ_NUMBER_MASK
}

#[cfg(test)]
mod socket_id_tests {
    use super::*;

    /// Bite test: run against the unfixed `random_socket_id` (a bare
    /// `random_u64() as u32`, only ever excluding `0`) and this fails —
    /// values above `MAX_SRT_SOCKET_ID` come up immediately (bit 31 alone is
    /// set on ~50% of draws). Fixed, every draw is folded into
    /// `1..=MAX_SRT_SOCKET_ID` by construction.
    #[test]
    fn random_socket_id_stays_in_the_libsrt_allocation_range() {
        for _ in 0..10_000 {
            let id = random_socket_id();
            assert!(id != 0, "socket id must never be 0");
            assert!(
                id <= MAX_SRT_SOCKET_ID,
                "socket id {id:#010x} exceeds the libsrt allocation range {MAX_SRT_SOCKET_ID:#010x}"
            );
        }
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

/// Extract the peer's `initial_seq_number` from a handshake control packet's
/// bytes. This seeds ARQ and TSBPD sequence tracking for the entire
/// connection, so "the bytes didn't parse as a Handshake" must be a distinct,
/// caller-visible outcome from "the peer's ISN is genuinely 0" — the two are
/// otherwise indistinguishable to a caller that only sees a `u32`. Returns
/// [`Error::InvalidField`] rather than defaulting to `0` on any parse
/// failure.
fn require_peer_isn(bytes: &[u8]) -> Result<u32> {
    match SrtPacket::parse(bytes) {
        Ok(SrtPacket::Control(ControlPacket::Handshake(hp))) => Ok(hp.initial_seq_number),
        _ => Err(Error::InvalidField {
            what: "peer isn",
            reason: "handshake reached Connected but its final packet did not re-parse as a \
                     Handshake control packet; refusing to seed ARQ/TSBPD with a fabricated ISN",
        }),
    }
}

#[cfg(test)]
mod isn_tests {
    use super::*;
    use crate::packet::{
        EncryptionField, HandshakeExtensionFlags, HandshakeExtensions, HandshakePacket,
        HandshakeType,
    };

    fn handshake_bytes(initial_seq_number: u32) -> Vec<u8> {
        let hp = HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0,
            version: 5,
            encryption_field: EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(0),
            initial_seq_number,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: HandshakeType::Conclusion,
            srt_socket_id: 42,
            syn_cookie: 0,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(&[]),
        };
        crate::handshake_sm::build_bytes(hp).expect("build handshake bytes")
    }

    #[test]
    fn require_peer_isn_extracts_nonzero_isn() {
        let bytes = handshake_bytes(0xABCD_1234);
        assert_eq!(require_peer_isn(&bytes).unwrap(), 0xABCD_1234);
    }

    #[test]
    fn require_peer_isn_distinguishes_genuine_zero_from_parse_failure() {
        // A genuine ISN of 0 is a valid, well-formed handshake and must
        // succeed as Ok(0) — not be conflated with "couldn't parse".
        let bytes = handshake_bytes(0);
        assert_eq!(require_peer_isn(&bytes).unwrap(), 0);

        // Bytes that don't parse as a Handshake control packet at all
        // (too short to even carry the fixed SRT header) must error, never
        // silently produce a plausible-looking 0.
        let err = require_peer_isn(&[0u8; 4]).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidField {
                what: "peer isn",
                ..
            }
        ));
    }

    #[test]
    fn require_peer_isn_rejects_non_handshake_control_packet() {
        // A well-formed control packet of the WRONG type (Keep-Alive, not
        // Handshake) must also error rather than default to 0.
        let ka = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: 0,
            dest_socket_id: 7,
            libsrt_pad: false,
        });
        let mut buf = alloc::vec![0u8; ka.serialized_len()];
        ka.serialize_into(&mut buf).expect("serialize keepalive");
        let err = require_peer_isn(&buf).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidField {
                what: "peer isn",
                ..
            }
        ));
    }
}

// ===========================================================================
// Adapter security tests (r08-SRT-C2 / C6 / C7)
// ===========================================================================

#[cfg(all(test, feature = "tokio"))]
mod adapter_tests {
    use super::*;
    #[cfg(feature = "crypto")]
    use crate::handshake_sm::CryptoConfig;
    use crate::packet::{
        AckCif, AckPacket, DropReqPacket, EncryptionKeyField, HandshakeExtensionFlags,
        HandshakeExtensions, HandshakePacket, PacketPosition,
    };

    const PEER: &str = "10.0.0.1:9999";
    /// TSBPD latency used by the test drivers (the spec minimum, §4.5.1).
    const LATENCY_MS: u64 = 120;

    fn peer_addr() -> std::net::SocketAddr {
        PEER.parse().unwrap()
    }

    /// A `Driver` wired to a throwaway bound socket and an unbounded delivery
    /// channel, with the same engine defaults `spawn` uses — so `ingress`,
    /// `tick_engines` and `staged` can be driven directly without a live
    /// handshake.
    async fn test_driver(max_flow_window: u32) -> (Driver, mpsc::Receiver<Vec<u8>>) {
        let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (deliver, rx) = mpsc::channel(DELIVER_CHANNEL_CAPACITY);
        // These tests drive `ingress`/`tick_engines` directly, never
        // `Driver::run`'s select loop, so the inbound-datagram channel is
        // never actually read from — an unused sender/no route cleanup is
        // fine here.
        let (_datagram_tx, datagram_rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
        let driver = Driver {
            udp,
            peer_addr: peer_addr(),
            peer_socket_id: 1,
            stats: Arc::new(Counters::default()),
            rx: datagram_rx,
            _route_cleanup: RouteCleanup(None),
            _forwarder_guard: TaskGuard(None),
            last_rx_at: Instant::now(),
            sender: ArqSender::new(1),
            receiver: ArqReceiver::new(1, 0, max_flow_window),
            tsbpd: TsbpdScheduler::new(0, 0, LATENCY_MS, DEFAULT_DRIFT_US, true, None),
            livecc: LiveCC::new(DEFAULT_MAX_BW),
            next_message_number: 1,
            next_send_seq: 0,
            epoch: Instant::now(),
            staged: std::collections::BTreeMap::new(),
            outbound_data: VecDeque::new(),
            outbound_control: VecDeque::new(),
            next_data_send_at: None,
            deliver,
            peer_shutdown: false,
            max_flow_window,
        };
        (driver, rx)
    }

    fn data_bytes(seq: u32, timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let dp = DataPacket {
            seq_number: seq,
            position: PacketPosition::Solo,
            in_order: true,
            key_flag: EncryptionKeyField::NotEncrypted,
            retransmitted: false,
            message_number: seq,
            timestamp,
            dest_socket_id: 1,
            data: payload,
        };
        let mut buf = alloc::vec![0u8; dp.serialized_len()];
        dp.serialize_into(&mut buf).expect("serialize data");
        buf
    }

    fn encrypted_data_bytes(seq: u32, timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut buf = data_bytes(seq, timestamp, payload);
        // Flip the `KK` field of word 1 from `00b` to `01b` (even key).
        let word1 = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let word1 = (word1 & !(0b11 << 27)) | (u32::from(EncryptionKeyField::Even.to_bits()) << 27);
        buf[4..8].copy_from_slice(&word1.to_be_bytes());
        buf
    }

    fn dropreq_bytes(first: u32, last: u32) -> Vec<u8> {
        let pkt = ControlPacket::DropReq(DropReqPacket {
            message_number: 0,
            timestamp: 0,
            dest_socket_id: 1,
            first_seq: first,
            last_seq: last,
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).expect("serialize dropreq");
        buf
    }

    fn induction_bytes(socket_id: u32) -> Vec<u8> {
        let hp = HandshakePacket {
            timestamp: 0,
            dest_socket_id: 0,
            version: crate::handshake_sm::HANDSHAKE_VERSION_4,
            encryption_field: crate::packet::EncryptionField::NoEncryption,
            extension_field: HandshakeExtensionFlags(
                crate::handshake_sm::INDUCTION_LEGACY_SOCKET_TYPE,
            ),
            initial_seq_number: 0,
            mtu: 1500,
            max_flow_window_size: 8192,
            handshake_type: crate::packet::HandshakeType::Induction,
            srt_socket_id: socket_id,
            syn_cookie: 0,
            peer_ip: [0; 4],
            extensions: HandshakeExtensions(&[]),
        };
        let pkt = ControlPacket::Handshake(hp);
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).expect("serialize induction");
        buf
    }

    fn drain_deliveries(rx: &mut mpsc::Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(payload) = rx.try_recv() {
            out.push(payload);
        }
        out
    }

    // -----------------------------------------------------------------------
    // Item 1 — refuse encryption in the tokio adapter
    // -----------------------------------------------------------------------

    #[cfg(feature = "crypto")]
    fn crypto_config() -> HandshakeConfig {
        HandshakeConfig {
            crypto: Some(CryptoConfig {
                passphrase: b"test-passphrase".to_vec(),
                salt: [0x11u8; crate::crypto::SALT_LEN],
                sek: vec![0x22u8; 16],
            }),
            ..HandshakeConfig::default()
        }
    }

    #[cfg(feature = "crypto")]
    #[tokio::test]
    async fn connect_with_crypto_is_refused_before_any_io() {
        // A bound UDP socket stands in for the "peer": a refused connection
        // must not emit even the INDUCTION datagram.
        let peer_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer_socket.local_addr().unwrap();

        let result = tokio::time::timeout(
            Duration::from_millis(2_000),
            SrtSocket::connect(peer_addr, crypto_config()),
        )
        .await
        .expect("crypto refusal must return immediately, not run the handshake");

        let err = result.expect_err("crypto-enabled connect must be refused");
        assert!(
            matches!(err, Error::InvalidField { what: "crypto", .. }),
            "expected InvalidField {{ what: \"crypto\" }}, got {err:?}"
        );

        let mut buf = [0u8; 64];
        assert!(
            tokio::time::timeout(Duration::from_millis(200), peer_socket.recv_from(&mut buf))
                .await
                .is_err(),
            "a refused connect must send nothing to the peer"
        );
    }

    #[cfg(feature = "crypto")]
    #[tokio::test]
    async fn bind_with_crypto_is_refused() {
        let err = SrtListener::bind("127.0.0.1:0", crypto_config())
            .await
            .expect_err("crypto-enabled bind must be refused");
        assert!(
            matches!(err, Error::InvalidField { what: "crypto", .. }),
            "expected InvalidField {{ what: \"crypto\" }}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn encrypted_data_packet_is_never_staged_or_delivered() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let ts = 0u32;
        // The unencrypted packet at seq 0 is staged normally.
        driver
            .ingress(&data_bytes(0, ts, b"plain"))
            .expect("ingress plain");
        // An encrypted packet (KK != 0) must never reach `staged` or the app.
        driver
            .ingress(&encrypted_data_bytes(1, ts, b"ciphertext"))
            .expect("ingress encrypted");

        assert!(
            !driver.staged.contains_key(&1),
            "a KK != 0 data packet must never be staged"
        );
        let delivered = drain_deliveries(&mut rx);
        assert!(
            delivered.iter().all(|p| p != b"ciphertext"),
            "ciphertext must never be delivered to the application"
        );
    }

    // -----------------------------------------------------------------------
    // Issue #1062 — sequence/message-number wrap must not panic
    // -----------------------------------------------------------------------

    /// Pre-fix, `send_one` incremented both counters with a plain
    /// `u32::wrapping_add(1)` (wraps at `u32::MAX`, not the wire field's own
    /// bit width). Starting both counters two steps below their real 31-bit
    /// (sequence) / 26-bit (message number) boundaries and sending four
    /// packets crosses both wraps; pre-fix this panicked inside
    /// `DataPacket::serialize_into`'s `.expect("buffer sized from
    /// serialized_len")` call sites in `arq::sender` with `FieldTooWide`
    /// (verified via a copied pre-fix `io.rs`, `git show HEAD:srt-runtime/src/io.rs`,
    /// never `git stash` — shared worktree — swapped back immediately after:
    /// `panicked ... buffer sized from serialized_len: FieldTooWide { what:
    /// "Packet Sequence Number", value: 2147483648, bits: 31 }`).
    #[tokio::test]
    async fn send_one_wraps_seq_and_message_number_without_panicking() {
        let (mut driver, _rx) = test_driver(8192).await;
        driver.next_send_seq = SEQ_NUMBER_MASK - 1;
        driver.next_message_number = crate::packet::data::MESSAGE_NUMBER_MASK - 1;

        // Four sends: seq goes MASK-1, MASK, 0, 1 (wrapping at 2^31); message
        // number goes MASK-1, MASK, 1, 2 (wrapping at 2^26 but skipping 0 —
        // libsrt's `SRT_MSGNO_CONTROL` sentinel, see `next_message_number`).
        // Each call re-derives the wire bytes through the exact
        // `serialize_into` path the panic was in — reaching here without
        // panicking is the test.
        for _ in 0..4 {
            driver.send_one(b"payload");
        }

        assert_eq!(
            driver.next_send_seq, 2,
            "sequence number must wrap at 2^31, not 2^32"
        );
        assert_eq!(
            driver.next_message_number, 3,
            "message number must wrap at 2^26 (skipping 0), not 2^32"
        );

        // Every enqueued DATA packet must itself carry a legal (in-range)
        // seq/message number — not just the counter that produced it.
        for bytes in &driver.outbound_data {
            let dp = DataPacket::parse(bytes).expect("parse enqueued data packet");
            assert!(dp.seq_number <= SEQ_NUMBER_MASK);
            assert!(dp.message_number <= crate::packet::data::MESSAGE_NUMBER_MASK);
        }
    }

    // -----------------------------------------------------------------------
    // Item 6 — ACK/NAK must not queue behind a not-yet-due paced DATA packet
    // -----------------------------------------------------------------------

    /// Pre-fix, a single FIFO `outbound` queue (each item tagged `is_data`)
    /// meant `flush_outbound` stopped at the first not-yet-due DATA packet
    /// and never looked past it — so an AckAck enqueued *behind* one (the
    /// ordinary case: `send_one` queues DATA, then the peer's own Ack —
    /// handled by `ingress` — queues the reply) sat there just as
    /// pacing-throttled as the DATA in front of it, even though §5.1.2 paces
    /// DATA only.
    #[tokio::test]
    async fn ackack_is_not_queued_behind_a_not_yet_due_paced_data_packet() {
        let (mut driver, _rx) = test_driver(8192).await;
        // `flush_outbound` does a real `send_to` below — a loopback address
        // accepts it without needing an actual listener there, unlike
        // `test_driver`'s default non-routable `PEER` test address.
        driver.peer_addr = "127.0.0.1:1".parse().unwrap();
        // A near-zero bandwidth cap pushes each DATA packet's pacing slot far
        // into the future. The very first send establishes that pacing
        // schedule (there is no deadline yet before anything has been sent);
        // the second one is the packet this test actually checks stays
        // queued behind its own not-yet-arrived slot.
        driver.livecc = LiveCC::new(MaxBwConfig::Set(1));
        driver.send_one(&[0u8; 200]);
        driver
            .flush_outbound()
            .await
            .expect("flush first data packet");
        driver.send_one(&[0u8; 200]);
        assert_eq!(driver.outbound_data.len(), 1, "second DATA must be queued");
        assert!(
            driver.next_paced_send_wait().is_some(),
            "the second DATA packet must not be immediately due"
        );

        let ack = ControlPacket::Ack(AckPacket {
            ack_number: 1,
            timestamp: 0,
            dest_socket_id: 1,
            cif: AckCif::Full {
                last_ack_seq: 0,
                rtt_us: 0,
                rtt_var_us: 0,
                avail_buf_size: 8192,
                pkt_recv_rate: 0,
                est_link_capacity: 0,
                recv_rate_bps: 0,
            },
        });
        let mut ack_bytes = alloc::vec![0u8; ack.serialized_len()];
        ack.serialize_into(&mut ack_bytes).expect("serialize ack");
        driver.ingress(&ack_bytes).expect("ingress ack");
        assert_eq!(
            driver.outbound_control.len(),
            1,
            "the peer's Ack must have queued exactly one AckAck"
        );

        driver.flush_outbound().await.expect("flush");

        assert!(
            driver.outbound_control.is_empty(),
            "the AckAck must have been sent even though the DATA packet's \
             pacing slot had not arrived yet"
        );
        assert_eq!(
            driver.outbound_data.len(),
            1,
            "the still-not-due DATA packet must remain queued, untouched"
        );
    }

    // -----------------------------------------------------------------------
    // Item 2 — one loss must not freeze delivery or grow memory
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delivery_resumes_after_permanent_loss_once_latency_passes() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let ts0 = driver.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;

        // seq 1 is lost and never retransmitted.
        for seq in [0u32, 2, 3] {
            driver
                .ingress(&data_bytes(seq, ts0, &[seq as u8]))
                .expect("ingress");
        }
        assert!(
            drain_deliveries(&mut rx).is_empty(),
            "play time not yet due"
        );

        // §4.6 / rule 21: at the successor's play time (one latency after
        // send) the gap is skipped and packets 2 and 3 are DELIVERED — not
        // thrown away with the lost packet. Poll ticks like the driver task
        // would, finishing well inside the too-late threshold window.
        let mut delivered = Vec::new();
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            driver.tick_engines();
            delivered.extend(drain_deliveries(&mut rx));
            if delivered.len() >= 3 {
                break;
            }
        }
        let seqs: Vec<u8> = delivered.iter().map(|p| p[0]).collect();
        assert_eq!(
            seqs,
            vec![0, 2, 3],
            "the gap must be skipped at the successor's play time and its \
             packets delivered (live mode, §4.6/rule 21)"
        );
    }

    #[tokio::test]
    async fn dropreq_unblocks_delivery_immediately() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let ts0 = driver.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;

        for seq in [0u32, 2, 3] {
            driver
                .ingress(&data_bytes(seq, ts0, &[seq as u8]))
                .expect("ingress");
        }
        assert!(
            drain_deliveries(&mut rx).is_empty(),
            "play time not yet due"
        );

        // The peer announces it will never send seq 1.
        driver
            .ingress(&dropreq_bytes(1, 1))
            .expect("ingress dropreq");

        tokio::time::sleep(Duration::from_millis(140)).await;
        driver.tick_engines();

        let delivered = drain_deliveries(&mut rx);
        let seqs: Vec<u8> = delivered.iter().map(|p| p[0]).collect();
        assert_eq!(seqs, vec![0, 2, 3], "DROPREQ must unblock the gap");
    }

    #[tokio::test]
    async fn staged_never_exceeds_the_flow_window() {
        const WINDOW: u32 = 64;
        let (mut driver, mut rx) = test_driver(WINDOW).await;
        let ts = 0u32;

        // seq 0 is the delivery cursor and never arrives; feed window+100
        // packets past the gap.
        for seq in 1..=(WINDOW + 100) {
            driver
                .ingress(&data_bytes(seq, ts, &[seq as u8]))
                .expect("ingress");
            assert!(
                driver.staged.len() as u32 <= WINDOW,
                "staged grew to {} past a gap (window {WINDOW})",
                driver.staged.len()
            );
        }
        drain_deliveries(&mut rx);
    }

    // -----------------------------------------------------------------------
    // Item 3 — bound listener pending state
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn listener_pending_is_capped_under_a_source_flood() {
        let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
            .await
            .expect("bind");

        for i in 0..5000u32 {
            let src = std::net::SocketAddr::from(([198, 18, (i >> 8) as u8, i as u8], 40_000));
            let _ = listener.handle_datagram(src, &induction_bytes(i));
        }

        assert!(
            listener.pending.len() <= 1024,
            "pending grew to {} — must be capped at MAX_PENDING",
            listener.pending.len()
        );
    }

    #[tokio::test]
    async fn pending_entries_expire_while_other_sources_keep_sending() {
        let config = HandshakeConfig {
            retransmit_after_ticks: 1,
            max_retries: 2,
            ..HandshakeConfig::default()
        };
        let mut listener = SrtListener::bind("127.0.0.1:0", config)
            .await
            .expect("bind");

        let stalling_src = std::net::SocketAddr::from(([203, 0, 113, 1], 1234));
        listener
            .handle_datagram(stalling_src, &induction_bytes(7))
            .expect("first induction");

        // Continuous traffic from *other* sources must not keep the stalling
        // entry alive: the timer-driven tick expires it.
        for i in 0..10u32 {
            let src = std::net::SocketAddr::from(([198, 51, (i >> 8) as u8, i as u8], 40_000));
            let _ = listener.handle_datagram(src, &induction_bytes(100 + i));
            listener.tick_pending();
        }

        assert!(
            !listener.pending.contains_key(&stalling_src),
            "a stalled handshake must expire even under continuous traffic"
        );
    }

    // -----------------------------------------------------------------------
    // Item 2 (BLOCKER) — dropping an accepted socket must not leak its route
    // -----------------------------------------------------------------------

    /// Pre-fix, `Driver::run`'s route-table cleanup sat at the tail of its
    /// `loop`, reached only on a *normal* exit — `SrtSocket::Drop` instead
    /// calls `handle.abort()`, which cancels the task at its next await point
    /// and drops its future (the `Driver` included) without ever running
    /// that inline code. The entry then sat in `routes` forever: a harmless
    /// no-op for the new ID-keyed demux's correctness (a stale entry's key is
    /// a dead Socket ID no live peer packet names), but an unbounded
    /// per-listener memory leak across repeated accept/drop cycles.
    /// `RouteCleanup`'s `Drop` impl runs on both an abort and a normal
    /// return, since it fires whenever the guard itself — owned by the
    /// `Driver`, and so by the task's future — is dropped.
    #[tokio::test]
    async fn dropping_an_accepted_socket_removes_its_route_entry() {
        let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
            .await
            .expect("bind");
        let listener_addr = listener.local_addr().expect("local addr");

        let accept_jh = tokio::spawn(async move {
            let s = listener.accept().await.expect("accept");
            (listener, s)
        });
        let caller = SrtSocket::connect(listener_addr, HandshakeConfig::default())
            .await
            .expect("connect");
        let (listener, accepted) = accept_jh.await.expect("join accept");

        assert_eq!(
            listener.routes.lock().unwrap().len(),
            1,
            "the freshly accepted connection must have registered exactly one route"
        );

        drop(accepted); // SrtSocket::Drop aborts the driver task.
        drop(caller);

        // The abort is not synchronous with `drop` — the aborted task's
        // `Drop` impls (including `RouteCleanup`) run at the task's own next
        // poll, which the runtime schedules promptly but not instantly.
        let mut cleaned = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if listener.routes.lock().unwrap().is_empty() {
                cleaned = true;
                break;
            }
        }
        assert!(
            cleaned,
            "route table still has {} entries after the accepted socket was \
             dropped — the route leaked (pre-fix: `Driver::run`'s cleanup only \
             ran on a normal loop exit, never on `SrtSocket::Drop`'s task abort)",
            listener.routes.lock().unwrap().len()
        );
    }
}
