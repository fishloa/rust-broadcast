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
    timeouts: IngestTimeouts,
}

impl std::fmt::Debug for TsUdpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsUdpRoute")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("multicast_group", &self.multicast_group)
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
            timeouts: IngestTimeouts::default(),
        }
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
pub async fn bind(route: &TsUdpRoute) -> Result<UdpSocket> {
    bind_udp(&route.addr, route.multicast_group.as_deref()).await
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
) -> MultimuxError {
    let socket = match bind(route).await {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut dialer = TsUdpDialer;
    let session = dialer
        .dial()
        .unwrap_or_else(|never: Infallible| match never {});
    let mut driver = media_plane::ingress::IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let mut buf = vec![0u8; MAX_TS_READ];
    let read_timeout = route.timeouts.read;
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();
    loop {
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        let n = match recv_and_feed(&socket, &mut buf, &mut driver, read_timeout, now).await {
            Ok(n) => n,
            Err(e) => return e,
        };
        // EIT p/f tracking (issue #903) — a no-op unless some program on
        // this route has DVR enabled with `dvb_service_id` set.
        route_handle.feed_si_ts(&buf[..n]);
        crate::source::advance_route(&driver, route_handle, &mut progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use media_plane::ingress::{HealthState, IngestDriver, ProgramId};

    #[tokio::test]
    async fn loopback_udp_established_and_samples_land_in_trunk() {
        let reserved = UdpSocket::bind("127.0.0.1:0").await.expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let route = TsUdpRoute::new("cam-ts", addr.to_string(), None);
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
}
