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
//! engines → socket TX and, crucially, drives the timers (retransmit, ACK,
//! NAK, TSBPD release, keep-alive, pacing) from the connection's own next
//! deadline (`Driver::poll_timeout`) — so loss recovery keeps making progress
//! even when the application is neither sending nor receiving at that instant,
//! without a fixed tick. The sans-IO core stays `no_std`; the adapter is pure
//! plumbing.
//!
//! Timeouts and the datagram size are explicit: [`IoConfig`]
//! (`connect`/`handshake`/`read_idle`/`write`, `max_datagram`). Datagrams are
//! [`bytes::Bytes`], one exactly-sized allocation per received datagram (never a
//! view into a shared chunk, which would pin the whole chunk while one packet
//! is held); the payload handed to the application is a slice of it, so there
//! is no further copy.
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
//! application call timing: [`SrtSocket::send`] enqueues a payload,
//! [`SrtSocket::recv`] awaits a delivered payload, and the task in between
//! runs the select loop (RX / app-send / next deadline) until the peer shuts
//! down, falls silent, or the [`SrtSocket`] is dropped.
//!
//! # Connection lifetime
//!
//! - **Keep-alive** (§3.2.3): after one second without sending anything, the
//!   driver sends a Keep-Alive, so a quiet sender is not mistaken for a dead
//!   one by a peer's own idle timeout (libsrt's default is five seconds).
//! - **Peer idle timeout**: five seconds without *any* packet from the peer
//!   ends the connection ([`SrtSocket::recv`] then returns `None`).
//! - **Shutdown** (§3.2.7): dropping an [`SrtSocket`] sends the peer a
//!   SHUTDOWN so it closes at once instead of waiting out its idle timeout;
//!   a received SHUTDOWN ends the connection.
//! - **Backpressure**: [`SrtSocket::send`] waits while the sender already
//!   holds a full flow window of unacknowledged packets, so a stalled or dead
//!   peer cannot make the send buffer grow without bound.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use alloc::collections::VecDeque;
use core::time::Duration;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::arq::seq::{seq_diff, seq_in_closed_range, seq_lt, seq_next};
use crate::arq::{Receiver as ArqReceiver, Sender as ArqSender};
use crate::caller::CallerHandshake;
use crate::error::{Error, Result};
use crate::handshake_sm::{self, HandshakeConfig, HandshakeOutput, RejectionReason, derive_cookie};
use crate::listener::{ListenerHandshake, ListenerHandshakeState};
use crate::livecc::{LiveCC, MaxBwConfig};
use crate::packet::data::next_message_number;
use crate::packet::misc::{KeepAlivePacket, ShutdownPacket};
use crate::packet::{
    ControlPacket, DropReqPacket, EncryptionField, EncryptionKeyField, HandshakeExtensionFlags,
    HandshakeExtensions, HandshakePacket, HandshakeType, SEQ_NUMBER_MASK, SRT_HEADER_LEN,
    SrtPacket,
};
use crate::tsbpd::{TickOutcome, TsbpdScheduler};

// ===========================================================================
// Constants
// ===========================================================================

/// Default for [`IoConfig::max_datagram`]: one Ethernet MTU of UDP payload.
const DEFAULT_MAX_DATAGRAM: usize = 1500;
/// Smallest accepted [`IoConfig::max_datagram`]: a full SRT header (16 bytes)
/// plus room for a control CIF. A smaller request is clamped up to this (and a
/// larger one down to [`MAX_MAX_DATAGRAM`]).
pub const MIN_MAX_DATAGRAM: usize = 64;
/// Largest accepted [`IoConfig::max_datagram`]: the biggest UDP payload.
pub const MAX_MAX_DATAGRAM: usize = 65_535;

/// Timeouts and sizes for the tokio adapter ([`SrtSocket::connect_with`],
/// [`SrtListener::bind_with`]).
///
/// Every field has a documented default; use the `with_*` builders so new
/// fields can be added without breaking callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IoConfig {
    /// Largest datagram accepted from the network (bytes). Default 1500. A
    /// larger datagram is dropped and counted in [`SocketStats::rx_oversize`]
    /// (UDP would otherwise truncate it silently). Values below
    /// [`MIN_MAX_DATAGRAM`] are clamped up to it.
    pub max_datagram: usize,
    /// Bound on resolving the remote address and binding the local socket.
    /// Default 10 s.
    pub connect: Duration,
    /// Budget for the whole caller handshake. Default 5 s (the previous
    /// `HANDSHAKE_TIMEOUT` constant).
    pub handshake: Duration,
    /// Longest silence from a connected peer before the connection is torn
    /// down. Default 5 s: libsrt's `SRTO_PEERIDLETIMEO` default of 5000 ms (the
    /// previous `PEER_IDLE_TIMEOUT` constant).
    pub read_idle: Duration,
    /// Longest a single `send_to` may take. Default 5 s.
    pub write: Duration,
}

impl Default for IoConfig {
    fn default() -> Self {
        IoConfig {
            max_datagram: DEFAULT_MAX_DATAGRAM,
            connect: Duration::from_secs(10),
            handshake: Duration::from_secs(5),
            read_idle: Duration::from_secs(5),
            write: Duration::from_secs(5),
        }
    }
}

impl IoConfig {
    /// Set [`max_datagram`](Self::max_datagram), clamped up to [`MIN_MAX_DATAGRAM`].
    #[must_use]
    pub fn with_max_datagram(mut self, n: usize) -> Self {
        self.max_datagram = n.clamp(MIN_MAX_DATAGRAM, MAX_MAX_DATAGRAM);
        self
    }

    /// Set [`connect`](Self::connect).
    #[must_use]
    pub fn with_connect(mut self, d: Duration) -> Self {
        self.connect = d;
        self
    }

    /// Set [`handshake`](Self::handshake).
    #[must_use]
    pub fn with_handshake(mut self, d: Duration) -> Self {
        self.handshake = d;
        self
    }

    /// Set [`read_idle`](Self::read_idle).
    #[must_use]
    pub fn with_read_idle(mut self, d: Duration) -> Self {
        self.read_idle = d;
        self
    }

    /// Set [`write`](Self::write).
    #[must_use]
    pub fn with_write(mut self, d: Duration) -> Self {
        self.write = d;
        self
    }
}

