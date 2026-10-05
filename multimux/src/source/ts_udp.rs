//! MPEG-2 Transport Stream over UDP ingest source (issue #663 P3a; ported
//! onto the media-plane ingress traits at plan step 5a): a UDP socket
//! (unicast or multicast) feeding the shared
//! [`crate::source::ts_program::TsIngestSession`].
//!
//! This module owns **only the socket**. All PAT/PMT/PES demuxing,
//! codec-config recovery, and `DemuxEvent`→`SessionEvent` translation
//! (including the B5 mid-stream `NewProgram` handling) live in
//! [`crate::source::ts_program`], shared verbatim with
//! [`crate::source::ts_http`] and [`crate::source::srt`].
//!
//! # Why this source fits the sans-IO reshape with zero executor bridge
//!
//! UDP is connectionless: binding a local socket is a purely local operation
//! (no peer round-trip at all), so [`TsUdpDialer::dial`] performs **no I/O**
//! — it just constructs a fresh `TsIngestSession`, which immediately queues
//! [`media_plane::ingress::SessionEvent::Established`]. The actual
//! `UdpSocket::bind` (still real I/O, just never a multi-round-trip
//! *handshake*) happens in [`bind`]/[`run_ts_udp`], the multimux-side driver
//! that owns the socket and pumps
//! [`media_plane::ingress::IngestDriver`] — exactly where the plan
//! (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md` step 5)
//! says tokio belongs.

use std::convert::Infallible;
use std::time::Duration;

use broadcast_common::Timestamp;
use media_plane::ingress::Dialer;
use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};
use crate::source::ts_program::TsIngestSession;
use crate::source::udp::bind_udp;
use crate::source::{IngestTimeouts, MAX_TS_READ, Source};

/// An MPEG-2 TS-over-UDP route: bind address (+ optional multicast group) —
/// no control plane, no out-of-band SDP (the PMT carries the track set
/// in-band, unlike raw RTP/UDP). Replaces the old (pre-5a) `TsUdpSource`;
/// [`run_ts_udp`] is the new `connect()`+`next_samples()` loop, now driving
/// [`media_plane::ingress::IngestDriver`] instead of hand-rolling its own
/// demux drain.
#[derive(Clone)]
pub struct TsUdpRoute {
    name: String,
    addr: String,
    multicast_group: Option<String>,
    /// Socket options for the bind (SP1.6); the default (all unset except
    /// `reuse_address: false`) reproduces the pre-SP1.6 bind.
    socket: crate::source::udp::UdpBindOptions,
    timeouts: IngestTimeouts,
    /// A socket bound by the caller (SP7.1): a test binds `127.0.0.1:0`, reads
    /// the live address, and hands the socket in, so `bind` uses it directly
    /// and never races reserve-then-rebind. `None` (the production shape)
    /// makes `bind` bind `addr` itself.
    prebound: std::sync::Arc<std::sync::Mutex<Option<UdpSocket>>>,
}

impl std::fmt::Debug for TsUdpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsUdpRoute")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("multicast_group", &self.multicast_group)
            .field("socket", &self.socket)
            .finish()
    }
}

impl TsUdpRoute {
    /// Build a route descriptor.
    pub fn new(
        name: impl Into<String>,
        addr: impl Into<String>,
        multicast_group: Option<String>,
    ) -> Self {
        TsUdpRoute {
            name: name.into(),
            addr: addr.into(),
            multicast_group,
            socket: crate::source::udp::UdpBindOptions::default(),
            timeouts: IngestTimeouts::default(),
            prebound: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Build a route over an already-bound UDP socket (SP7.1): a test binds
    /// `127.0.0.1:0`, learns the port, and hands the socket in, rather than
    /// racing reserve-then-rebind. Mirrors
    /// `crate::source::whip::WhipRoute::with_listener`.
    ///
    /// The supplied socket is used verbatim, so a `multicast_group` cannot be
    /// joined on it: setting one (via [`Self::with_multicast_group`]) makes
    /// [`bind`] return an error rather than silently ignore the group.
    pub fn with_socket(name: impl Into<String>, socket: UdpSocket) -> Self {
        let addr = socket
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "127.0.0.1:0".to_string());
        TsUdpRoute {
            name: name.into(),
            addr,
            multicast_group: None,
            socket: crate::source::udp::UdpBindOptions::default(),
            timeouts: IngestTimeouts::default(),
            prebound: std::sync::Arc::new(std::sync::Mutex::new(Some(socket))),
        }
    }

    /// Configure the multicast group to join. Meaningful only for a route that
    /// binds its own `addr`; on a [`Self::with_socket`] route it is a
    /// contradiction (`bind` returns an error rather than silently dropping
    /// it) — see that ctor.
    #[must_use]
    pub fn with_multicast_group(mut self, group: impl Into<String>) -> Self {
        self.multicast_group = Some(group.into());
        self
    }

    /// Set the UDP socket options applied at bind (SP1.6).
    #[must_use]
    pub fn with_socket_options(mut self, socket: crate::source::udp::UdpBindOptions) -> Self {
        self.socket = socket;
        self
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

impl Source for TsUdpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Constructs a [`TsIngestSession`] — performs **no I/O** (see the module
/// doc's "zero executor bridge" section).
#[derive(Clone, Copy, Debug, Default)]
pub struct TsUdpDialer;

impl Dialer for TsUdpDialer {
    type Session = TsIngestSession;
    /// Construction cannot fail — there is no fallible local step (unlike,
    /// say, parsing a URL).
    type Error = Infallible;

    fn dial(&mut self) -> core::result::Result<TsIngestSession, Infallible> {
        Ok(TsIngestSession::new())
    }
}

/// Binds `route`'s UDP socket and returns it, ready for
/// [`recv_and_feed`]/[`run_ts_udp`] — split out so tests can synchronise on
/// the bound address before a synthetic sender starts writing to it (UDP has
/// no connect-then-accept handshake to synchronise on otherwise).
///
/// A caller-supplied pre-bound socket ([`TsUdpRoute::with_socket`]) is
/// consumed **once**: the first `bind` takes it, and every later call (a
/// reconnect after the read loop ends) falls back to binding `route.addr`.
/// The pre-bound socket exists only so a *test* can own the exact port for its
/// first attempt; a production route never sets one, so a reconnect always
/// re-binds its configured `addr`, exactly as before `with_socket` existed.
pub async fn bind(route: &TsUdpRoute) -> Result<UdpSocket> {
    let prebound = match route.prebound.lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(socket) = prebound {
        // A caller-supplied socket is used as-is; joining a multicast group on
        // it is not implemented, so refuse rather than silently ignoring the
        // configured group (which would leave the operator's multicast source
        // unreachable with no signal).
        if let Some(group) = route.multicast_group.as_deref() {
            return Err(crate::MultimuxError::ConfigInvalid {
                field: "routes.input.multicast_group",
                reason: format!(
                    "a pre-bound socket cannot join multicast_group {group:?}; drop the group, \
                     or let the route bind its own addr"
                ),
            });
        }
        return Ok(socket);
    }
    bind_udp(
        &route.addr,
        route.multicast_group.as_deref(),
        route.socket.clone(),
    )
    .await
}

/// Reads one datagram from `socket` (bounded by `read_timeout`) and feeds it
/// to `driver`. Returns the number of bytes read (and fed) on a normal
/// read, or `Err` on a read stall or socket error — UDP is connectionless,
/// so unlike a TCP/HTTP source there is no transport-level clean
/// end-of-stream; every termination here is reported as an I/O-layer error
/// for the caller (production: the route supervisor) to reconnect on,
/// exactly as the pre-5a `next_samples` did.
///
/// The returned count lets the caller also feed the same bytes to the
/// route's DVR EIT tracker (`RouteHandle::feed_si_ts`, crate-private —
/// issue #903) without an extra allocation: `buf` is a reused buffer, so
/// only `&buf[..n]` is this read's actual data.
pub async fn recv_and_feed(
    socket: &UdpSocket,
    buf: &mut [u8],
    driver: &mut media_plane::ingress::IngestDriver<TsIngestSession>,
    read_timeout: Duration,
    now: Timestamp,
) -> Result<usize> {
    let n = tokio::time::timeout(read_timeout, socket.recv(buf))
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!("ts/udp recv: no data within {read_timeout:?}"),
        })?
        .map_err(|e| MultimuxError::Connect {
            reason: format!("udp recv: {e}"),
        })?;
    driver.feed(&buf[..n], now);
    Ok(n)
}

/// The ts/udp source's [`IngestStep`](crate::source::driver::IngestStep):
/// one bounded datagram receive, feeding the driver and the route's DVR EIT
/// tracker (`feed_si_ts`).
pub(crate) struct TsUdpStep {
    pub socket: UdpSocket,
    pub buf: Vec<u8>,
    pub start: tokio::time::Instant,
    pub route_handle: std::sync::Arc<crate::route::RouteHandle>,
}

impl crate::source::driver::IngestStep<TsIngestSession> for TsUdpStep {
    async fn step(
        &mut self,
        driver: &mut media_plane::ingress::IngestDriver<TsIngestSession>,
        window: Duration,
    ) -> crate::source::driver::StepOutcome {
        use crate::source::driver::StepOutcome;
        match tokio::time::timeout(window, self.socket.recv(&mut self.buf)).await {
            Ok(Ok(n)) => {
                let now = Timestamp::from_instant(
                    self.start.into_std(),
                    tokio::time::Instant::now().into_std(),
                );
                driver.feed(&self.buf[..n], now);
                // EIT p/f tracking (issue #903) — a no-op unless some program
                // on this route has DVR enabled with `dvb_service_id` set.
                self.route_handle.feed_si_ts(&self.buf[..n]);
                StepOutcome::Received
            }
            Ok(Err(e)) => StepOutcome::Failed(MultimuxError::Connect {
                reason: format!("udp recv: {e}"),
            }),
            Err(_) => StepOutcome::Idle,
        }
    }

    fn stalled(&self, read: Duration) -> MultimuxError {
        MultimuxError::Connect {
            reason: format!("ts/udp recv: no data within {read:?}"),
        }
    }
}

/// Binds `route`'s socket and drives a fresh [`TsIngestSession`] through
/// [`media_plane::ingress::IngestDriver`] until a read stall (bounded by
/// [`IngestTimeouts::read`]) — the new `connect()`+`next_samples()` loop,
/// replacing the pre-5a `TsUdpSource`/`TsUdpSession` pair. Returns the error
/// that ended the loop (always a read-side error — see [`recv_and_feed`]);
/// the caller (the route supervisor) reconnects on it.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_ts_udp(
    route: &TsUdpRoute,
    trunk_config: media_plane::trunk::TrunkConfig,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> MultimuxError {
    let socket = match bind(route).await {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut dialer = TsUdpDialer;
    let session = dialer
        .dial()
        .unwrap_or_else(|never: Infallible| match never {});
    let driver = media_plane::ingress::IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let mut step = TsUdpStep {
        socket,
        buf: vec![0u8; MAX_TS_READ],
        start: tokio::time::Instant::now(),
        route_handle: std::sync::Arc::clone(route_handle),
    };
    let mut progress = crate::source::DriverProgress::new();
    // A connectionless datagram source has no `poll_transmit`, so the write
    // half is a sink; the scaffold still bounds it (defect 4 discipline).
    let mut sink = tokio::io::sink();
    crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        &mut progress,
        cancel,
        route.timeouts,
        crate::source::driver::DEFAULT_WRITE_TIMEOUT,
        &mut sink,
        &mut step,
        |never: Infallible| match never {},
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use media_plane::ingress::{HealthState, IngestDriver, ProgramId};

    #[tokio::test]
    async fn loopback_udp_established_and_samples_land_in_trunk() {
        // Bind `127.0.0.1:0` and hand the LIVE socket to `with_socket` — no
        // reserve-then-rebind window (SP7.1).
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("local addr");
        let route = TsUdpRoute::with_socket("cam-ts", socket);
        // Enough samples (spread over several 7*188-byte datagrams, each
        // paced 5ms apart) that the PMT resolves on an early datagram while
        // later datagrams still carry fresh samples — otherwise the whole
        // stream could fit in the single datagram that also resolves the
        // PMT, and `Trunk::subscribe`'s "starting from now" contract (no
        // backlog) would have nothing left to observe.
        let ts_bytes = build_ts_bytes(1, 0xAB, 60);
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");

        let socket = bind(&route).await.expect("bind route socket");
        let send_task = tokio::spawn(async move {
            for chunk in ts_bytes.chunks(7 * 188) {
                sender.send_to(chunk, addr).await.expect("send TS datagram");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let mut dialer = TsUdpDialer;
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        assert!(matches!(driver.health(), HealthState::Establishing));

        let mut buf = vec![0u8; MAX_TS_READ];
        let mut cursor = None;
        let mut saw_sample = false;
        for i in 0..200u64 {
            // HANG GUARD (issue #826): per-iteration read timeout in a
            // real-socket loopback test. Real UDP on loopback resolves in
            // ~ms; this only exists to keep the 200-iteration loop from
            // spinning on an empty socket, not a timing claim.
            let read = recv_and_feed(
                &socket,
                &mut buf,
                &mut driver,
                Duration::from_secs(60),
                Timestamp::from_nanos(i),
            )
            .await;
            if read.is_err() {
                break;
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                while let Some(item) = c.poll() {
                    if matches!(item, media_plane::trunk::SampleCursorItem::Timed { .. }) {
                        saw_sample = true;
                    }
                }
            }
            if saw_sample {
                break;
            }
        }
        let _ = send_task.await;
        assert!(matches!(driver.health(), HealthState::Live));
        assert!(
            saw_sample,
            "expected at least one sample published into the Trunk"
        );
    }

    /// A source that stops sending datagrams must fail (not hang) within a
    /// bounded multiple of the configured read timeout — issue #663 P5.2,
    /// preserved from the pre-5a test of the same intent.
    #[tokio::test]
    async fn recv_and_feed_times_out_when_source_goes_silent() {
        let reserved = UdpSocket::bind("127.0.0.1:0").await.expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        const READ_TIMEOUT: Duration = Duration::from_secs(2);
        let route = TsUdpRoute::new("cam-ts", addr.to_string(), None);
        let socket = bind(&route).await.expect("bind");

        let mut dialer = TsUdpDialer;
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut buf = vec![0u8; MAX_TS_READ];

        // DISCRIMINATOR (issue #826): the assertion window must prove the
        // operation returns through the CONFIGURED read timeout, not via
        // any longer default/system timeout. Widen the gap rather than
        // only the assertion side: the configured READ_TIMEOUT was raised
        // from 100ms to 2s (generous for scheduling under load) and the
        // assertion window from 500ms to 10s — 10s is still well below
        // any plausible fallback, so a broken implementation that ignores
        // the configured timeout and falls through to a 30+s system
        // default is still caught. MUTATION CHECKED: inflating
        // READ_TIMEOUT to 30s makes the 10s outer timeout fire first,
        // producing `Elapsed` instead of `recv_and_feed`'s own error,
        // proving this bound still discriminates.
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            recv_and_feed(
                &socket,
                &mut buf,
                &mut driver,
                READ_TIMEOUT,
                Timestamp::ZERO,
            ),
        )
        .await
        .expect("recv_and_feed must not exceed the assertion window");
        assert!(
            outcome.is_err(),
            "expected a recoverable read-timeout error"
        );
    }

    /// A pre-bound socket handed to [`TsUdpRoute::with_socket`] is consumed
    /// **once**: the first [`bind`] takes it (its address equals the socket
    /// the caller bound), and a second `bind` (a reconnect after the read loop
    /// ends) falls back to binding `route.addr` — a *fresh* socket, not the
    /// pre-bound one. Pins the documented reconnect-after-take behaviour.
    #[tokio::test]
    async fn with_socket_is_consumed_once_then_reconnect_rebinds_addr() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("local addr");
        let route = TsUdpRoute::with_socket("cam-ts", socket);
        // The pre-bound socket is present until the first `bind` takes it.
        assert!(
            route
                .prebound
                .lock()
                .expect("prebound lock")
                .as_ref()
                .is_some(),
            "with_socket must install the caller's socket"
        );

        // First bind: the caller's own pre-bound socket (same address), and the
        // slot is now empty (consumed once).
        let first = bind(&route).await.expect("first bind");
        assert_eq!(
            first.local_addr().expect("addr"),
            addr,
            "the first bind must return the caller's pre-bound socket"
        );
        assert!(
            route
                .prebound
                .lock()
                .expect("prebound lock")
                .as_ref()
                .is_none(),
            "the pre-bound socket must be consumed by the first bind"
        );
        // Drop the first socket so the port is free for the reconnect's bind.
        drop(first);

        // Second bind (reconnect): the pre-bound socket is gone, so the route
        // re-binds its own `addr`.
        let second = bind(&route).await.expect("second bind");
        assert_eq!(
            second.local_addr().expect("addr"),
            addr,
            "the reconnect must re-bind the configured addr"
        );
    }

    /// A `with_socket` route carrying a configured `multicast_group` must be
    /// REJECTED by `bind`, not silently use the caller's socket without joining
    /// the group (which would leave the multicast source unreachable with no
    /// signal). PRE-FIX `bind` returned the pre-bound socket and dropped the
    /// group, so this returned `Ok`.
    #[tokio::test]
    async fn with_socket_rejects_a_configured_multicast_group() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let route = TsUdpRoute::with_socket("cam-ts", socket).with_multicast_group("239.1.2.3");
        let err = bind(&route)
            .await
            .expect_err("a pre-bound socket cannot join a multicast group");
        assert!(
            matches!(
                err,
                crate::MultimuxError::ConfigInvalid { field, .. }
                    if field == "routes.input.multicast_group"
            ),
            "got {err:?}"
        );
    }

    /// A poisoned pre-bound lock must NOT panic [`bind`] (production code has
    /// no `.expect`): it recovers the inner value and binds normally. Reverting
    /// `bind`'s match to `.expect("ts-udp prebound lock")` makes the poisoning
    /// thread's drop panic propagate and this test fail.
    #[tokio::test]
    async fn bind_recovers_from_a_poisoned_prebound_lock() {
        let route = TsUdpRoute::new("cam-ts", "127.0.0.1:0", None);
        // Poison the lock: hold it across a panic.
        let prebound = std::sync::Arc::clone(&route.prebound);
        let _ = std::thread::spawn(move || {
            let _guard = prebound.lock().expect("lock");
            panic!("poison the prebound lock");
        })
        .join();

        // `bind` must recover and bind the configured (ephemeral) addr, not
        // panic on the poisoned lock.
        let socket = bind(&route).await.expect("bind must not panic");
        assert!(socket.local_addr().is_ok());
    }
}