/// Reads one datagram into the reusable `scratch` buffer (length
/// `max + 1`) and returns it as an exactly-sized `Bytes`, or `None` for an
/// oversize datagram (which has been counted and dropped).
///
/// The copy-out is deliberate: a `Bytes` that was a view into a shared receive
/// chunk would keep the WHOLE chunk alive for as long as that one packet is
/// staged behind a gap, queued for the application or held by it (on a shared
/// listener socket, interleaved connections multiply that). One small
/// allocation per datagram bounds retained memory to the datagrams actually
/// held; the payload is still not copied again on its way to the application
/// (`Driver::release` slices it).
async fn recv_datagram(
    udp: &UdpSocket,
    scratch: &mut [u8],
    max: usize,
    oversize: &AtomicU64,
) -> std::io::Result<Option<(Bytes, std::net::SocketAddr)>> {
    // `scratch` is `max + 1` long: a datagram of max+1 bytes proves "larger
    // than max" (UDP would otherwise truncate it silently).
    let (n, src) = udp.recv_from(scratch).await?;
    if n > max {
        oversize.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    Ok(Some((Bytes::copy_from_slice(&scratch[..n]), src)))
}

/// One [`RouteTable`] entry: the accepted connection's ingress channel, plus
/// the source address it was accepted from. libsrt demultiplexes its
/// multiplexed listener socket by Destination Socket ID, not source address
/// (draft §4.3.1; `CRcvQueue::worker` dispatches on it in `srtcore/core.cpp`)
/// — the address is kept here only to *check* it against the datagram that
/// claims this ID (see [`routing_pump`]), so a spoofed ID from
/// the wrong peer cannot be injected into someone else's session.
#[derive(Debug, Clone)]
struct RouteEntry {
    addr: std::net::SocketAddr,
    tx: mpsc::Sender<Bytes>,
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
type RouteTable = Arc<Mutex<std::collections::HashMap<u32, RouteEntry>>>;

/// Lock `mutex`, recovering the guard if a previous holder panicked.
///
/// A poisoned lock must never turn into a second panic — least of all inside
/// a `Drop` (see [`RouteCleanup`]) or the routing pump, which would then take
/// the whole listener down for one panicking connection task. Every critical
/// section here is a single `HashMap` insert/remove/get that cannot leave the
/// map half-updated, so the state behind a poisoned lock is still consistent
/// (issue #1134).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

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
            lock(&routes).remove(&id);
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
/// buffering forever (when TLPKTDROP was negotiated, §4.6).
const DELIVER_CHANNEL_CAPACITY: usize = 4096;
/// Capacity of the application-to-driver payload channel
/// ([`SrtSocket::send`]). Bounded so the application is paced by the driver
/// instead of queueing without limit; the driver additionally stops draining
/// it while the sender holds a full flow window of unacknowledged packets
/// (see `Driver::send_window_open`).
const SEND_CHANNEL_CAPACITY: usize = 1024;

/// Floor on how soon [`Driver::poll_timeout`] may ask to be woken: never a
/// deadline in the past, and a 1 ms minimum cannot spin (it matches tokio's
/// timer granularity).
const MIN_WAKE: Duration = Duration::from_millis(1);
/// Most DATA packets one `flush_outbound` pass sends, and the most pacing
/// periods of catch-up credit a late wake may claim: with the ~1 ms timer
/// granularity (`MIN_WAKE`) this lifts the idle-backlog drain ceiling from
/// ~1000 packets/s to `PACING_BURST_CAP` x 1000 packets/s without ever
/// dumping more than this many packets at once.
const PACING_BURST_CAP: usize = 16;
/// Initial TSBPD drift (zero: the scheduler estimates it from there).
const DEFAULT_DRIFT_US: i64 = 0;
/// Default max bandwidth (1 Gbps).
const DEFAULT_MAX_BW: MaxBwConfig = MaxBwConfig::Set(125_000_000);
/// How often an unanswered handshake request is re-sent. libsrt re-sends its
/// handshake request every 250 ms until it is answered; the previous 15 s
/// (three five-second receive timeouts) made one lost INDUCTION cost the
/// whole connect budget.
const HANDSHAKE_RETRANSMIT_PERIOD: Duration = Duration::from_millis(250);
/// Floor on the handshake tick period, so an absurd
/// [`HandshakeConfig::retransmit_after_ticks`] cannot make it zero (a zero
/// `tokio::time::interval` period panics).
const MIN_HANDSHAKE_TICK: Duration = Duration::from_millis(1);
/// Keep-Alive period (`draft-sharabayko-srt-01` §3.2.3: "The default timeout
/// for a keep-alive packet to be sent is 1 second", measured from the last
/// time any packet — control or data — was sent).
const KEEPALIVE_PERIOD: Duration = Duration::from_secs(1);
/// Upper bound on concurrently in-progress listener handshakes. Every new
/// source address that sends an INDUCTION allocates a [`PendingListener`]
/// entry; without a cap, a flood of (possibly spoofed) handshake packets from
/// many sources exhausts memory before any of them times out.
const MAX_PENDING: usize = 1024;
/// How often [`SrtListener::accept`] drives pending-handshake expiry and
/// retransmits, independent of receive activity.
const PENDING_TICK_INTERVAL: Duration = Duration::from_millis(100);
/// How often the SYN-cookie key is replaced (§4.3.1.1 buckets the cookie's
/// time input to one minute).
const COOKIE_KEY_ROTATION: Duration = Duration::from_secs(60);
/// How long an accepted connection's CONCLUSION response is remembered so a
/// repeated CONCLUSION (the response was lost) can be answered again — a
/// little longer than the Caller's own default connect budget ([`IoConfig::handshake`]).
const RECENT_ACCEPT_TTL: Duration = Duration::from_secs(10);
/// Cap on remembered CONCLUSION responses (one per recently accepted source
/// address), the same bound as [`MAX_PENDING`].
const MAX_RECENT_ACCEPTS: usize = MAX_PENDING;

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
    late_dropped: AtomicU64,
    rx_oversize: AtomicU64,
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
    /// DATA packets ignored because they were not ahead of the delivery
    /// cursor: duplicates, retransmissions of what was already delivered or
    /// given up on, and stale sequence numbers (never staged).
    pub late_dropped: u64,
    /// Datagrams larger than [`IoConfig::max_datagram`], dropped rather than
    /// truncated.
    pub rx_oversize: u64,
}

// ===========================================================================
// SrtSocket — a handle to an established SRT connection
// ===========================================================================

/// What [`SrtSocket::spawn`] needs to build one connection's engines and
/// driver task.
struct ConnParams {
    udp: Arc<UdpSocket>,
    peer_addr: std::net::SocketAddr,
    rx: mpsc::Receiver<Bytes>,
    stats: Arc<Counters>,
    route_cleanup: RouteCleanup,
    forwarder_guard: TaskGuard,
    /// For a listener-accepted connection: keeps the listener's tasks (and so
    /// its socket) alive for as long as this connection is.
    life: Option<Arc<ListenerLife>>,
    /// [`IoConfig::read_idle`] for this connection.
    read_idle: Duration,
    /// [`IoConfig::write`] for this connection.
    write: Duration,
    our_initial_seq: u32,
    peer_initial_seq: u32,
    peer_socket_id: u32,
    /// `TsbpdTimeBase` (§4.5.1.1, rule 12): `T_NOW - HSREQ_TIMESTAMP`, with
    /// `T_NOW` measured from `epoch` — so `-(the peer's handshake timestamp)`
    /// when `epoch` is the instant that handshake packet arrived.
    tsbpd_time_base: i64,
    tsbpd_delay_ms: u64,
    epoch: Instant,
    /// The negotiated Maximum Flow Window Size (§3.2.1): the smaller of the
    /// two sides' values.
    max_flow_window: u32,
    /// The negotiated MTU (§3.2.1): the smaller of the two sides' values.
    mtu: u32,
    /// Whether Too-Late Packet Drop was negotiated (the `TLPKTDROP` flag both
    /// sides advertised, §3.2.1.1.1): only then may the receiver skip a gap.
    tlpkt_drop: bool,
}

/// A handle to an established SRT connection over UDP.
///
/// Created by [`SrtSocket::connect`] (caller role) or
/// [`SrtListener::accept`] (listener role). The protocol itself runs on a
/// background [`tokio`] task the handle owns; [`send`](Self::send) enqueues an
/// application payload and [`recv`](Self::recv) awaits a delivered one.
/// Dropping the handle sends the peer a SHUTDOWN (§3.2.7) and aborts the
/// driver task.
#[derive(Debug)]
pub struct SrtSocket {
    peer_addr: std::net::SocketAddr,
    /// Application payloads flowing to the driver task for transmission.
    to_driver: mpsc::Sender<Bytes>,
    /// TSBPD/ARQ-delivered payloads flowing back from the driver task.
    from_driver: mpsc::Receiver<Bytes>,
    /// The driver task; aborted on drop.
    driver: Option<tokio::task::JoinHandle<()>>,
    stats: Arc<Counters>,
    /// The connection's socket, kept to send the SHUTDOWN from `Drop`.
    udp: Arc<UdpSocket>,
    /// The peer's SRT Socket ID — the SHUTDOWN's Destination Socket ID.
    peer_socket_id: u32,
    /// The connection epoch, for the SHUTDOWN's wire timestamp.
    epoch: Instant,
    /// Keeps a listener's background tasks alive while this accepted
    /// connection is; `None` for a [`connect`](Self::connect)ed caller.
    /// Declared last so it drops after the SHUTDOWN in `Drop`.
    _life: Option<Arc<ListenerLife>>,
}

impl SrtSocket {
    /// Connect to a remote SRT peer as a Caller.
    ///
    /// # Encryption
    /// A [`HandshakeConfig`] with `crypto` set is refused with
    /// [`Error::EncryptionUnsupported`] before any network I/O: this
    /// adapter's data path does not encrypt, so an encrypted connection must
    /// not be silently downgraded to plaintext. The sans-IO engines still
    /// negotiate keys; the adapter will support encryption once its data path
    /// does.
    ///
    /// # Errors
    /// [`Error::Rejected`] carries the peer's [`RejectionReason`];
    /// [`Error::HandshakeTimedOut`] means the peer never answered within the
    /// handshake budget; [`Error::Handshake`] wraps an engine failure with the
    /// stage it happened in.
    pub async fn connect<A: tokio::net::ToSocketAddrs>(
        remote_addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        Self::connect_with(remote_addr, config, IoConfig::default()).await
    }

    /// Like [`Self::connect`], with explicit timeouts and datagram size.
    pub async fn connect_with<A: tokio::net::ToSocketAddrs>(
        remote_addr: A,
        config: HandshakeConfig,
        io: IoConfig,
    ) -> Result<Self> {
        refuse_crypto(&config)?;
        let local = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
        Self::connect_from_with(local, remote_addr, config, io).await
    }

    /// Connect from a specific local address.
    ///
    /// # Encryption
    /// Like [`Self::connect`], refuses a `crypto`-enabled
    /// [`HandshakeConfig`] with [`Error::EncryptionUnsupported`] before any I/O.
    pub async fn connect_from<A: tokio::net::ToSocketAddrs>(
        local_addr: std::net::SocketAddr,
        remote_addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        Self::connect_from_with(local_addr, remote_addr, config, IoConfig::default()).await
    }

    /// Like [`Self::connect_from`], with explicit timeouts and datagram size.
    pub async fn connect_from_with<A: tokio::net::ToSocketAddrs>(
        local_addr: std::net::SocketAddr,
        remote_addr: A,
        config: HandshakeConfig,
        io: IoConfig,
    ) -> Result<Self> {
        refuse_crypto(&config)?;
        let max_datagram = io.max_datagram.clamp(MIN_MAX_DATAGRAM, MAX_MAX_DATAGRAM);
        let socket = bounded(io.connect, "bind", async {
            UdpSocket::bind(local_addr)
                .await
                .map_err(|e| io_err("bind", e))
        })
        .await?;
        let peer = bounded(io.connect, "resolve", resolve_one(remote_addr)).await?;
        let socket = Arc::new(socket);

        // Both this Caller's own Socket ID and its ISN must be freshly
        // generated per connection, not carried over from `config` (whose
        // `initial_seq_number` a caller may leave at its `0` default) —
        // `draft-sharabayko-srt-01` §3/§4.3.1.1 expects both random, and a
        // fixed or predictable pair lets an off-path party that knows the
        // 4-tuple guess a live connection's wire identifiers outright.
        let own_socket_id = random_socket_id()?;
        let mut config = config;
        config.initial_seq_number = random_isn()?;
        let mut hs = CallerHandshake::new(own_socket_id, config.clone());

        // Send INDUCTION.
        let induction = hs.start().map_err(|e| handshake_err("caller start", e))?;
        send_bounded(&socket, &induction, peer, io.write, "send induction").await?;

        // Re-send an unanswered request every `HANDSHAKE_RETRANSMIT_PERIOD`
        // (the engine counts `retransmit_after_ticks` ticks per retransmit),
        // and give up at `IoConfig::handshake` whatever the retry budget says.
        let tick_period = (HANDSHAKE_RETRANSMIT_PERIOD / config.retransmit_after_ticks.max(1))
            .max(MIN_HANDSHAKE_TICK);
        let mut ticker = tokio::time::interval_at(Instant::now() + tick_period, tick_period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let deadline = Instant::now() + io.handshake;
        let mut scratch = vec![0u8; max_datagram + 1];
        // The handshake loop has no `SocketStats` yet; oversize datagrams are
        // simply ignored (the real peer retransmits).
        let hs_oversize = AtomicU64::new(0);

        loop {
            enum Event {
                Datagram(Bytes, std::net::SocketAddr),
                Oversize,
                Tick,
                Deadline,
            }
            let event = tokio::select! {
                r = recv_datagram(&socket, &mut scratch, max_datagram, &hs_oversize) => {
                    match r.map_err(|e| io_err("recv hs", e))? {
                        Some((bytes, src)) => Event::Datagram(bytes, src),
                        None => Event::Oversize,
                    }
                }
                _ = ticker.tick() => Event::Tick,
                _ = tokio::time::sleep_until(deadline) => Event::Deadline,
            };

            let (outputs, received) = match event {
                Event::Deadline => {
                    return Err(Error::HandshakeTimedOut {
                        stage: "caller connect",
                    });
                }
                Event::Oversize => continue,
                Event::Tick => (hs.tick(), None),
                Event::Datagram(bytes, src) => {
                    // A packet from anyone but the peer we're connecting to
                    // (some unrelated traffic that happened to land on this
                    // freshly bound port) must not fail the connect — ignore
                    // it and keep waiting for the real peer.
                    if src != peer {
                        continue;
                    }
                    // Likewise, a packet that isn't a well-formed Handshake
                    // control packet right now (a stray Keep-Alive, a
                    // resent/duplicate reply, garbage) must not fail the
                    // connect either — only a genuine handshake rejection or
                    // timeout does that.
                    let Ok(outputs) = hs.feed_bytes(&bytes) else {
                        continue;
                    };
                    (outputs, Some(bytes))
                }
            };

            for outcome in outputs {
                match outcome {
                    HandshakeOutput::Send(bytes) => {
                        send_bounded(&socket, &bytes, peer, io.write, "send hs").await?;
                    }
                    HandshakeOutput::Connected(params) => {
                        // `Connected` only ever comes out of `feed_bytes`, so
                        // the datagram that completed the handshake is still
                        // in `buf`. The peer's ISN (seeds ARQ/TSBPD sequence
                        // tracking) and handshake timestamp (seeds the TSBPD
                        // time base, rule 12) are carried in those bytes; the
                        // peer's SRT Socket ID (the wire `dest_socket_id`
                        // for every outgoing packet) is the negotiated
                        // `params.peer_socket_id` — the two ids are unrelated
                        // values (§3).
                        //
                        // `peer_handshake` errors (rather than defaulting to
                        // 0) if the very bytes that just drove the handshake
                        // state machine to `Connected` fail to re-parse as a
                        // Handshake control packet — an internal
                        // inconsistency. A silent `0` fallback would be
                        // indistinguishable from a genuine ISN of 0 and would
                        // seed ARQ/TSBPD sequence tracking wrong for the life
                        // of the connection.
                        let received = received.ok_or(Error::InvalidField {
                            what: "handshake",
                            reason: "Connected without a received datagram",
                        })?;
                        let peer_hs = peer_handshake(&received)?;
                        let epoch = Instant::now();
                        // This socket is exclusively this connection's own
                        // (bound fresh in `connect_from`, never shared) — no
                        // demux table needed, just a forwarder task unifying
                        // it with the same `Driver::rx` path a
                        // listener-accepted connection uses (issue #1029).
                        let stats = Arc::new(Counters::default());
                        let (rx, forwarder) = spawn_dedicated_socket_forwarder(
                            Arc::clone(&socket),
                            peer,
                            Arc::clone(&stats),
                            max_datagram,
                        );
                        return Ok(SrtSocket::spawn(ConnParams {
                            udp: socket,
                            peer_addr: peer,
                            rx,
                            stats,
                            route_cleanup: RouteCleanup(None),
                            forwarder_guard: TaskGuard(Some(forwarder)),
                            life: None,
                            read_idle: io.read_idle,
                            write: io.write,
                            our_initial_seq: config.initial_seq_number,
                            peer_initial_seq: peer_hs.initial_seq_number,
                            peer_socket_id: params.peer_socket_id,
                            tsbpd_time_base: tsbpd_time_base_from(peer_hs.timestamp_us),
                            tsbpd_delay_ms: u64::from(params.latency_ms),
                            epoch,
                            max_flow_window: params.max_flow_window_size,
                            mtu: params.mtu,
                            tlpkt_drop: params.flags.tlpktdrop(),
                        }));
                    }
                    HandshakeOutput::Rejected(reason) => return Err(Error::Rejected(reason)),
                    HandshakeOutput::TimedOut => {
                        return Err(Error::HandshakeTimedOut {
                            stage: "caller retransmit budget",
                        });
                    }
                }
            }
        }
    }

    /// Build the engine state, spawn its background driver task, and return
    /// the [`SrtSocket`] handle wired to it.
    fn spawn(p: ConnParams) -> Self {
        let (to_driver, app_out) = mpsc::channel::<Bytes>(SEND_CHANNEL_CAPACITY);
        let (deliver, from_driver) = mpsc::channel(DELIVER_CHANNEL_CAPACITY);

        let udp = Arc::clone(&p.udp);
        let (peer_addr, peer_socket_id, epoch) = (p.peer_addr, p.peer_socket_id, p.epoch);
        let stats = Arc::clone(&p.stats);
        let life = p.life.clone();
        let driver = Driver::new(p, deliver);
        let handle = tokio::spawn(driver.run(app_out));

        SrtSocket {
            peer_addr,
            to_driver,
            from_driver,
            driver: Some(handle),
            stats,
            udp,
            peer_socket_id,
            epoch,
            _life: life,
        }
    }

    /// Enqueue a payload for transmission to the peer.
    ///
    /// Returns once the payload is handed to the driver task — actual
    /// transmission, ACK/NAK handling, and retransmission all happen on that
    /// task. Waits (applying backpressure) while the sender already holds a
    /// full flow window of unacknowledged packets or the hand-off queue is
    /// full. Fails only if the driver task has stopped (peer shut down, went
    /// silent, or a connection error).
    pub async fn send(&mut self, payload: &[u8]) -> Result<()> {
        self.send_bytes(Bytes::copy_from_slice(payload)).await
    }

    /// Like [`send`](Self::send) but takes ownership of an already-built
    /// [`Bytes`], so the hand-off to the driver task does not copy it.
    pub async fn send_bytes(&mut self, payload: Bytes) -> Result<()> {
        self.to_driver.send(payload).await.map_err(|_| Error::Io {
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
    pub async fn recv(&mut self) -> Result<Option<Bytes>> {
        Ok(self.from_driver.recv().await)
    }

    /// A snapshot of this connection's internal-backpressure drop counters
    /// (item 4) — datagrams or delivered payloads this adapter's own bounded
    /// channels dropped because they were full, not wire-level loss.
    pub fn stats(&self) -> SocketStats {
        SocketStats {
            rx_dropped: self.stats.rx_dropped.load(Ordering::Relaxed),
            deliver_dropped: self.stats.deliver_dropped.load(Ordering::Relaxed),
            late_dropped: self.stats.late_dropped.load(Ordering::Relaxed),
            rx_oversize: self.stats.rx_oversize.load(Ordering::Relaxed),
        }
    }
}

impl Drop for SrtSocket {
    fn drop(&mut self) {
        if let Some(handle) = self.driver.take() {
            // Tell a still-connected peer we are leaving (§3.2.7) so it can
            // close at once instead of waiting out its idle timeout. Not when
            // the driver already ended: the peer shut down (or went silent)
            // and has nothing left to be told.
            if !handle.is_finished() {
                let timestamp = crate::arq::duration_to_wire_us(
                    Instant::now().saturating_duration_since(self.epoch),
                );
                send_shutdown(&self.udp, self.peer_addr, self.peer_socket_id, timestamp);
            }
            handle.abort();
        }
    }
}

/// The SHUTDOWN datagram (§3.2.7) for a peer.
fn shutdown_datagram(peer_socket_id: u32, timestamp: u32) -> Option<Vec<u8>> {
    let pkt = ControlPacket::Shutdown(ShutdownPacket {
        timestamp,
        dest_socket_id: peer_socket_id,
        libsrt_pad: true,
    });
    let mut buf = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut buf).ok()?;
    Some(buf)
}

/// Best-effort, non-blocking SHUTDOWN (§3.2.7) to `peer`, for `Drop`, where
/// nothing can be awaited. A failure (socket buffer full, peer unreachable, or
/// a socket that was never polled for writability) is ignored: the peer's idle
/// timeout is the fallback.
fn send_shutdown(udp: &UdpSocket, peer: std::net::SocketAddr, peer_socket_id: u32, timestamp: u32) {
    if let Some(buf) = shutdown_datagram(peer_socket_id, timestamp) {
        let _ = udp.try_send_to(&buf, peer);
    }
}

// ===========================================================================
// Driver — the per-connection background task
// ===========================================================================

/// One packet held by the receive staging buffer until TSBPD releases it: the
/// whole datagram, header included. Keeping the datagram's own allocation —
/// rather than copying the payload out of it — means a received packet is
/// copied once (socket buffer to this `Vec`) and then handed to the
/// application with the header drained off the front, never copied into a
/// second allocation.
type StagedDatagram = Bytes;

/// The engine state driven by one connection's background task. Owns the
/// socket, the sans-IO ARQ/TSBPD/LiveCC engines, and the outbound queue; runs
/// the RX / app-send / next-deadline select loop in [`Driver::run`].
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
    rx: mpsc::Receiver<Bytes>,
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
    // Absolute time the last datagram of any kind was sent to the peer — the
    // basis for the Keep-Alive timer (§3.2.3).
    last_tx_at: Instant,

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

    // Staging: seq → received datagram, released to `deliver` by TSBPD/ARQ.
    staged: std::collections::BTreeMap<u32, StagedDatagram>,

    // Negotiated maximum flow window (§3.2.1): the cap on how far ahead of
    // the delivery cursor a packet may be staged, and on how many
    // unacknowledged packets the sender may hold.
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
    deliver: mpsc::Sender<Bytes>,

    peer_shutdown: bool,

    /// Longest silence from the peer before the connection ends
    /// ([`IoConfig::read_idle`]).
    read_idle: Duration,
    /// Longest a single `send_to` may take ([`IoConfig::write`]).
    write_timeout: Duration,

    /// Test hook: counts `tick_engines` calls, so a test can prove the run
    /// loop wakes at deadlines rather than on a fixed tick.
    #[cfg(test)]
    tick_counter: Option<Arc<AtomicU64>>,
}

impl Driver {
    fn new(p: ConnParams, deliver: mpsc::Sender<Bytes>) -> Self {
        let now = Instant::now();
        Driver {
            udp: p.udp,
            peer_addr: p.peer_addr,
            peer_socket_id: p.peer_socket_id,
            stats: p.stats,
            rx: p.rx,
            _route_cleanup: p.route_cleanup,
            _forwarder_guard: p.forwarder_guard,
            last_rx_at: now,
            last_tx_at: now,
            // `dest_socket_id` on every outgoing DATA/ACKACK/NAK/ACK packet
            // must be the peer's negotiated SRT Socket ID, not its ISN — the
            // two are unrelated values (§3).
            sender: ArqSender::new(p.peer_socket_id),
            receiver: ArqReceiver::new(p.peer_socket_id, p.peer_initial_seq, p.max_flow_window)
                .with_mtu(p.mtu),
            tsbpd: TsbpdScheduler::new(
                p.peer_initial_seq,
                p.tsbpd_time_base,
                p.tsbpd_delay_ms,
                DEFAULT_DRIFT_US,
                p.tlpkt_drop,
                None,
            ),
            livecc: LiveCC::new(DEFAULT_MAX_BW),
            next_message_number: 1,
            next_send_seq: p.our_initial_seq,
            epoch: p.epoch,
            staged: std::collections::BTreeMap::new(),
            max_flow_window: p.max_flow_window,
            outbound_data: VecDeque::new(),
            outbound_control: VecDeque::new(),
            next_data_send_at: None,
            deliver,
            peer_shutdown: false,
            read_idle: p.read_idle,
            write_timeout: p.write,
            #[cfg(test)]
            tick_counter: None,
        }
    }

    /// The next instant at which this connection must act without any inbound
    /// event: the next Full ACK / NAK, the next TSBPD release (or too-late
    /// skip), the Keep-Alive when idle, the peer-idle expiry and the next
    /// pacing slot. Never in the past (floored at [`MIN_WAKE`]).
    fn poll_timeout(&self) -> Option<Instant> {
        let now = self.elapsed();
        // Full ACK / NAK.
        let mut at = self.epoch + self.receiver.next_timeout();
        // TSBPD release / TLPKTDROP skip.
        if let Some(t) = self.tsbpd.next_release_after(now) {
            at = at.min(self.epoch + t);
        }
        // Keep-Alive (only judged when nothing is queued).
        if self.outbound_control.is_empty() && self.outbound_data.is_empty() {
            at = at.min(self.last_tx_at + KEEPALIVE_PERIOD);
        }
        // Peer idle.
        at = at.min(self.last_rx_at + self.read_idle);
        // Pacing slot.
        if let Some(wait) = self.next_paced_send_wait() {
            at = at.min(Instant::now() + wait);
        }
        Some(at.max(Instant::now() + MIN_WAKE))
    }

    /// The select loop: socket RX, application-send, and a periodic engine
    /// tick — the tick arm is what keeps retransmit/ACK/NAK progressing when
    /// neither peer is actively sending application data.
    async fn run(mut self, mut app_out: mpsc::Receiver<Bytes>) {
        let mut app_open = true;

        // Set when the application handle is gone (its `to_driver` sender was
        // dropped): the loop makes one final flush pass and then exits, so a
        // dropped [`SrtSocket`] does not leave a driver task parked forever on
        // the socket/timer arms — which would keep a current-thread runtime
        // from shutting down. (`SrtSocket::Drop` also aborts the task; this is
        // the cooperative path that does not rely on abort-during-shutdown.)
        let mut shutting_down = false;

        loop {
            // The next instant this connection must act on its own (ACK/NAK,
            // TSBPD release, keep-alive, peer idle, pacing slot): the loop
            // sleeps until then or until an event, never on a fixed tick.
            let wake_at = self
                .poll_timeout()
                .unwrap_or_else(|| Instant::now() + KEEPALIVE_PERIOD);
            // Stop taking application payloads while a full flow window of
            // packets is still unacknowledged: the hand-off channel then
            // fills and `SrtSocket::send` waits, instead of the send buffer
            // growing without bound behind a stalled or dead peer.
            let accept_app_data = app_open && self.send_window_open();

            tokio::select! {
                // Application handed us a payload to send.
                maybe = app_out.recv(), if accept_app_data => {
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
                            let _ = self.ingress(bytes);
                        }
                        None => break, // upstream demux/forwarder is gone.
                    }
                }
                // Deadline: retransmit, ACK, NAK, TSBPD release, keep-alive
                // and the pacing slot are all folded into `poll_timeout`.
                _ = tokio::time::sleep_until(wake_at) => {}
            }

            // Timer-driven engine work runs after EVERY wake-up: it is a pure
            // function of `now`, so an inbound packet also gets its
            // ACK/NAK/TSBPD release without waiting for a tick.
            self.tick_engines();

            if self.flush_outbound().await.is_err() {
                break;
            }
            if self.peer_idle(Instant::now()) {
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
        // A cooperative exit because *we* are leaving (not the peer's
        // SHUTDOWN, nor an idle peer, nor a socket error) tells the peer
        // (§3.2.7); `SrtSocket::drop` does the same for the abort path.
        if shutting_down
            && !self.peer_shutdown
            && let Some(buf) = shutdown_datagram(self.peer_socket_id, self.elapsed_us())
        {
            let _ = send_bounded(&self.udp, &buf, self.peer_addr, self.write_timeout, "send").await;
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

    /// No packet at all has arrived from the peer for [`IoConfig::read_idle`].
    fn peer_idle(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_rx_at) >= self.read_idle
    }

    /// Room in the send window: fewer than a full flow window of packets are
    /// buffered unacknowledged (§3.2.1 Maximum Flow Window Size).
    fn send_window_open(&self) -> bool {
        let window = usize::try_from(self.max_flow_window)
            .unwrap_or(usize::MAX)
            .max(1);
        self.sender.buffered_count() < window
    }

    fn send_one(&mut self, payload: &[u8]) {
        let now = self.elapsed();
        self.livecc
            .on_data_packet(u64::try_from(payload.len()).unwrap_or(u64::MAX));
        // The counters are kept within their wire widths below, so this cannot
        // be refused; if it ever were, the payload is dropped, not the task.
        let Ok(bytes) =
            self.sender
                .on_data(self.next_send_seq, self.next_message_number, payload, now)
        else {
            return;
        };
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

    /// Process one datagram from the peer. Takes the datagram by value so a
    /// DATA packet's own allocation can be staged without a copy.
    fn ingress(&mut self, datagram: Bytes) -> Result<()> {
        // Any datagram from the peer — even one that fails to parse below —
        // is evidence the peer is still there, resetting the idle timeout.
        self.last_rx_at = Instant::now();
        let now = self.elapsed();
        match SrtPacket::parse(&datagram)? {
            SrtPacket::Data(d) => {
                let (seq_number, timestamp, key_flag) = (d.seq_number, d.timestamp, d.key_flag);
                self.ingress_data(datagram, seq_number, timestamp, key_flag, now);
            }
            SrtPacket::Control(c) => self.ingress_control(&c, now),
        }
        Ok(())
    }

    fn ingress_data(
        &mut self,
        datagram: Bytes,
        seq_number: u32,
        timestamp: u32,
        key_flag: EncryptionKeyField,
        now: Duration,
    ) {
        // The adapter never decrypts, so a ciphertext packet must not reach
        // the application (§3.1 `KK`, §6): drop it before ARQ or TSBPD see it
        // (acknowledging data we can never deliver would be worse than
        // dropping it).
        if key_flag != EncryptionKeyField::NotEncrypted {
            return;
        }

        // The ARQ receiver drives *reliability* only: loss detection and the
        // resulting NAK (rules 4, 14) plus the ACK point. Application
        // delivery is the TSBPD scheduler's job — it is the single in-order
        // delivery authority (see below), so its `outcome.delivered` is
        // intentionally NOT used to deliver here. Running both cursors over
        // one `staged` map races them and reorders retransmitted packets.
        //
        // Flow-window overflow guard (§3.2.1): a packet further ahead of the
        // ack point than the negotiated maximum flow window can never be
        // released in this connection's lifetime — staging it would let an
        // unrecoverable gap grow memory without bound, so the receiver
        // refuses it and nothing downstream sees it.
        //
        // A packet that is not ahead of the delivery cursor (a duplicate, a
        // retransmission of what was delivered or given up on, or an
        // attacker-chosen stale number with a future timestamp) has nothing
        // left to do: TSBPD would ignore it and nothing would ever remove it
        // from `staged`, so it is never staged at all.
        if seq_lt(seq_number, self.tsbpd.next_release()) {
            self.stats.late_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let outcome = self.receiver.feed_data(seq_number, now);
        if outcome.out_of_window {
            return;
        }
        if let Some(nak_bytes) = outcome.nak {
            self.outbound_control.push_back(nak_bytes);
        }

        self.staged.entry(seq_number).or_insert(datagram);

        // TSBPD is the sole delivery cursor: it releases packets in strict
        // sequence order, waiting for a NAK-recovered gap to be filled —
        // until the next available packet reaches its play time, at which
        // point live mode skips the unrecoverable gap and delivers that
        // packet (only if TLPKTDROP was negotiated, §3.2.1.1.1; otherwise it waits for the
        // retransmission, and the ARQ ack point never moves past the gap).
        let tsbpd_out = self.tsbpd.feed_data(seq_number, timestamp, now);
        self.release(&tsbpd_out);
    }

    fn ingress_control(&mut self, c: &ControlPacket<'_>, now: Duration) {
        match c {
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
            // A Keep-Alive only tells us the peer is alive (`last_rx_at` is
            // already refreshed): it is not answered. Echoing it made two of
            // these adapters bounce a Keep-Alive back and forth forever now
            // that an idle connection sends its own every second.
            ControlPacket::KeepAlive(_) => {}
            ControlPacket::Shutdown(_) => {
                self.peer_shutdown = true;
            }
            _ => {}
        }
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
    ///
    /// What the scheduler gave up on (§4.6) is also skipped in the ARQ
    /// receiver: its ack point then moves past the dropped packets — the
    /// "fake ACK" of Too-Late Packet Drop — so it stops NAKing packets that
    /// can no longer be delivered, the ACKs it sends let the peer free them,
    /// and the flow window it measures new arrivals against advances with the
    /// delivery cursor instead of stalling at the lost packet forever.
    fn release(&mut self, outcome: &TickOutcome) {
        for &seq in &outcome.delivered {
            if let Some(datagram) = self.staged.remove(&seq) {
                // The staged datagram is a DATA packet (it parsed as one):
                // hand over the payload as a view into the same allocation.
                let payload = datagram.slice(SRT_HEADER_LEN.min(datagram.len())..);
                if self.deliver.try_send(payload).is_err() {
                    // Full (app isn't calling `recv` fast enough) or the
                    // handle is gone; either way, drop rather than buffer
                    // without bound (item 4) and count it.
                    self.stats.deliver_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        for &seq in &outcome.dropped {
            self.staged.remove(&seq);
        }
        for (first, last) in contiguous_runs(&outcome.dropped) {
            self.receiver.skip_range(first, last);
        }
    }

    fn tick_engines(&mut self) {
        #[cfg(test)]
        if let Some(counter) = &self.tick_counter {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        let now = self.elapsed();

        // Retransmitted DATA packets first (rules 5, 15, 16, 18): they are
        // drained from the NAK-populated loss list and queued *before* any
        // new first-time data appended later this cycle, reproducing the
        // sans-IO engine's "loss list before first transmission" priority
        // (see `arq::sender`'s module doc). Feed LiveCC the same as a
        // first-time send (`specs/rules/srt-livecc.md` §5.1.2, L3216-3217:
        // "original or retransmitted") and tag them `is_data` so pacing
        // applies. The payload length is the datagram less its fixed header —
        // no need to re-parse the packet just to learn it.
        for bytes in self.sender.tick(now) {
            self.livecc.on_data_packet(
                u64::try_from(bytes.len().saturating_sub(SRT_HEADER_LEN)).unwrap_or(u64::MAX),
            );
            self.outbound_data.push_back(bytes);
        }

        // Keep-Alive (§3.2.3): a connection that has sent nothing for a
        // second says so, or the peer's own idle timeout ends it. Judged
        // before this tick's ACK/NAK are queued: it asks whether anything has
        // gone out for a second, not whether something is about to. (The
        // receiver's 10 ms Full ACK normally keeps `last_tx_at` fresh, so
        // this is the backstop for when it does not.)
        if self.outbound_control.is_empty()
            && self.outbound_data.is_empty()
            && Instant::now().saturating_duration_since(self.last_tx_at) >= KEEPALIVE_PERIOD
        {
            self.queue_keepalive();
        }

        // Periodic ACK/NAK (rules 11, 12, 21, 22): control feedback, never
        // paced.
        for bytes in self.receiver.tick(now) {
            self.outbound_control.push_back(bytes);
        }

        let tsbpd_out = self.tsbpd.tick(now);
        self.release(&tsbpd_out);
    }

    fn queue_keepalive(&mut self) {
        let pkt = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: self.elapsed_us(),
            dest_socket_id: self.peer_socket_id,
            libsrt_pad: true,
        });
        let mut buf = vec![0u8; pkt.serialized_len()];
        if pkt.serialize_into(&mut buf).is_ok() {
            self.outbound_control.push_back(buf);
        }
    }

    fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(self.epoch)
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
            send_bounded(
                &self.udp,
                &bytes,
                self.peer_addr,
                self.write_timeout,
                "send",
            )
            .await?;
            self.last_tx_at = Instant::now();
        }
        let mut sent = 0;
        while self.outbound_data.front().is_some() && sent < PACING_BURST_CAP {
            let now = Instant::now();
            if let Some(deadline) = self.next_data_send_at
                && now < deadline
            {
                // Not yet due — leave it (and everything behind it, to
                // preserve send order) queued for a later pass.
                break;
            }
            let Some(bytes) = self.outbound_data.pop_front() else {
                break;
            };
            let period = self.livecc.on_ack_received();
            // Catch-up: a wake that arrives late (the timer has ~1 ms
            // granularity, `MIN_WAKE`) still owes the packets whose slots
            // passed meanwhile, so the schedule advances from the previous
            // slot even if it is already past — but never by more than
            // `PACING_BURST_CAP` periods of credit, so a stall cannot be
            // repaid as an unbounded burst.
            let floor = now
                .checked_sub(period * PACING_BURST_CAP as u32)
                .unwrap_or(now);
            let base = self.next_data_send_at.map_or(now, |t| t.max(floor));
            self.next_data_send_at = Some(base + period);
            sent += 1;
            send_bounded(
                &self.udp,
                &bytes,
                self.peer_addr,
                self.write_timeout,
                "send",
            )
            .await?;
            self.last_tx_at = Instant::now();
        }
        // An idle connection owes nothing: drop an expired slot so the next
        // backlog starts at "now" instead of replaying the idle gap as credit.
        if self.outbound_data.is_empty() {
            self.next_data_send_at = self.next_data_send_at.filter(|&t| t > Instant::now());
        }
        Ok(())
    }
}

/// Split ascending, circularly ordered sequence numbers into inclusive
/// `(first, last)` runs of consecutive values.
fn contiguous_runs(seqs: &[u32]) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &s in seqs {
        match runs.last_mut() {
            Some((_, last)) if seq_next(*last) == s => *last = s,
            _ => runs.push((s, s)),
        }
    }
    runs
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
    max_datagram: usize,
) -> (mpsc::Receiver<Bytes>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
    let handle = tokio::spawn(async move {
        let mut scratch = vec![0u8; max_datagram + 1];
        loop {
            match recv_datagram(&udp, &mut scratch, max_datagram, &stats.rx_oversize).await {
                Ok(Some((bytes, src))) if src == peer => {
                    if tx.try_send(bytes).is_err() {
                        if tx.is_closed() {
                            break; // Driver is gone.
                        }
                        // Full — the driver isn't draining fast enough; drop
                        // this datagram rather than buffer without bound
                        // (item 4) and count it.
                        stats.rx_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(_) => {} // datagram from another peer, or oversize (counted); ignore.
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
///
/// A repeated CONCLUSION from an already-accepted Caller (its copy of our
/// response was lost) carries Destination Socket ID `0` too, so it reaches
/// the listener's background task, which answers it again — no one has to be
/// polling [`accept`](Self::accept) for that.
///
/// # Lifetime (defect 3)
///
/// Handshakes advance in a tracked background task, not inside
/// [`accept`](Self::accept); the routing pump is a second tracked task. Both end
/// on a [`CancellationToken`], fired when the last of {this handle, every
/// accepted [`SrtSocket`]} is dropped — so an idle listener releases its UDP
/// port the moment the handle is dropped, while connections it already
/// accepted keep being routed until they too are gone.
pub struct SrtListener {
    /// Held only so its `Drop` (cancel the tasks) runs with the handle.
    _life: Arc<ListenerLife>,
    local: std::net::SocketAddr,
    accepted: mpsc::Receiver<Result<SrtSocket>>,
    /// Shared demux table (see [`RouteTable`]); the core and the routing pump
    /// hold the same `Arc`. Read by tests only.
    #[cfg_attr(not(test), allow(dead_code))]
    routes: RouteTable,
    unrouted_dropped: Arc<AtomicU64>,
    overflow_dropped: Arc<AtomicU64>,
    send_errors: Arc<AtomicU64>,
}

/// Capacity of the pending-`accept` queue: handshakes that finished while
/// `accept` was not being polled.
const ACCEPT_BACKLOG: usize = 64;

/// Owns the listener's background tasks. Dropped when the last of the
/// [`SrtListener`] handle and every accepted [`SrtSocket`] is gone; that
/// cancels the tasks, which closes the UDP socket.
#[derive(Debug)]
struct ListenerLife {
    cancel: CancellationToken,
    tasks: TaskTracker,
}

impl Drop for ListenerLife {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.tasks.close();
    }
}

/// The listener's handshake state, driven by one tracked background task
/// ([`ListenerCore::run`]).
struct ListenerCore {
    udp: Arc<UdpSocket>,
    config: HandshakeConfig,
    io: IoConfig,
    /// Per-listener 128-bit key for [`derive_cookie`] (`draft-sharabayko-srt-01`
    /// §4.3.1.1: "a cookie that is crafted based on host, port and current
    /// time"), drawn from the OS at [`SrtListener::bind`] and rotated every
    /// [`COOKIE_KEY_ROTATION`], so a cookie is neither shared across listeners
    /// nor valid for the life of the process.
    cookie_keys: CookieKeys,
    pending: std::collections::HashMap<std::net::SocketAddr, PendingListener>,
    outbound_queue: std::collections::HashMap<std::net::SocketAddr, VecDeque<Vec<u8>>>,
    /// CONCLUSION responses of recently accepted connections, by source
    /// address, so a repeated CONCLUSION can be answered again (see
    /// [`RecentAccept`]).
    recent_accepts: std::collections::HashMap<std::net::SocketAddr, RecentAccept>,
    /// Shared demux table the routing pump task consults for every inbound
    /// datagram; `drain_completed` registers a new entry for each freshly
    /// accepted connection.
    routes: RouteTable,
    /// Datagrams the routing pump could not match to any entry in `routes`
    /// — candidate new-connection (handshake) traffic. Bounded (item 4): see
    /// [`UNROUTED_CHANNEL_CAPACITY`].
    unrouted_rx: mpsc::Receiver<(std::net::SocketAddr, Bytes)>,
    /// Weak, so the core never keeps the listener alive: a connection is only
    /// handed over while a handle (or another connection) still holds the
    /// `Arc`.
    life: std::sync::Weak<ListenerLife>,
    /// Finished handshakes, for [`SrtListener::accept`].
    accepted_tx: mpsc::Sender<Result<SrtSocket>>,
    /// Connections dropped because `accepted_tx` was full.
    overflow_dropped: Arc<AtomicU64>,
    /// Send failures met while answering peers (counted, never queued: they
    /// must not compete with finished connections for `accepted_tx` slots).
    send_errors: Arc<AtomicU64>,
    /// `false` once the handle is gone: no new handshakes are served, but the
    /// task keeps draining `unrouted_rx` so the routing pump (which accepted
    /// connections still depend on) is never starved into exiting.
    accepting: bool,
    /// When `tick_pending` last ran: the basis of [`ListenerCore::poll_timeout`].
    last_pending_tick: Instant,
}

/// The listener's SYN-cookie key and when it was drawn. Each pending
/// handshake keeps the cookie it was *issued* (the engine compares the
/// CONCLUSION's cookie with it), so a handshake that began under the previous
/// key still completes after a rotation — the "previous key stays acceptable"
/// window is exactly one handshake's lifetime; only cookies issued from then
/// on use the new key.
struct CookieKeys {
    current: u128,
    drawn_at: Instant,
}

impl CookieKeys {
    fn new(now: Instant) -> Result<Self> {
        Ok(CookieKeys {
            current: random_u128()?,
            drawn_at: now,
        })
    }

    fn current(&self) -> u128 {
        self.current
    }

    /// Replace the key once it is [`COOKIE_KEY_ROTATION`] old. Returns whether
    /// it did. A failing OS random source keeps the old key (and is retried on
    /// the next call) rather than ending the listener.
    fn rotate_if_due(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.drawn_at) < COOKIE_KEY_ROTATION {
            return false;
        }
        match random_u128() {
            Ok(key) => {
                self.current = key;
                self.drawn_at = now;
                true
            }
            Err(_) => false,
        }
    }
}

impl core::fmt::Debug for CookieKeys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CookieKeys")
            .field("current", &format_args!("<redacted>"))
            .field("drawn_at", &self.drawn_at)
            .finish()
    }
}

impl core::fmt::Debug for ListenerCore {
    // Manual: the cookie key must never reach a log via `{:?}` (#1142).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListenerCore")
            .field("local_addr", &self.udp.local_addr().ok())
            .field("cookie_keys", &self.cookie_keys)
            .field("pending", &self.pending.len())
            .field("recent_accepts", &self.recent_accepts.len())
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for SrtListener {
    // Manual: only the address — the cookie key lives in `ListenerCore`,
    // which is never printed from here (#1142).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SrtListener")
            .field("local_addr", &self.local)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct PendingListener {
    handshake: ListenerHandshake,
    params: Option<HandshakeOutput>,
    /// The peer's ISN, extracted from the INDUCTION handshake packet.
    peer_initial_seq: u32,
    /// The `Timestamp` of the peer's CONCLUSION, which seeds the TSBPD time
    /// base (§4.5.1.1: `TsbpdTimeBase = T_NOW - HSREQ_TIMESTAMP`).
    peer_timestamp_us: u32,
    /// When that CONCLUSION arrived: `T_NOW` of the same formula, hence the
    /// connection epoch (not the later moment `accept` happens to hand the
    /// connection over).
    concluded_at: Option<Instant>,
    /// The last handshake datagram this entry produced — on completion, the
    /// CONCLUSION response a lost-response retransmit must be answered with.
    last_response: Option<Vec<u8>>,
    /// This listener's own ISN for this connection, generated once when the
    /// entry was created and echoed back here (rather than re-read from
    /// `self.config` at completion) so it is guaranteed to be the exact
    /// value the [`ListenerHandshake`] was constructed with — see
    /// [`SrtListener::handle_datagram`].
    our_initial_seq: u32,
}

/// The CONCLUSION response of an accepted connection, kept for
/// [`RECENT_ACCEPT_TTL`] so that when the Caller repeats its CONCLUSION (our
/// response was lost, so it never saw the handshake finish) the identical
/// response is sent again — libsrt re-answers a repeated CONCLUSION the same
/// way. Without it the repeat reaches a listener that has already forgotten
/// the handshake, nothing is sent, and the Caller times out against a
/// connection the application is already using.
#[derive(Debug)]
struct RecentAccept {
    /// The Caller's SRT Socket ID — a repeat must carry the same one.
    peer_socket_id: u32,
    response: Vec<u8>,
    accepted_at: Instant,
}

impl SrtListener {
    /// Bind an SRT listener on `addr`.
    ///
    /// # Encryption
    /// A [`HandshakeConfig`] with `crypto` set is refused with
    /// [`Error::EncryptionUnsupported`] before any network I/O, and a peer
    /// whose CONCLUSION requests encryption (a Key Material extension or a
    /// non-zero `Encryption Field`) is rejected — this adapter's data path
    /// does not encrypt, so an encrypted connection must never be accepted.
    pub async fn bind<A: tokio::net::ToSocketAddrs>(
        addr: A,
        config: HandshakeConfig,
    ) -> Result<Self> {
        Self::bind_with(addr, config, IoConfig::default()).await
    }

    /// Like [`Self::bind`], with explicit timeouts and datagram size.
    pub async fn bind_with<A: tokio::net::ToSocketAddrs>(
        addr: A,
        config: HandshakeConfig,
        io: IoConfig,
    ) -> Result<Self> {
        refuse_crypto(&config)?;
        let socket = Arc::new(UdpSocket::bind(addr).await.map_err(|e| io_err("bind", e))?);
        let local = socket.local_addr().map_err(|e| io_err("local_addr", e))?;
        let routes: RouteTable = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let (unrouted_tx, unrouted_rx) = mpsc::channel(UNROUTED_CHANNEL_CAPACITY);
        let (accepted_tx, accepted) = mpsc::channel(ACCEPT_BACKLOG);
        let unrouted_dropped = Arc::new(AtomicU64::new(0));
        let overflow_dropped = Arc::new(AtomicU64::new(0));
        let send_errors = Arc::new(AtomicU64::new(0));
        let life = Arc::new(ListenerLife {
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
        });

        let core = ListenerCore {
            udp: Arc::clone(&socket),
            config,
            io,
            cookie_keys: CookieKeys::new(Instant::now())?,
            pending: std::collections::HashMap::new(),
            outbound_queue: std::collections::HashMap::new(),
            recent_accepts: std::collections::HashMap::new(),
            routes: Arc::clone(&routes),
            unrouted_rx,
            life: Arc::downgrade(&life),
            accepted_tx,
            overflow_dropped: Arc::clone(&overflow_dropped),
            send_errors: Arc::clone(&send_errors),
            accepting: true,
            last_pending_tick: Instant::now(),
        };
        life.tasks.spawn(routing_pump(
            Arc::clone(&socket),
            Arc::clone(&routes),
            unrouted_tx,
            Arc::clone(&unrouted_dropped),
            io.max_datagram.clamp(MIN_MAX_DATAGRAM, MAX_MAX_DATAGRAM),
            life.cancel.clone(),
        ));
        life.tasks.spawn(core.run(life.cancel.clone()));
        Ok(SrtListener {
            _life: life,
            local,
            accepted,
            routes,
            unrouted_dropped,
            overflow_dropped,
            send_errors,
        })
    }

    /// Datagrams dropped because the candidate-new-connection queue was full
    /// (item 4) — a handshake flood outrunning the listener's handshake task —
    /// or because they exceeded [`IoConfig::max_datagram`].
    pub fn unrouted_dropped(&self) -> u64 {
        self.unrouted_dropped.load(Ordering::Relaxed)
    }

    /// Connections that finished their handshake but were dropped because
    /// [`accept`](Self::accept) was not keeping up (the pending-accept queue
    /// holds 64).
    pub fn accept_overflow_dropped(&self) -> u64 {
        self.overflow_dropped.load(Ordering::Relaxed)
    }

    /// Send failures the background task met while answering peers (for
    /// example an unreachable spoofed source). They are counted here and never
    /// surface from [`accept`](Self::accept), so a flood of them cannot displace
    /// finished connections or end an `accept` loop.
    pub fn send_errors(&self) -> u64 {
        self.send_errors.load(Ordering::Relaxed)
    }

    /// The local socket address the listener is bound to.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        Ok(self.local)
    }

    /// Accept the next incoming SRT connection.
    ///
    /// Handshakes progress in the background whether or not this is being
    /// polled; this only waits for a finished one. Send errors met while
    /// answering peers are counted ([`send_errors`](Self::send_errors)), not
    /// returned.
    pub async fn accept(&mut self) -> Result<SrtSocket> {
        self.accepted.recv().await.unwrap_or(Err(Error::Io {
            kind: std::io::ErrorKind::BrokenPipe,
            context: "listener task ended",
        }))
    }
}

impl ListenerCore {
    /// Next instant a pending handshake needs a tick; `None` when nothing is
    /// pending (an idle listener has no timer at all).
    fn poll_timeout(&self) -> Option<Instant> {
        (self.accepting && !self.pending.is_empty())
            .then(|| self.last_pending_tick + PENDING_TICK_INTERVAL)
    }

    /// The handshake task: answer unrouted datagrams, tick pending handshakes
    /// on a deadline, hand finished connections to [`SrtListener::accept`].
    async fn run(mut self, cancel: CancellationToken) {
        loop {
            while self.accepting
                && let Some(conn) = self.drain_completed()
            {
                match self.accepted_tx.try_send(conn) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_dropped)) => {
                        // Dropping the SrtSocket sends the peer a SHUTDOWN; count it.
                        self.overflow_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        // The handle is gone but accepted connections may
                        // still be alive: stop serving new handshakes, keep
                        // draining so the routing pump keeps running.
                        self.accepting = false;
                        self.pending.clear();
                        self.outbound_queue.clear();
                    }
                }
            }
            let deadline = self.poll_timeout();
            tokio::select! {
                _ = cancel.cancelled() => return,
                r = self.unrouted_rx.recv() => match r {
                    Some((src, bytes)) => {
                        if !self.accepting {
                            continue;
                        }
                        // Cookie rotation no longer rides a tick: do it whenever a
                        // datagram arrives.
                        self.cookie_keys.rotate_if_due(Instant::now());
                        let _ = self.handle_datagram(src, &bytes);
                        // Raced against cancellation: a stuck send must not
                        // delay releasing the port by up to `io.write`.
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            r = self.flush_for_peer(src) => self.count_send_error(r),
                        }
                    }
                    None => return,
                },
                _ = async {
                    match deadline {
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    self.last_pending_tick = Instant::now();
                    self.tick_pending();
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        r = self.flush_all() => self.count_send_error(r),
                    }
                }
            }
        }
    }

    /// Count a send failure (one unreachable peer must neither end the
    /// listener nor reach `accept`).
    fn count_send_error(&self, r: Result<()>) {
        if r.is_err() {
            self.send_errors.fetch_add(1, Ordering::Relaxed);
        }
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

    fn handle_datagram(&mut self, src: std::net::SocketAddr, bytes: &[u8]) -> Result<()> {
        let packet = SrtPacket::parse(bytes).map_err(|e| handshake_err("parse", e))?;

        let ctrl = match packet {
            SrtPacket::Control(c) => c,
            _ => return Ok(()),
        };

        // A repeated CONCLUSION from a Caller we already accepted means our
        // response never reached it: send the very same response again. A
        // fresh INDUCTION from the same address starts over instead.
        if let ControlPacket::Handshake(hp) = &ctrl {
            match hp.handshake_type {
                HandshakeType::Conclusion => {
                    if let Some(recent) = self.recent_accepts.get(&src)
                        && recent.peer_socket_id == hp.srt_socket_id
                        && recent.accepted_at.elapsed() < RECENT_ACCEPT_TTL
                    {
                        let response = recent.response.clone();
                        self.outbound_queue
                            .entry(src)
                            .or_default()
                            .push_back(response);
                        return Ok(());
                    }
                }
                HandshakeType::Induction => {
                    self.recent_accepts.remove(&src);
                }
                _ => {}
            }
        }

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
            let asks_kmreq =
                hp.handshake_type == HandshakeType::Conclusion && hp.extension_field.kmreq();
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
            let own_socket_id = random_socket_id()?;
            let our_initial_seq = random_isn()?;
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
            let syn_cookie = derive_cookie(peer_key, time_bucket, self.cookie_keys.current());
            let hs = ListenerHandshake::new(own_socket_id, syn_cookie, per_conn_config);
            self.pending.insert(
                src,
                PendingListener {
                    handshake: hs,
                    params: None,
                    peer_initial_seq: peer_isn,
                    peer_timestamp_us: 0,
                    concluded_at: None,
                    last_response: None,
                    our_initial_seq,
                },
            );
        }

        let entry = self.pending.get_mut(&src).ok_or(Error::InvalidField {
            what: "pending",
            reason: "no pending entry",
        })?;

        if let ControlPacket::Handshake(hp) = &ctrl
            && hp.handshake_type == HandshakeType::Conclusion
        {
            // The first CONCLUSION fixes the pair (a repeat must not move it).
            if entry.concluded_at.is_none() {
                entry.peer_timestamp_us = hp.timestamp;
                entry.concluded_at = Some(Instant::now());
            }
        }

        let outcomes = entry
            .handshake
            .feed(&ctrl)
            .map_err(|e| handshake_err("listener feed", e))?;

        for outcome in outcomes {
            match outcome {
                HandshakeOutput::Send(bytes) => {
                    entry.last_response = Some(bytes.clone());
                    self.outbound_queue.entry(src).or_default().push_back(bytes);
                }
                HandshakeOutput::Connected(negotiated) => {
                    entry.params = Some(HandshakeOutput::Connected(negotiated));
                }
                HandshakeOutput::Rejected(reason) => {
                    self.pending.remove(&src);
                    return Err(Error::Rejected(reason));
                }
                HandshakeOutput::TimedOut => {
                    self.pending.remove(&src);
                    return Err(Error::HandshakeTimedOut { stage: "listener" });
                }
            }
        }

        Ok(())
    }

    fn tick_pending(&mut self) {
        self.cookie_keys.rotate_if_due(Instant::now());
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
        self.recent_accepts
            .retain(|_, r| r.accepted_at.elapsed() < RECENT_ACCEPT_TTL);
    }

    /// Remember an accepted connection's CONCLUSION response (bounded: expired
    /// entries are reaped first, then the oldest is evicted if still full).
    fn remember_accept(&mut self, addr: std::net::SocketAddr, recent: RecentAccept) {
        self.recent_accepts
            .retain(|_, r| r.accepted_at.elapsed() < RECENT_ACCEPT_TTL);
        if self.recent_accepts.len() >= MAX_RECENT_ACCEPTS
            && let Some(oldest) = self
                .recent_accepts
                .iter()
                .min_by_key(|(_, r)| r.accepted_at)
                .map(|(a, _)| *a)
        {
            self.recent_accepts.remove(&oldest);
        }
        self.recent_accepts.insert(addr, recent);
    }

    fn drain_completed(&mut self) -> Option<Result<SrtSocket>> {
        // Every handle is gone: nothing may be handed over (the cancel token
        // ends this task right after).
        let life = self.life.upgrade()?;
        let addr = self
            .pending
            .iter()
            .find(|(_, p)| {
                p.params.is_some()
                    && matches!(p.handshake.state(), ListenerHandshakeState::Connected)
            })
            .map(|(addr, _)| *addr)?;

        let entry = self.pending.remove(&addr)?;
        // `Connected` entries always carry negotiated parameters, but a
        // missing one must not panic the accept loop.
        let Some(negotiated) = entry.handshake.negotiated() else {
            return Some(Err(Error::InvalidField {
                what: "listener handshake",
                reason: "Connected without negotiated parameters",
            }));
        };
        let peer_initial_seq = entry.peer_initial_seq;
        // The peer's negotiated SRT Socket ID (distinct from its ISN above).
        let peer_socket_id = negotiated.peer_socket_id;
        // This listener's own Socket ID for this connection — the value a
        // real peer's packets carry as `dest_socket_id` from here on, and so
        // the key the routing pump demultiplexes by (item 2; see
        // [`routing_pump`]).
        let own_socket_id = negotiated.own_socket_id;
        // The exact ISN this listener generated for `entry` at INDUCTION
        // time (`handle_datagram`) — not `self.config.initial_seq_number`,
        // which is the listener-wide template and no longer carries the
        // per-connection value once randomized.
        let our_initial_seq = entry.our_initial_seq;
        let tsbpd_delay_ms = u64::from(negotiated.latency_ms);
        let tsbpd_time_base = tsbpd_time_base_from(entry.peer_timestamp_us);
        let epoch = entry.concluded_at.unwrap_or_else(Instant::now);

        if let Some(response) = entry.last_response.clone() {
            self.remember_accept(
                addr,
                RecentAccept {
                    peer_socket_id,
                    response,
                    accepted_at: epoch,
                },
            );
        }

        // Register this connection's own route in the shared demux table
        // (issue #1029) *before* handing off to its driver task — the
        // routing pump must never see a datagram naming this connection's
        // Socket ID land in `unrouted_rx` again once this connection exists.
        let (tx, rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
        let stats = Arc::new(Counters::default());
        lock(&self.routes).insert(
            own_socket_id,
            RouteEntry {
                addr,
                tx,
                stats: Arc::clone(&stats),
            },
        );

        // Share the listener's Arc<UdpSocket> with the connection's driver.
        let conn = SrtSocket::spawn(ConnParams {
            udp: Arc::clone(&self.udp),
            peer_addr: addr,
            rx,
            stats,
            route_cleanup: RouteCleanup(Some((own_socket_id, Arc::clone(&self.routes)))),
            forwarder_guard: TaskGuard(None),
            life: Some(life),
            read_idle: self.io.read_idle,
            write: self.io.write,
            our_initial_seq,
            peer_initial_seq,
            peer_socket_id,
            tsbpd_time_base,
            tsbpd_delay_ms,
            epoch,
            max_flow_window: negotiated.max_flow_window_size,
            mtu: negotiated.mtu,
            tlpkt_drop: negotiated.flags.tlpktdrop(),
        });
        Some(Ok(conn))
    }

    async fn flush_for_peer(&mut self, addr: std::net::SocketAddr) -> Result<()> {
        if let Some(queue) = self.outbound_queue.get_mut(&addr) {
            while let Some(bytes) = queue.pop_front() {
                send_bounded(&self.udp, &bytes, addr, self.io.write, "send_to").await?;
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

/// The single task that ever calls `recv_from` on a [`SrtListener`]'s
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
async fn routing_pump(
    udp: Arc<UdpSocket>,
    routes: RouteTable,
    unrouted_tx: mpsc::Sender<(std::net::SocketAddr, Bytes)>,
    unrouted_dropped: Arc<AtomicU64>,
    max_datagram: usize,
    cancel: CancellationToken,
) {
    let mut scratch = vec![0u8; max_datagram + 1];
    loop {
        let received = tokio::select! {
            _ = cancel.cancelled() => break,
            r = recv_datagram(&udp, &mut scratch, max_datagram, &unrouted_dropped) => r,
        };
        let (bytes, src) = match received {
            Ok(Some(v)) => v,
            Ok(None) => continue, // oversize: dropped and counted.
            Err(_) => break,      // socket error — end the pump.
        };
        match crate::packet::peek_dest_socket_id(&bytes) {
            Some(0) | None => {
                // `0` (candidate new connection) or too short to carry a
                // full header at all — either way, not a routable
                // established-connection packet; hand it to the handshake
                // task, which parses (and rejects malformed bytes) properly.
                if unrouted_tx.try_send((src, bytes)).is_err() {
                    if unrouted_tx.is_closed() {
                        // The handshake task is gone; nothing more this pump can do.
                        break;
                    }
                    unrouted_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Some(id) => {
                let route = lock(&routes).get(&id).cloned();
                // No `route` entry (no connection was ever accepted with
                // this ID), or one exists but from a different source
                // address, is never routed — spoofed-source packets
                // included.
                if let Some(entry) = route
                    && entry.addr == src
                    && entry.tx.try_send(bytes).is_err()
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
        return Err(Error::EncryptionUnsupported);
    }
    #[cfg(not(feature = "crypto"))]
    let _ = config;
    Ok(())
}

/// Runs `fut` for at most `limit`; expiry is `Error::Io { kind: TimedOut, context }`.
async fn bounded<T>(
    limit: Duration,
    context: &'static str,
    fut: impl core::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| Error::Io {
            kind: std::io::ErrorKind::TimedOut,
            context,
        })?
}

/// `send_to` bounded by `limit` (the [`IoConfig::write`] timeout).
async fn send_bounded(
    udp: &UdpSocket,
    bytes: &[u8],
    to: std::net::SocketAddr,
    limit: Duration,
    context: &'static str,
) -> Result<()> {
    bounded(limit, context, async {
        udp.send_to(bytes, to)
            .await
            .map(|_| ())
            .map_err(|e| io_err(context, e))
    })
    .await
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

/// Wraps an engine/codec failure with the handshake stage it happened in,
/// keeping the underlying error as the source (r08-SRT-W12) rather than
/// flattening it to a generic invalid-field error.
fn handshake_err(stage: &'static str, source: Error) -> Error {
    Error::Handshake {
        stage,
        source: alloc::boxed::Box::new(source),
    }
}

/// Mixes a [`std::net::SocketAddr`] into a `u64` for use as `derive_cookie`'s
/// `peer_key` input (§4.3.1.1: the cookie is "crafted based on host,
/// port..."). Not a spec-defined algorithm — any stable, well-distributed
/// mix of the peer's address is sufficient here: the secrecy of the cookie
/// rests on the keyed hash in [`derive_cookie`], not on this.
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
    u32::try_from(secs / 60).unwrap_or(u32::MAX)
}

/// A random `u64` from the OS randomness source (`getrandom`), used as
/// `derive_cookie`'s `secret` input and to draw the per-connection Socket ID
/// and ISN (SRT-W11). A failing OS source is reported, never panicked on.
fn random_u64() -> Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|_| Error::Io {
        kind: std::io::ErrorKind::Other,
        context: "getrandom",
    })?;
    Ok(u64::from_ne_bytes(bytes))
}

/// A random `u128` (two OS draws).
fn random_u128() -> Result<u128> {
    Ok((u128::from(random_u64()?) << 64) | u128::from(random_u64()?))
}

/// The low 32 bits of a fresh random `u64`.
fn random_u32() -> Result<u32> {
    Ok(u32::try_from(random_u64()? & u64::from(u32::MAX)).unwrap_or(0))
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
fn random_socket_id() -> Result<u32> {
    Ok(random_u32()? % MAX_SRT_SOCKET_ID + 1)
}

/// A random Initial Sequence Number in the legal 31-bit SRT sequence-number
/// range (`SEQ_NUMBER_MASK`, §3) for a new connection. Every new Caller
/// connect and every new accepted Listener connection must draw a fresh
/// value here rather than reuse a fixed/default `HandshakeConfig::initial_seq_number`
/// (SRT-W11): `draft-sharabayko-srt-01` §3/§4.3.1.1 expects the ISN
/// unpredictable per connection, same as the Socket ID above.
fn random_isn() -> Result<u32> {
    Ok(random_u32()? & SEQ_NUMBER_MASK)
}

async fn resolve_one<A: tokio::net::ToSocketAddrs>(addr: A) -> Result<std::net::SocketAddr> {
    let mut addrs = tokio::net::lookup_host(addr)
        .await
        .map_err(|e| io_err("resolve", e))?;
    addrs.next().ok_or(Error::Io {
        kind: std::io::ErrorKind::AddrNotAvailable,
        context: "resolve: no addresses",
    })
}

/// The fields of the peer's final handshake packet the adapter seeds its
/// engines from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeerHandshake {
    /// The peer's Initial Packet Sequence Number (§3.2.1).
    initial_seq_number: u32,
    /// The packet's `Timestamp` (§3), microseconds since the peer's
    /// connection start — `HSREQ_TIMESTAMP` of `TsbpdTimeBase = T_NOW -
    /// HSREQ_TIMESTAMP` (§4.5.1.1).
    timestamp_us: u32,
}

/// Extract the peer's `initial_seq_number` and timestamp from a handshake
/// control packet's bytes. These seed ARQ and TSBPD sequence/time tracking for
/// the entire connection, so "the bytes didn't parse as a Handshake" must be a
/// distinct, caller-visible outcome from "the peer's ISN is genuinely 0" — the
/// two are otherwise indistinguishable to a caller that only sees a `u32`.
/// Returns [`Error::InvalidField`] rather than defaulting to `0` on any parse
/// failure.
fn peer_handshake(bytes: &[u8]) -> Result<PeerHandshake> {
    match SrtPacket::parse(bytes) {
        Ok(SrtPacket::Control(ControlPacket::Handshake(hp))) => Ok(PeerHandshake {
            initial_seq_number: hp.initial_seq_number,
            timestamp_us: hp.timestamp,
        }),
        _ => Err(Error::InvalidField {
            what: "peer handshake",
            reason: "handshake reached Connected but its final packet did not re-parse as a \
                     Handshake control packet; refusing to seed ARQ/TSBPD with a fabricated ISN",
        }),
    }
}

/// `TsbpdTimeBase` seed (§4.5.1.1, rule 12): `T_NOW - HSREQ_TIMESTAMP` with
/// `T_NOW` = 0 at the connection epoch (the instant the handshake packet
/// arrived), i.e. minus the peer's handshake timestamp. Signed: the
/// timestamp, counted from the peer's own start, is generally positive, so
/// the base is generally negative.
fn tsbpd_time_base_from(peer_timestamp_us: u32) -> i64 {
    -i64::from(peer_timestamp_us)
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
            let id = random_socket_id().expect("OS randomness");
            assert!(id != 0, "socket id must never be 0");
            assert!(
                id <= MAX_SRT_SOCKET_ID,
                "socket id {id:#010x} exceeds the libsrt allocation range {MAX_SRT_SOCKET_ID:#010x}"
            );
        }
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
    fn peer_handshake_extracts_nonzero_isn() {
        let bytes = handshake_bytes(0xABCD_1234);
        assert_eq!(
            peer_handshake(&bytes).unwrap().initial_seq_number,
            0xABCD_1234
        );
    }

    #[test]
    fn peer_handshake_distinguishes_genuine_zero_from_parse_failure() {
        // A genuine ISN of 0 is a valid, well-formed handshake and must
        // succeed as Ok(0) — not be conflated with "couldn't parse".
        let bytes = handshake_bytes(0);
        assert_eq!(peer_handshake(&bytes).unwrap().initial_seq_number, 0);

        // Bytes that don't parse as a Handshake control packet at all
        // (too short to even carry the fixed SRT header) must error, never
        // silently produce a plausible-looking 0.
        let err = peer_handshake(&[0u8; 4]).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidField {
                what: "peer handshake",
                ..
            }
        ));
    }

    #[test]
    fn peer_handshake_rejects_non_handshake_control_packet() {
        // A well-formed control packet of the WRONG type (Keep-Alive, not
        // Handshake) must also error rather than default to 0.
        let ka = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: 0,
            dest_socket_id: 7,
            libsrt_pad: false,
        });
        let mut buf = alloc::vec![0u8; ka.serialized_len()];
        ka.serialize_into(&mut buf).expect("serialize keepalive");
        let err = peer_handshake(&buf).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidField {
                what: "peer handshake",
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
        AckCif, AckPacket, DataPacket, DropReqPacket, EncryptionKeyField, HandshakeExtensionFlags,
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
    async fn test_driver(max_flow_window: u32) -> (Driver, mpsc::Receiver<Bytes>) {
        let (driver, rx, _datagram_tx) = test_driver_with_ingress(max_flow_window).await;
        // The inbound-datagram sender is dropped here: the tests that use this
        // drive `ingress`/`tick_engines` directly, never `Driver::run`'s
        // select loop (which would see the closed channel as "upstream gone").
        (driver, rx)
    }

    /// Like [`test_driver`], but also returns the live sender of the
    /// driver's inbound-datagram channel, so `Driver::run` can be spawned.
    async fn test_driver_with_ingress(
        max_flow_window: u32,
    ) -> (Driver, mpsc::Receiver<Bytes>, mpsc::Sender<Bytes>) {
        let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        // Make the socket's writability known up front. Without it the first
        // `send_to` yields to the reactor, and under a paused clock the
        // runtime would then auto-advance time to the (new) write-timeout
        // timer before the real send completes.
        udp.writable().await.unwrap();
        let (deliver, rx) = mpsc::channel(DELIVER_CHANNEL_CAPACITY);
        let (datagram_tx, datagram_rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
        let driver = Driver::new(
            ConnParams {
                udp,
                peer_addr: peer_addr(),
                rx: datagram_rx,
                stats: Arc::new(Counters::default()),
                route_cleanup: RouteCleanup(None),
                forwarder_guard: TaskGuard(None),
                life: None,
                read_idle: IoConfig::default().read_idle,
                write: IoConfig::default().write,
                our_initial_seq: 0,
                peer_initial_seq: 0,
                peer_socket_id: 1,
                tsbpd_time_base: 0,
                tsbpd_delay_ms: LATENCY_MS,
                epoch: Instant::now(),
                max_flow_window,
                mtu: 1500,
                tlpkt_drop: true,
            },
            deliver,
        );
        (driver, rx, datagram_tx)
    }

    fn data_bytes(seq: u32, timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let dp = DataPacket {
            seq_number: seq,
            position: PacketPosition::Solo,
            in_order: true,
            key_flag: EncryptionKeyField::NotEncrypted,
            retransmitted: false,
            message_number: seq & crate::packet::data::MESSAGE_NUMBER_MASK,
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

    /// Review-focus 2. UDP truncates a datagram larger than the receive buffer
    /// without telling the reader; a truncated DATA packet then parses as a
    /// shorter, valid packet and its payload is delivered corrupted. The
    /// forwarder must read `max + 1` bytes, drop anything longer than `max`,
    /// and count it.
    #[tokio::test]
    async fn an_oversize_datagram_is_dropped_and_counted_never_truncated() {
        let local = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stats = Arc::new(Counters::default());
        let (mut rx, _guard) = spawn_dedicated_socket_forwarder(
            Arc::clone(&local),
            peer.local_addr().unwrap(),
            Arc::clone(&stats),
            300,
        );
        let local_addr = local.local_addr().unwrap();
        peer.send_to(&[1u8; 250], local_addr).await.unwrap();
        peer.send_to(&[2u8; 400], local_addr).await.unwrap(); // > 300
        peer.send_to(&[3u8; 300], local_addr).await.unwrap(); // exactly max: accepted
        let a = rx.recv().await.unwrap();
        let b = rx.recv().await.unwrap();
        assert_eq!((a.len(), a[0]), (250, 1));
        assert_eq!(
            (b.len(), b[0]),
            (300, 3),
            "the 400-byte datagram must not appear, truncated or not"
        );
        assert_eq!(stats.rx_oversize.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn io_config_clamps_a_tiny_max_datagram() {
        assert_eq!(
            IoConfig::default().with_max_datagram(1).max_datagram,
            MIN_MAX_DATAGRAM
        );
        assert_eq!(IoConfig::default().max_datagram, 1500);
        assert_eq!(
            IoConfig::default()
                .with_max_datagram(usize::MAX)
                .max_datagram,
            MAX_MAX_DATAGRAM,
            "an absurd size is clamped, never overflows `max + 1`"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_driver_deadline_is_the_ack_period_not_a_two_ms_tick() {
        let (driver, _rx) = test_driver(8192).await;
        let d = driver
            .poll_timeout()
            .expect("a connected driver always has a deadline");
        assert_eq!(d - driver.epoch, crate::arq::FULL_ACK_PERIOD);
    }

    /// The old fixed 2 ms ticker ran `tick_engines` ~50 times per 100 ms of idle time; the
    /// deadline-driven loop wakes for the 10 ms ACK cadence only.
    #[tokio::test(start_paused = true)]
    async fn the_run_loop_wakes_at_deadlines_not_on_a_fixed_two_ms_tick() {
        let (mut driver, _rx, _tx) = test_driver_with_ingress(8192).await;
        // The loop really sends its ACKs: a loopback address accepts them, where
        // the default non-routable test peer errors on some hosts and ends the loop.
        driver.peer_addr = "127.0.0.1:1".parse().unwrap();
        let ticks = Arc::new(AtomicU64::new(0));
        driver.tick_counter = Some(Arc::clone(&ticks));
        let (_app_tx, app_rx) = mpsc::channel(8);
        let handle = tokio::spawn(driver.run(app_rx));
        // `advance` jumps the clock in ONE step, so walk it 1 ms at a time to let every timer fire in order.
        for _ in 0..100 {
            tokio::time::advance(Duration::from_millis(1)).await;
            tokio::task::yield_now().await;
        }
        let n = ticks.load(Ordering::Relaxed);
        assert!(
            (8..=14).contains(&n),
            "expected ~10 wake-ups in 100 ms (10 ms ACK cadence), got {n}"
        );
        handle.abort();
    }

    /// A [`ListenerCore`] over a throwaway socket, never spawned: tests drive
    /// `handle_datagram`/`tick_pending` and read its state directly.
    async fn test_core(config: HandshakeConfig) -> ListenerCore {
        test_core_with_channels(config).await.0
    }

    /// [`test_core`] plus the live ends of its channels: the unrouted-datagram
    /// sender and the accepted-connection receiver.
    async fn test_core_with_channels(
        config: HandshakeConfig,
    ) -> (
        ListenerCore,
        mpsc::Sender<(std::net::SocketAddr, Bytes)>,
        mpsc::Receiver<Result<SrtSocket>>,
    ) {
        let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (unrouted_tx, unrouted_rx) = mpsc::channel(UNROUTED_CHANNEL_CAPACITY);
        let (accepted_tx, accepted) = mpsc::channel(ACCEPT_BACKLOG);
        let core = ListenerCore {
            udp,
            config,
            io: IoConfig::default(),
            cookie_keys: CookieKeys::new(Instant::now()).unwrap(),
            pending: std::collections::HashMap::new(),
            outbound_queue: std::collections::HashMap::new(),
            recent_accepts: std::collections::HashMap::new(),
            routes: Arc::new(Mutex::new(std::collections::HashMap::new())),
            unrouted_rx,
            life: std::sync::Weak::new(),
            accepted_tx,
            overflow_dropped: Arc::new(AtomicU64::new(0)),
            send_errors: Arc::new(AtomicU64::new(0)),
            accepting: true,
            last_pending_tick: Instant::now(),
        };
        (core, unrouted_tx, accepted)
    }

    #[tokio::test(start_paused = true)]
    async fn bounded_turns_a_stuck_future_into_a_timed_out_io_error() {
        let r: Result<()> = bounded(
            Duration::from_secs(3),
            "send",
            std::future::pending::<Result<()>>(),
        )
        .await;
        assert!(
            matches!(
                r,
                Err(Error::Io {
                    kind: std::io::ErrorKind::TimedOut,
                    context: "send"
                })
            ),
            "{r:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_configured_read_idle_replaces_the_five_second_constant() {
        let (mut driver, _rx) = test_driver(8192).await;
        driver.read_idle = Duration::from_secs(2);
        assert!(!driver.peer_idle(Instant::now()));
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(
            driver.peer_idle(Instant::now()),
            "idle after the configured 2 s, not 5 s"
        );
    }

    /// Real time, real socket: a paused clock would auto-advance past the
    /// timers while the first `send_to` waits on the reactor.
    #[tokio::test]
    async fn the_handshake_budget_comes_from_the_config() {
        // black hole: bound UDP socket that never answers
        let hole = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let budget = Duration::from_millis(400);
        let io = IoConfig::default().with_handshake(budget);
        let t0 = Instant::now();
        let err =
            SrtSocket::connect_with(hole.local_addr().unwrap(), HandshakeConfig::default(), io)
                .await
                .expect_err("nobody answers");
        // The default retransmit budget (12 x 250 ms) would end an unanswered connect at ~3 s with
        // stage "caller retransmit budget"; only the configured deadline yields "caller connect".
        assert!(
            matches!(
                err,
                Error::HandshakeTimedOut {
                    stage: "caller connect"
                }
            ),
            "{err:?}"
        );
        let took = Instant::now() - t0;
        assert!(
            took >= budget && took < Duration::from_secs(2),
            "the configured {budget:?} deadline, got {took:?}"
        );
        drop(hole);
    }

    // -----------------------------------------------------------------------
    // Each `Driver::poll_timeout` term, with every EARLIER term pushed out of
    // the way, so deleting any one of them changes the asserted deadline.
    // -----------------------------------------------------------------------

    async fn quiet_driver() -> Driver {
        let (mut driver, _rx) = test_driver(8192).await;
        driver.peer_addr = "127.0.0.1:1".parse().unwrap();
        driver
    }

    /// TSBPD release: a packet due at 120 ms, the Full ACK just ticked at
    /// 118 ms (next ACK 128 ms), so the release is the earliest deadline.
    #[tokio::test(start_paused = true)]
    async fn the_deadline_is_the_tsbpd_release_when_it_is_the_earliest_term() {
        let mut driver = quiet_driver().await;
        driver.ingress(Bytes::from(data_bytes(0, 0, b"p"))).unwrap();
        tokio::time::advance(Duration::from_millis(118)).await;
        driver.tick_engines();
        assert_eq!(
            driver.poll_timeout().unwrap(),
            driver.epoch + Duration::from_millis(LATENCY_MS),
            "the TSBPD release must be the next wake-up"
        );
    }

    /// Pacing: a queued DATA packet whose slot is 3 ms away beats the 10 ms ACK.
    #[tokio::test(start_paused = true)]
    async fn the_deadline_is_the_pacing_slot_when_it_is_the_earliest_term() {
        let mut driver = quiet_driver().await;
        let now = Instant::now();
        driver.outbound_data.push_back(alloc::vec![0u8; 100]);
        driver.next_data_send_at = Some(now + Duration::from_millis(3));
        assert_eq!(
            driver.poll_timeout().unwrap(),
            now + Duration::from_millis(3),
            "the pacing slot must be the next wake-up"
        );
    }

    /// Peer idle: a 4 ms read-idle limit beats the 10 ms ACK and the 1 s keep-alive.
    #[tokio::test(start_paused = true)]
    async fn the_deadline_is_the_peer_idle_expiry_when_it_is_the_earliest_term() {
        let mut driver = quiet_driver().await;
        driver.read_idle = Duration::from_millis(4);
        assert_eq!(
            driver.poll_timeout().unwrap(),
            driver.last_rx_at + Duration::from_millis(4),
            "the peer-idle expiry must be the next wake-up"
        );
    }

    /// Keep-Alive: nothing sent for 995 ms and nothing queued, so the keep-alive
    /// at +5 ms beats the 10 ms ACK.
    #[tokio::test(start_paused = true)]
    async fn the_deadline_is_the_keepalive_when_it_is_the_earliest_term() {
        let mut driver = quiet_driver().await;
        let now = Instant::now();
        driver.last_tx_at = now - Duration::from_millis(995);
        assert!(driver.outbound_control.is_empty() && driver.outbound_data.is_empty());
        assert_eq!(
            driver.poll_timeout().unwrap(),
            now + Duration::from_millis(5),
            "the keep-alive must be the next wake-up"
        );
        // ... and it is NOT a term while something is queued.
        driver.outbound_control.push_back(alloc::vec![0u8; 16]);
        assert_eq!(
            driver.poll_timeout().unwrap(),
            driver.epoch + crate::arq::FULL_ACK_PERIOD
        );
    }

    /// Wake the loop exactly as `run` would (at `poll_timeout`) for `window`
    /// of virtual time, returning how many DATA packets left the queue.
    async fn drain_for(driver: &mut Driver, window: Duration) -> usize {
        let start = Instant::now();
        let before = driver.outbound_data.len();
        while Instant::now() - start < window {
            let at = driver.poll_timeout().unwrap();
            tokio::time::advance(at - Instant::now()).await;
            driver.tick_engines();
            driver.flush_outbound().await.expect("flush");
        }
        before - driver.outbound_data.len()
    }

    /// A backlog with NO inbound events and no application traffic (retransmit
    /// burst after a NAK, idle app) drains at wake-up granularity: the old
    /// one-packet-per-wake ceiling was ~10.5 Mbit/s.
    #[tokio::test(start_paused = true)]
    async fn an_idle_backlog_drains_faster_than_one_packet_per_wake() {
        let mut driver = quiet_driver().await;
        for _ in 0..3000 {
            driver.send_one(&[0u8; 1316]);
        }
        let sent = drain_for(&mut driver, Duration::from_millis(100)).await;
        let mbit_s = sent as f64 * 1316.0 * 8.0 / 0.1 / 1e6;
        assert!(
            mbit_s >= 20.0,
            "drained {sent} packets in 100 ms = {mbit_s:.1} Mbit/s"
        );
    }

    /// Pacing still holds with catch-up: at 2.5 MB/s (20 Mbit/s) 100 ms is ~190
    /// packets plus at most one burst, never an instantaneous dump.
    #[tokio::test(start_paused = true)]
    async fn catch_up_never_exceeds_the_paced_rate_by_more_than_one_burst() {
        let mut driver = quiet_driver().await;
        for _ in 0..3000 {
            driver.send_one(&[0u8; 1316]);
        }
        driver.livecc = LiveCC::new(MaxBwConfig::Set(2_500_000));
        driver.livecc.on_data_packet(1316);
        let sent = drain_for(&mut driver, Duration::from_millis(100)).await;
        let allowed = (2_500_000.0_f64 * 0.1 / 1316.0).ceil() as usize + PACING_BURST_CAP + 1;
        assert!(
            sent <= allowed,
            "sent {sent} packets, paced allowance {allowed}"
        );
        assert!(sent >= 100, "pacing starved the drain: {sent}");
    }

    // -----------------------------------------------------------------------
    // Listener: the deadline wiring and send-error handling
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn the_listener_deadline_exists_only_while_a_handshake_is_pending() {
        let mut core = test_core(HandshakeConfig::default()).await;
        assert_eq!(core.poll_timeout(), None, "an idle listener has no timer");
        core.handle_datagram(peer_addr(), &induction_bytes(7))
            .expect("induction");
        assert_eq!(
            core.poll_timeout(),
            Some(core.last_pending_tick + PENDING_TICK_INTERVAL)
        );
        core.accepting = false;
        assert_eq!(
            core.poll_timeout(),
            None,
            "a handle-less core serves nobody"
        );
    }

    /// The run loop actually ticks a pending handshake on that deadline: with no
    /// further inbound traffic the INDUCTION response is retransmitted.
    #[tokio::test]
    async fn a_pending_handshake_is_retransmitted_by_the_listener_tick_alone() {
        let config = HandshakeConfig {
            retransmit_after_ticks: 1,
            max_retries: 5,
            ..HandshakeConfig::default()
        };
        let (core, unrouted_tx, _accepted) = test_core_with_channels(config).await;
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(core.run(cancel.clone()));
        unrouted_tx
            .send((peer.local_addr().unwrap(), Bytes::from(induction_bytes(7))))
            .await
            .unwrap();
        let mut buf = [0u8; 2048];
        for n in 1..=2 {
            tokio::time::timeout(Duration::from_secs(5), peer.recv_from(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("datagram {n} never arrived: the tick is not wired"))
                .unwrap();
        }
        cancel.cancel();
        let _ = task.await;
    }

    /// Send errors are counted, never queued: a flood of them must not touch
    /// the accept queue that finished connections depend on.
    #[tokio::test]
    async fn a_flood_of_send_errors_does_not_displace_completed_connections() {
        let (core, _tx, mut accepted) = test_core_with_channels(HandshakeConfig::default()).await;
        for _ in 0..(ACCEPT_BACKLOG * 20) {
            core.count_send_error(Err(Error::Io {
                kind: std::io::ErrorKind::HostUnreachable,
                context: "send_to",
            }));
        }
        assert_eq!(
            core.send_errors.load(Ordering::Relaxed),
            (ACCEPT_BACKLOG * 20) as u64
        );
        assert_eq!(
            core.accepted_tx.capacity(),
            ACCEPT_BACKLOG,
            "every accept-queue slot is still free for a real connection"
        );
        assert!(accepted.try_recv().is_err());
    }

    fn drain_deliveries(rx: &mut mpsc::Receiver<Bytes>) -> Vec<Bytes> {
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
            matches!(err, Error::EncryptionUnsupported),
            "expected EncryptionUnsupported, got {err:?}"
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
            matches!(err, Error::EncryptionUnsupported),
            "expected EncryptionUnsupported, got {err:?}"
        );
    }

    #[tokio::test]
    async fn encrypted_data_packet_is_never_staged_or_delivered() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let ts = 0u32;
        // The unencrypted packet at seq 0 is staged normally.
        driver
            .ingress(Bytes::from(data_bytes(0, ts, b"plain")))
            .expect("ingress plain");
        // An encrypted packet (KK != 0) must never reach `staged` or the app.
        driver
            .ingress(Bytes::from(encrypted_data_bytes(1, ts, b"ciphertext")))
            .expect("ingress encrypted");

        assert!(
            !driver.staged.contains_key(&1),
            "a KK != 0 data packet must never be staged"
        );
        let delivered = drain_deliveries(&mut rx);
        assert!(
            delivered.iter().all(|p| &p[..] != b"ciphertext"),
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
        driver.ingress(Bytes::from(ack_bytes)).expect("ingress ack");
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

    // Paused-clock tests: `Driver` reads `tokio::time::Instant`, so
    // `tokio::time::advance` moves its notion of "now" exactly and instantly —
    // no real sleeping, no dependence on scheduler timing.

    #[tokio::test(start_paused = true)]
    async fn delivery_resumes_after_permanent_loss_once_latency_passes() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let ts0 = 0u32; // the driver epoch is "now" under the paused clock

        // seq 1 is lost and never retransmitted.
        for seq in [0u32, 2, 3] {
            driver
                .ingress(Bytes::from(data_bytes(
                    seq,
                    ts0,
                    &[u8::try_from(seq).unwrap()],
                )))
                .expect("ingress");
        }
        assert!(
            drain_deliveries(&mut rx).is_empty(),
            "play time not yet due"
        );

        // §4.6 / rule 21: at the successor's play time (one latency after
        // send) the gap is skipped and packets 2 and 3 are DELIVERED — not
        // thrown away with the lost packet. Step the clock like the driver's
        // ticker would, finishing well inside the too-late threshold window.
        let mut delivered = Vec::new();
        for _ in 0..40 {
            tokio::time::advance(Duration::from_millis(10)).await;
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

    /// r08-SRT-W2 (adapter side): when TSBPD gives up on a packet (§4.6), the
    /// ARQ receiver's ack point follows it — the "fake ACK" of Too-Late
    /// Packet Drop. Otherwise it keeps NAKing a packet nobody will ever
    /// deliver, the ACKs never let the peer free anything past it, and the
    /// flow window it measures arrivals against stalls at the lost packet.
    #[tokio::test(start_paused = true)]
    async fn a_tsbpd_skip_advances_the_arq_ack_point() {
        let (mut driver, mut rx) = test_driver(8192).await;
        for seq in [0u32, 2, 3] {
            driver
                .ingress(Bytes::from(data_bytes(
                    seq,
                    0,
                    &[u8::try_from(seq).unwrap()],
                )))
                .expect("ingress");
        }
        assert_eq!(driver.receiver.ack_point(), 1, "stalled at the gap");
        assert_eq!(driver.receiver.loss_list_len(), 1, "seq 1 is NAKed");

        tokio::time::advance(Duration::from_millis(130)).await;
        driver.tick_engines();
        assert_eq!(
            drain_deliveries(&mut rx).len(),
            3,
            "0, 2, 3 delivered once the gap is skipped"
        );
        assert_eq!(
            driver.receiver.ack_point(),
            4,
            "the ack point must move past the packet TSBPD dropped"
        );
        assert_eq!(
            driver.receiver.loss_list_len(),
            0,
            "a dropped packet must not stay in the loss list (it would be NAKed forever)"
        );
    }

    /// A delivered packet's payload is a view into the received datagram's own
    /// allocation (header sliced off the front) — one copy from the socket
    /// buffer, not a second `to_vec` per packet (r08-SRT-O4). Pointer identity
    /// is the deterministic evidence: a copy would live at a different address.
    #[tokio::test(start_paused = true)]
    async fn delivery_reuses_the_received_datagrams_allocation() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let datagram = bytes::Bytes::from(data_bytes(0, 0, b"zero-copy payload"));
        let allocation = datagram.as_ptr() as usize;
        driver.ingress(datagram).expect("ingress");
        tokio::time::advance(Duration::from_millis(130)).await;
        driver.tick_engines();
        let mut delivered = drain_deliveries(&mut rx);
        assert_eq!(delivered.len(), 1);
        let payload = delivered.remove(0);
        assert_eq!(&payload[..], b"zero-copy payload");
        assert_eq!(
            payload.as_ptr() as usize,
            allocation + SRT_HEADER_LEN,
            "the payload must be a view into the received datagram, not a copy"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn dropreq_unblocks_delivery_immediately() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let ts0 = 0u32;

        for seq in [0u32, 2, 3] {
            driver
                .ingress(Bytes::from(data_bytes(
                    seq,
                    ts0,
                    &[u8::try_from(seq).unwrap()],
                )))
                .expect("ingress");
        }
        assert!(
            drain_deliveries(&mut rx).is_empty(),
            "play time not yet due"
        );

        // The peer announces it will never send seq 1.
        driver
            .ingress(Bytes::from(dropreq_bytes(1, 1)))
            .expect("ingress dropreq");

        tokio::time::advance(Duration::from_millis(140)).await;
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
                .ingress(Bytes::from(data_bytes(
                    seq,
                    ts,
                    &[u8::try_from(seq).unwrap()],
                )))
                .expect("ingress");
            assert!(
                driver.staged.len() <= usize::try_from(WINDOW).unwrap(),
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
        let mut listener = test_core(HandshakeConfig::default()).await;

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
        let mut listener = test_core(config).await;

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
            lock(&listener.routes).len(),
            1,
            "the freshly accepted connection must have registered exactly one route"
        );

        drop(accepted); // SrtSocket::Drop aborts the driver task.
        drop(caller);

        // The abort is not synchronous with `drop` — the aborted task's
        // `Drop` impls (including `RouteCleanup`) run at the task's own next
        // poll. Yield to the (current-thread) runtime until it has had that
        // poll; no wall-clock sleep, bounded by an iteration count.
        let mut cleaned = false;
        for _ in 0..1_000 {
            tokio::task::yield_now().await;
            if lock(&listener.routes).is_empty() {
                cleaned = true;
                break;
            }
        }
        assert!(
            cleaned,
            "route table still has {} entries after the accepted socket was \
             dropped — the route leaked (pre-fix: `Driver::run`'s cleanup only \
             ran on a normal loop exit, never on `SrtSocket::Drop`'s task abort)",
            lock(&listener.routes).len()
        );
    }

    // -----------------------------------------------------------------------
    // r08-SRT-W6 — liveness: keep-alive, idle timeout, bounded send buffer
    // -----------------------------------------------------------------------

    fn control_kinds(driver: &Driver) -> Vec<&'static str> {
        driver
            .outbound_control
            .iter()
            .map(|b| match ControlPacket::parse(b).expect("control packet") {
                ControlPacket::KeepAlive(_) => "keepalive",
                ControlPacket::Ack(_) => "ack",
                ControlPacket::Nak(_) => "nak",
                ControlPacket::AckAck(_) => "ackack",
                _ => "other",
            })
            .collect()
    }

    fn keepalive_bytes() -> Vec<u8> {
        let ka = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: 0,
            dest_socket_id: 1,
            libsrt_pad: true,
        });
        let mut buf = alloc::vec![0u8; ka.serialized_len()];
        ka.serialize_into(&mut buf).expect("serialize keepalive");
        buf
    }

    /// A connection that has sent nothing for [`KEEPALIVE_PERIOD`] (§3.2.3:
    /// one second) queues a Keep-Alive — not a millisecond earlier — and the
    /// timer restarts from the last packet actually sent.
    #[tokio::test(start_paused = true)]
    async fn an_idle_connection_sends_a_keepalive_after_one_second() {
        let (mut driver, _rx) = test_driver(8192).await;
        // A loopback address accepts the real `send_to` in `flush_outbound`.
        driver.peer_addr = "127.0.0.1:1".parse().unwrap();

        tokio::time::advance(KEEPALIVE_PERIOD - Duration::from_millis(1)).await;
        driver.tick_engines();
        assert!(
            !control_kinds(&driver).contains(&"keepalive"),
            "no Keep-Alive before a full idle second: {:?}",
            control_kinds(&driver)
        );
        driver.outbound_control.clear();

        tokio::time::advance(Duration::from_millis(1)).await;
        driver.tick_engines();
        assert_eq!(
            control_kinds(&driver)
                .iter()
                .filter(|k| **k == "keepalive")
                .count(),
            1,
            "exactly one Keep-Alive once the second has elapsed: {:?}",
            control_kinds(&driver)
        );
        let ka = driver
            .outbound_control
            .iter()
            .find_map(|b| match ControlPacket::parse(b) {
                Ok(ControlPacket::KeepAlive(ka)) => Some(ka),
                _ => None,
            })
            .expect("keepalive");
        assert_eq!(ka.dest_socket_id, 1, "addressed to the peer's socket id");

        // Sending it restarts the idle clock: half a second later, no more.
        driver.flush_outbound().await.expect("flush");
        tokio::time::advance(KEEPALIVE_PERIOD / 2).await;
        driver.tick_engines();
        assert!(!control_kinds(&driver).contains(&"keepalive"));
    }

    /// Traffic in *either* direction counts: a connection that is sending data
    /// is not idle, so no Keep-Alive is added on top.
    #[tokio::test(start_paused = true)]
    async fn queued_data_suppresses_the_keepalive() {
        let (mut driver, _rx) = test_driver(8192).await;
        tokio::time::advance(KEEPALIVE_PERIOD).await;
        driver.send_one(b"payload");
        driver.tick_engines();
        assert!(!control_kinds(&driver).contains(&"keepalive"));
    }

    /// A received Keep-Alive only proves the peer is alive: it is not
    /// answered. (It used to be echoed; with each side now sending its own
    /// Keep-Alives, two adapters would bounce one back and forth forever.)
    #[tokio::test(start_paused = true)]
    async fn a_received_keepalive_is_not_answered() {
        let (mut driver, _rx) = test_driver(8192).await;
        driver
            .ingress(Bytes::from(keepalive_bytes()))
            .expect("ingress");
        assert!(
            driver.outbound_control.is_empty(),
            "a Keep-Alive must not be echoed: {:?}",
            control_kinds(&driver)
        );
        assert!(!driver.peer_shutdown);
    }

    /// Five silent seconds (any packet resets it) end the connection.
    #[tokio::test(start_paused = true)]
    async fn the_peer_idle_timeout_is_five_silent_seconds() {
        let (mut driver, _rx) = test_driver(8192).await;
        // The default (libsrt's SRTO_PEERIDLETIMEO) is five seconds.
        const PEER_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
        assert_eq!(IoConfig::default().read_idle, PEER_IDLE_TIMEOUT);
        tokio::time::advance(PEER_IDLE_TIMEOUT - Duration::from_millis(1)).await;
        assert!(!driver.peer_idle(Instant::now()));
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(driver.peer_idle(Instant::now()));

        // Any datagram from the peer — even a Keep-Alive — resets it.
        driver
            .ingress(Bytes::from(keepalive_bytes()))
            .expect("ingress");
        assert!(!driver.peer_idle(Instant::now()));
        tokio::time::advance(PEER_IDLE_TIMEOUT - Duration::from_millis(1)).await;
        assert!(!driver.peer_idle(Instant::now()));
    }

    /// The sender never holds more than a flow window of unacknowledged
    /// packets: the window closes when full and reopens when an ACK frees it.
    #[tokio::test]
    async fn the_send_window_closes_when_a_full_window_is_unacknowledged() {
        const WINDOW: u32 = 8;
        let (mut driver, _rx) = test_driver(WINDOW).await;
        for sent in 0..WINDOW {
            assert!(driver.send_window_open(), "open after {sent} sends");
            driver.send_one(b"x");
        }
        assert!(!driver.send_window_open(), "a full window closes it");

        let ack = ControlPacket::Ack(AckPacket {
            ack_number: 0,
            timestamp: 0,
            dest_socket_id: 1,
            cif: AckCif::Light {
                last_ack_seq: driver.next_send_seq,
            },
        });
        let mut ack_bytes = alloc::vec![0u8; ack.serialized_len()];
        ack.serialize_into(&mut ack_bytes).expect("serialize ack");
        driver.ingress(Bytes::from(ack_bytes)).expect("ingress ack");
        assert!(driver.send_window_open(), "the ACK freed the window");
    }

    /// `Driver::run` honours the window: with a peer that never acknowledges
    /// anything, only one flow window of the application's payloads is ever
    /// taken off the hand-off channel; the rest wait in it (where
    /// `SrtSocket::send` would block) instead of growing the send buffer.
    #[tokio::test(start_paused = true)]
    async fn run_stops_taking_app_payloads_once_the_window_is_full() {
        const WINDOW: u32 = 8;
        const QUEUED: usize = 24;
        let (mut driver, _rx, _datagrams) = test_driver_with_ingress(WINDOW).await;
        driver.peer_addr = "127.0.0.1:1".parse().unwrap();
        let (app_tx, app_rx) = mpsc::channel::<Bytes>(SEND_CHANNEL_CAPACITY);
        for _ in 0..QUEUED {
            app_tx
                .try_send(Bytes::from(alloc::vec![0u8; 100]))
                .expect("queue payload");
        }
        let handle = tokio::spawn(driver.run(app_rx));
        // Well inside the idle timeout, step the driver's clock a while.
        for _ in 0..50 {
            tokio::time::advance(Duration::from_millis(10)).await;
            tokio::task::yield_now().await;
        }
        let still_queued = app_tx.max_capacity() - app_tx.capacity();
        assert_eq!(
            still_queued,
            QUEUED - usize::try_from(WINDOW).unwrap(),
            "the driver must stop draining at one flow window"
        );
        handle.abort();
    }

    // -----------------------------------------------------------------------
    // r08-SRT-W12 — typed errors
    // -----------------------------------------------------------------------

    /// A datagram that is not an SRT packet is reported with its stage and
    /// the underlying codec error, not flattened to a generic invalid field.
    #[tokio::test]
    async fn listener_parse_failure_keeps_its_stage_and_cause() {
        let mut listener = test_core(HandshakeConfig::default()).await;
        let err = listener
            .handle_datagram(peer_addr(), &[0u8; 4])
            .expect_err("a 4-byte datagram is not an SRT packet");
        let Error::Handshake { stage, source } = err else {
            panic!("expected Error::Handshake, got {err:?}");
        };
        assert_eq!(stage, "parse");
        assert!(
            matches!(*source, Error::BufferTooShort { have: 4, .. }),
            "the cause must be preserved, got {source:?}"
        );
    }

    /// A rejection the engine produces surfaces as `Error::Rejected` with the
    /// reason — a feed after the handshake already finished is the engine's
    /// own out-of-sequence error, wrapped with its stage.
    #[tokio::test]
    async fn listener_feed_failure_is_wrapped_with_the_engine_error() {
        let mut listener = test_core(HandshakeConfig::default()).await;
        // A CONCLUSION as the very first packet from a source: the fresh
        // engine expects an INDUCTION.
        let conclusion = {
            let hp = HandshakePacket {
                timestamp: 0,
                dest_socket_id: 0,
                version: crate::handshake_sm::HANDSHAKE_VERSION_5,
                encryption_field: crate::packet::EncryptionField::NoEncryption,
                extension_field: HandshakeExtensionFlags(0),
                initial_seq_number: 0,
                mtu: 1500,
                max_flow_window_size: 8192,
                handshake_type: crate::packet::HandshakeType::Conclusion,
                srt_socket_id: 5,
                syn_cookie: 0,
                peer_ip: [0; 4],
                extensions: HandshakeExtensions(&[]),
            };
            let pkt = ControlPacket::Handshake(hp);
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut buf).unwrap();
            buf
        };
        let err = listener
            .handle_datagram(peer_addr(), &conclusion)
            .expect_err("a CONCLUSION before an INDUCTION");
        let Error::Handshake { stage, source } = err else {
            panic!("expected Error::Handshake, got {err:?}");
        };
        assert_eq!(stage, "listener feed");
        assert!(matches!(*source, Error::HandshakeOutOfSequence { .. }));
    }

    // -----------------------------------------------------------------------
    // #1134 — a poisoned lock must not cascade
    // -----------------------------------------------------------------------

    /// A connection task that panics while holding the route table's lock
    /// poisons it. Dropping a `RouteCleanup` (it runs in `Drop`, including
    /// while unwinding) and the routing pump's lookups must keep working, not
    /// panic a second time.
    #[test]
    fn a_poisoned_route_table_does_not_panic_the_cleanup_guard() {
        let routes: RouteTable = Arc::new(Mutex::new(std::collections::HashMap::new()));
        {
            let routes = Arc::clone(&routes);
            let panicked = std::thread::spawn(move || {
                let _held = routes.lock().expect("lock");
                panic!("a connection task panicked holding the route table lock");
            })
            .join();
            assert!(panicked.is_err());
        }
        assert!(routes.is_poisoned(), "the lock must really be poisoned");

        let (tx, _rx) = mpsc::channel(1);
        lock(&routes).insert(
            7,
            RouteEntry {
                addr: peer_addr(),
                tx,
                stats: Arc::new(Counters::default()),
            },
        );
        assert_eq!(lock(&routes).len(), 1);
        drop(RouteCleanup(Some((7, Arc::clone(&routes)))));
        assert!(
            lock(&routes).is_empty(),
            "the guard must still remove its entry"
        );
    }

    // -----------------------------------------------------------------------
    // r08-SRT-W9 — the time base is seeded from the handshake
    // -----------------------------------------------------------------------

    /// `TsbpdTimeBase = T_NOW - HSREQ_TIMESTAMP` with `T_NOW = 0` at the
    /// epoch is minus the peer's handshake timestamp (rule 12).
    #[test]
    fn the_time_base_is_minus_the_peer_handshake_timestamp() {
        assert_eq!(tsbpd_time_base_from(0), 0);
        assert_eq!(tsbpd_time_base_from(3_000_000), -3_000_000);
        assert_eq!(tsbpd_time_base_from(u32::MAX), -i64::from(u32::MAX));
    }

    /// A peer whose clock already reads 30 s when the handshake completes
    /// (its first data packet carries `Timestamp` ~30 s) is delivered one
    /// latency after arrival — not 30 s later, which is what a hard-coded
    /// zero base did (the packet's play time was `0 + 30 s + latency`).
    #[tokio::test(start_paused = true)]
    async fn a_peer_far_into_its_own_clock_is_not_delayed_by_its_timestamp() {
        const PEER_CLOCK_US: u32 = 30_000_000;
        let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (deliver, mut rx) = mpsc::channel(DELIVER_CHANNEL_CAPACITY);
        let (_datagram_tx, datagram_rx) = mpsc::channel(RX_CHANNEL_CAPACITY);
        let mut driver = Driver::new(
            ConnParams {
                udp,
                peer_addr: peer_addr(),
                rx: datagram_rx,
                stats: Arc::new(Counters::default()),
                route_cleanup: RouteCleanup(None),
                forwarder_guard: TaskGuard(None),
                life: None,
                read_idle: IoConfig::default().read_idle,
                write: IoConfig::default().write,
                our_initial_seq: 0,
                peer_initial_seq: 0,
                peer_socket_id: 1,
                tsbpd_time_base: tsbpd_time_base_from(PEER_CLOCK_US),
                tsbpd_delay_ms: LATENCY_MS,
                epoch: Instant::now(),
                max_flow_window: 8192,
                mtu: 1500,
                tlpkt_drop: true,
            },
            deliver,
        );
        driver
            .ingress(Bytes::from(data_bytes(0, PEER_CLOCK_US, b"first")))
            .expect("ingress");
        assert!(drain_deliveries(&mut rx).is_empty(), "not due on arrival");
        tokio::time::advance(Duration::from_millis(LATENCY_MS)).await;
        driver.tick_engines();
        assert_eq!(
            drain_deliveries(&mut rx),
            vec![b"first".to_vec()],
            "delivered one latency after arrival"
        );
    }

    // -----------------------------------------------------------------------
    // Pacing schedule (issue #1061), deterministically
    // -----------------------------------------------------------------------

    /// Under a paused clock the token-bucket schedule is exact: the first DATA
    /// packet goes at once, the next not a nanosecond before one pacing period
    /// later, and a burst after an idle gap is not penalized (it sends
    /// immediately, then resumes the period).
    #[tokio::test(start_paused = true)]
    async fn data_leaves_one_packet_per_pacing_period() {
        let (mut driver, _rx) = test_driver(8192).await;
        driver.peer_addr = "127.0.0.1:1".parse().unwrap();
        const BACKLOG: usize = 60;
        for _ in 0..BACKLOG {
            driver.send_one(&[0u8; 1316]);
        }
        let period = driver.livecc.on_ack_received();
        assert!(period > Duration::ZERO, "the default MAX_BW paces DATA");

        driver.flush_outbound().await.expect("flush");
        assert_eq!(
            driver.outbound_data.len(),
            BACKLOG - 1,
            "the first packet goes at once"
        );

        tokio::time::advance(period - Duration::from_nanos(1)).await;
        driver.flush_outbound().await.expect("flush");
        assert_eq!(
            driver.outbound_data.len(),
            BACKLOG - 1,
            "one nanosecond short of the slot"
        );

        tokio::time::advance(Duration::from_nanos(1)).await;
        driver.flush_outbound().await.expect("flush");
        assert_eq!(
            driver.outbound_data.len(),
            BACKLOG - 2,
            "the slot arrived: one more"
        );

        // A long stall does not let the backlog out in one burst: the late
        // wake claims at most `PACING_BURST_CAP` packets of catch-up (it used
        // to send exactly one and lose the rest of the credit).
        tokio::time::advance(period * 100).await;
        let before = driver.outbound_data.len();
        driver.flush_outbound().await.expect("flush");
        let sent = before - driver.outbound_data.len();
        assert_eq!(
            sent, PACING_BURST_CAP,
            "a stalled wake must send exactly the burst cap"
        );
        assert!(!driver.outbound_data.is_empty());
        assert!(driver.next_paced_send_wait().is_some());
    }

    /// r08-SRT-O2: a retransmission feeds LiveCC the packet's *payload* length
    /// (the datagram less its fixed header), exactly as a first transmission
    /// does — read from the length instead of re-parsing the packet.
    #[tokio::test(start_paused = true)]
    async fn a_retransmission_is_accounted_by_its_payload_length() {
        use crate::packet::nak::build_loss_list;
        use crate::packet::{LossListEntry, NakPacket};
        let (mut driver, _rx) = test_driver(8192).await;
        driver.send_one(&[0u8; 1000]); // seq 0
        let raw = build_loss_list(&[LossListEntry::Single(0)]).expect("loss list");
        let nak = ControlPacket::Nak(NakPacket {
            timestamp: 0,
            dest_socket_id: 1,
            raw_loss_list: &raw,
        });
        let mut bytes = alloc::vec![0u8; nak.serialized_len()];
        nak.serialize_into(&mut bytes).expect("serialize nak");
        driver.ingress(Bytes::from(bytes)).expect("ingress nak");
        driver.tick_engines();
        assert_eq!(
            driver.outbound_data.len(),
            2,
            "the packet and its retransmit"
        );

        // The reference sees two 1000-byte payloads; a header-inclusive length
        // (1016) would leave the average two bytes higher.
        let mut reference = LiveCC::new(DEFAULT_MAX_BW);
        reference.on_data_packet(1000);
        reference.on_data_packet(1000);
        assert_eq!(
            driver.livecc.avg_payload_size(),
            reference.avg_payload_size()
        );
    }

    /// When the application handle is gone and the driver ends cooperatively
    /// (not by abort), it still tells the peer it is leaving (§3.2.7). (Real
    /// clock, bounded real waits: a paused clock would auto-advance past the
    /// `recv` timeout before the loopback datagram is delivered.)
    #[tokio::test]
    async fn a_driver_ending_because_its_handle_is_gone_sends_shutdown() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (mut driver, _rx, _datagrams) = test_driver_with_ingress(8192).await;
        driver.peer_addr = peer.local_addr().unwrap();
        let (app_tx, app_rx) = mpsc::channel::<Bytes>(SEND_CHANNEL_CAPACITY);
        let handle = tokio::spawn(driver.run(app_rx));
        drop(app_tx);
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("the driver must end once its handle is gone")
            .expect("join");

        let mut buf = [0u8; 2048];
        let shutdown = loop {
            let n = tokio::time::timeout(Duration::from_secs(5), peer.recv(&mut buf))
                .await
                .expect("no SHUTDOWN reached the peer")
                .expect("recv");
            if let Ok(SrtPacket::Control(ControlPacket::Shutdown(s))) = SrtPacket::parse(&buf[..n])
            {
                break s;
            }
        };
        assert_eq!(
            shutdown.dest_socket_id, 1,
            "addressed to the peer's socket id"
        );
    }

    /// A hostile `retransmit_after_ticks` must not make the handshake timer's
    /// period zero (which would panic the connect).
    #[tokio::test]
    async fn an_absurd_retransmit_tick_count_does_not_panic_the_connect() {
        let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        for ticks in [0, 1, u32::MAX] {
            let config = HandshakeConfig {
                retransmit_after_ticks: ticks,
                ..HandshakeConfig::default()
            };
            // Nobody answers; the point is only that it neither panics nor
            // fails immediately.
            let outcome = tokio::time::timeout(
                Duration::from_millis(150),
                SrtSocket::connect(silent.local_addr().unwrap(), config),
            )
            .await;
            assert!(
                outcome.is_err(),
                "ticks={ticks}: still waiting for the peer"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Stale sequence numbers, and the negotiated TLPKTDROP flag
    // -----------------------------------------------------------------------

    /// Packets behind the delivery cursor are never staged: with the cursor at
    /// 3, a hundred thousand distinct stale sequence numbers (each stamped
    /// with a far-future timestamp, so TSBPD would hold them) leave `staged`
    /// empty and are counted as late. They used to be staged, ignored by
    /// TSBPD and never removed.
    #[tokio::test(start_paused = true)]
    async fn stale_sequence_numbers_are_never_staged() {
        let (mut driver, mut rx) = test_driver(8192).await;
        for seq in 0..3u32 {
            driver
                .ingress(Bytes::from(data_bytes(
                    seq,
                    0,
                    &[u8::try_from(seq).unwrap()],
                )))
                .expect("ingress");
        }
        tokio::time::advance(Duration::from_millis(130)).await;
        driver.tick_engines();
        assert_eq!(drain_deliveries(&mut rx).len(), 3);
        assert_eq!(driver.tsbpd.next_release(), 3);

        const STALE: u32 = 100_000;
        for back in 1..=STALE {
            // 2^31 - back is `back` behind the cursor at 0..3 in circular order.
            let seq = (1u32 << 31) - back;
            driver
                .ingress(Bytes::from(data_bytes(seq, u32::MAX, b"x")))
                .expect("ingress");
        }
        assert!(
            driver.staged.is_empty(),
            "{} stale packets staged",
            driver.staged.len()
        );
        assert_eq!(
            driver.stats.late_dropped.load(Ordering::Relaxed),
            u64::from(STALE)
        );
        // A duplicate of something already delivered is late too.
        driver
            .ingress(Bytes::from(data_bytes(1, 0, b"dup")))
            .expect("ingress");
        assert!(driver.staged.is_empty());
    }

    /// Without a negotiated TLPKTDROP the receiver never skips a gap: it waits
    /// for the retransmission, delivers nothing past the hole however long it
    /// takes, and never moves its ack point past it (which would tell the
    /// sender a packet that never arrived was received).
    #[tokio::test(start_paused = true)]
    async fn without_negotiated_tlpktdrop_a_gap_is_waited_out_not_skipped() {
        let (mut driver, mut rx) = test_driver(8192).await;
        driver.tsbpd = TsbpdScheduler::new(0, 0, LATENCY_MS, DEFAULT_DRIFT_US, false, None);
        for seq in [0u32, 2, 3] {
            driver
                .ingress(Bytes::from(data_bytes(
                    seq,
                    0,
                    &[u8::try_from(seq).unwrap()],
                )))
                .expect("ingress");
        }
        // Far beyond the latency (and the too-late threshold).
        for _ in 0..50 {
            tokio::time::advance(Duration::from_millis(100)).await;
            driver.tick_engines();
        }
        let delivered = drain_deliveries(&mut rx);
        assert_eq!(
            delivered.iter().map(|p| p[0]).collect::<Vec<_>>(),
            vec![0],
            "nothing may pass the hole"
        );
        assert_eq!(driver.receiver.ack_point(), 1, "never ACK past a gap");
        assert_eq!(driver.receiver.loss_list_len(), 1, "still asking for seq 1");

        // The retransmission arrives: everything flows, in order.
        driver
            .ingress(Bytes::from(data_bytes(1, 0, &[1])))
            .expect("ingress");
        driver.tick_engines();
        let delivered = drain_deliveries(&mut rx);
        assert_eq!(
            delivered.iter().map(|p| p[0]).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(driver.receiver.ack_point(), 4);
    }

    /// The negotiated flag, not a constant, decides: a connection negotiated
    /// with TLPKTDROP skips (the existing live-mode tests), one without does
    /// not — pinned through `Driver::new`'s own configuration.
    #[tokio::test]
    async fn the_driver_honours_the_negotiated_tlpktdrop_flag() {
        for negotiated in [true, false] {
            let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            let (deliver, _rx) = mpsc::channel(1);
            let (_tx, datagram_rx) = mpsc::channel(1);
            let driver = Driver::new(
                ConnParams {
                    udp,
                    peer_addr: peer_addr(),
                    rx: datagram_rx,
                    stats: Arc::new(Counters::default()),
                    route_cleanup: RouteCleanup(None),
                    forwarder_guard: TaskGuard(None),
                    life: None,
                    read_idle: IoConfig::default().read_idle,
                    write: IoConfig::default().write,
                    our_initial_seq: 0,
                    peer_initial_seq: 0,
                    peer_socket_id: 1,
                    tsbpd_time_base: 0,
                    tsbpd_delay_ms: LATENCY_MS,
                    epoch: Instant::now(),
                    max_flow_window: 8192,
                    mtu: 1500,
                    tlpkt_drop: negotiated,
                },
                deliver,
            );
            assert_eq!(driver.tsbpd.tlpktdrop_enabled(), negotiated);
        }
    }

    // -----------------------------------------------------------------------
    // #1142 / O5 — the cookie key is secret, 128 bits, and rotates
    // -----------------------------------------------------------------------

    /// `{:?}` of a listener (and of its key holder) never prints the cookie
    /// key, in decimal or hex.
    #[tokio::test]
    async fn listener_debug_never_prints_the_cookie_key() {
        let listener = test_core(HandshakeConfig::default()).await;
        let key = listener.cookie_keys.current();
        assert!(
            key > u128::from(u64::MAX),
            "a 128-bit key, not a 64-bit one"
        );
        for dump in [
            alloc::format!("{listener:?}"),
            alloc::format!("{listener:#?}"),
            alloc::format!("{listener:x?}"),
        ] {
            for needle in [
                alloc::format!("{key}"),
                alloc::format!("{key:x}"),
                alloc::format!("{:x}", key >> 64),
            ] {
                assert!(!dump.contains(&needle), "key leaked as {needle:?}: {dump}");
            }
            assert!(dump.contains("redacted"), "{dump}");
        }
    }

    fn induction_cookie(listener: &mut ListenerCore, src: std::net::SocketAddr) -> u32 {
        listener
            .handle_datagram(src, &induction_bytes(7))
            .expect("induction");
        let response = listener
            .outbound_queue
            .get_mut(&src)
            .and_then(VecDeque::pop_back)
            .expect("an INDUCTION response");
        match SrtPacket::parse(&response).expect("parse") {
            SrtPacket::Control(ControlPacket::Handshake(hp)) => hp.syn_cookie,
            other => panic!("expected a handshake, got {other:?}"),
        }
    }

    /// The key is replaced exactly once a minute is up; cookies issued after
    /// differ from those before for the same peer.
    #[tokio::test]
    async fn the_cookie_key_rotates_every_minute() {
        let mut listener = test_core(HandshakeConfig::default()).await;
        let t0 = listener.cookie_keys.drawn_at;
        let first = listener.cookie_keys.current();

        assert!(
            !listener
                .cookie_keys
                .rotate_if_due(t0 + COOKIE_KEY_ROTATION - Duration::from_millis(1))
        );
        assert_eq!(listener.cookie_keys.current(), first, "not yet due");

        let src = peer_addr();
        let before = induction_cookie(&mut listener, src);
        listener.pending.clear();
        assert!(listener.cookie_keys.rotate_if_due(t0 + COOKIE_KEY_ROTATION));
        assert_ne!(
            listener.cookie_keys.current(),
            first,
            "a fresh key was drawn"
        );
        assert_eq!(
            listener.cookie_keys.drawn_at,
            t0 + COOKIE_KEY_ROTATION,
            "the next rotation is measured from this one"
        );
        let after = induction_cookie(&mut listener, src);
        assert_ne!(
            before, after,
            "the same peer gets a different cookie after rotation"
        );
    }

    /// A handshake begun under the previous key still completes after the key
    /// rotates: its pending entry keeps the cookie it was issued.
    #[tokio::test]
    async fn a_handshake_begun_before_a_rotation_still_completes() {
        use crate::caller::CallerHandshake;
        let mut listener = test_core(HandshakeConfig::default()).await;
        let src = peer_addr();
        let mut caller = CallerHandshake::new(0x0000_0777, HandshakeConfig::default());
        let induction = caller.start().expect("start");
        listener
            .handle_datagram(src, &induction)
            .expect("induction");
        let response = listener
            .outbound_queue
            .get_mut(&src)
            .and_then(VecDeque::pop_back)
            .expect("response");

        let t0 = listener.cookie_keys.drawn_at;
        assert!(listener.cookie_keys.rotate_if_due(t0 + COOKIE_KEY_ROTATION));

        let conclusion = caller
            .feed_bytes(&response)
            .expect("feed")
            .into_iter()
            .find_map(|o| match o {
                HandshakeOutput::Send(b) => Some(b),
                _ => None,
            })
            .expect("CONCLUSION");
        listener
            .handle_datagram(src, &conclusion)
            .expect("the CONCLUSION must be accepted across the rotation");
        assert!(
            listener.pending[&src].params.is_some(),
            "the handshake must have completed"
        );
        // The connection epoch is when the CONCLUSION arrived, and a repeat
        // does not move it.
        let concluded = listener.pending[&src].concluded_at.expect("recorded");
        listener
            .handle_datagram(src, &conclusion)
            .expect("a repeated CONCLUSION");
        assert_eq!(listener.pending[&src].concluded_at, Some(concluded));
    }
}
