//! MPEG-2 Transport Stream over UDP ingest source: a UDP socket (unicast or multicast) driven by
//! a [`broadcast_client::Session`] in relay mode through the shared `crate::source::client_host`.
//!
//! The session decides the socket (bind, multicast join, `SO_RCVBUF`) and the host performs it;
//! TS alignment checks, demux and the programme split are the session's and `EventIngest`'s.

use std::convert::Infallible;

use broadcast_client::{Config, SourceKind};
use tokio::net::UdpSocket;

use crate::error::MultimuxError;
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep, UdpHost};
use crate::source::{Source, Timeouts};

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
    timeouts: Timeouts,
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
            timeouts: Timeouts::default(),
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
    /// [`run_ts_udp`] fail rather than silently ignore the group.
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
            timeouts: Timeouts::default(),
            prebound: std::sync::Arc::new(std::sync::Mutex::new(Some(socket))),
        }
    }

    /// Configure the multicast group to join. Meaningful only for a route that
    /// binds its own `addr`; on a [`Self::with_socket`] route it is a
    /// contradiction (`run_ts_udp` fails rather than silently dropping
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

    /// Overrides the default [`Timeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

impl Source for TsUdpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

impl TsUdpRoute {
    fn take_prebound(&self) -> Option<UdpSocket> {
        crate::source::udp::take_prebound(&self.prebound)
    }

    fn session_url(&self) -> Result<String, MultimuxError> {
        crate::source::udp::session_url("udp", &self.addr, self.multicast_group.as_deref())
    }

    fn session_config(&self) -> Result<Config, MultimuxError> {
        let mut config = Config::relay();
        config.kind = SourceKind::Udp;
        config.max_queued_bytes = crate::source::LIVE_EVENT_QUEUE_BYTES;
        crate::source::udp::apply_socket_options(
            &mut config,
            self.multicast_group.as_deref(),
            &self.socket,
        )?;
        Ok(config)
    }
}

/// Opens `route`'s socket through a relay-mode [`broadcast_client::Session`] and drives it until a
/// read stall (bounded by [`Timeouts::read_idle`]). Returns the error that ended the loop
/// (always a read-side error); the caller (the route supervisor) reconnects on it.
///
/// A caller-supplied pre-bound socket ([`TsUdpRoute::with_socket`]) is consumed **once**: the
/// first attempt uses it, and every later attempt (a reconnect after the read loop ends) binds
/// `route.addr` itself. The pre-bound socket exists only so a *test* can own the exact port for
/// its first attempt.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 — see
/// `rtsp::run_rtsp`'s own doc for what `crate::source::report_driver_progress` does with it each
/// iteration.
pub async fn run_ts_udp(
    route: &TsUdpRoute,
    trunk_config: media_plane::trunk::TrunkConfig,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> MultimuxError {
    let prebound = route.take_prebound();
    if prebound.is_some()
        && let Some(group) = route.multicast_group.as_deref()
    {
        return MultimuxError::ConfigInvalid {
            field: "routes.input.multicast_group",
            reason: format!(
                "a pre-bound socket cannot join multicast_group {group:?}; drop the group, \
                 or let the route bind its own addr"
            ),
        };
    }
    let (url, config) = match (route.session_url(), route.session_config()) {
        (Ok(url), Ok(config)) => (url, config),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let step = SessionStep::open(
        &url,
        config,
        None,
        SessionLabels::new(WHAT, route.timeouts),
        std::sync::Arc::clone(route_handle),
        true,
    );
    let mut step = match step {
        Ok(step) => step.with_udp(UdpHost {
            prebound,
            options: route.socket.clone(),
        }),
        Err(e) => return e,
    };
    let driver = media_plane::ingress::IngestDriver::new(
        EventIngest::new(),
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut sink = tokio::io::sink();
    crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        cancel,
        route.timeouts,
        &mut sink,
        &mut step,
        |never: Infallible| match never {},
    )
    .await
}

const WHAT: &str = "ts/udp";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteConfig;
    use media_plane::trunk::SampleCursorItem;
    use media_plane::ts_session::test_support::{build_ts_bytes, handshake, trunk_config};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;
    use transmux::TrackId;

    const TRUNK_WAIT: Duration = Duration::from_secs(10);

    fn route_handle() -> std::sync::Arc<crate::route::RouteHandle> {
        std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ))
    }

    #[tokio::test]
    async fn loopback_udp_samples_land_in_trunk() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("local addr");
        let route = TsUdpRoute::with_socket("cam-ts", socket);
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 60);
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");
        let handle = route_handle();
        let cancel = CancellationToken::new();
        let run = tokio::spawn({
            let handle = std::sync::Arc::clone(&handle);
            let cancel = cancel.clone();
            async move { run_ts_udp(&route, trunk_config(), handshake(), &handle, cancel).await }
        });
        let send_task = tokio::spawn(async move {
            for _ in 0..200 {
                for chunk in ts_bytes.chunks(7 * 188) {
                    sender.send_to(chunk, addr).await.expect("send TS datagram");
                }
                tokio::task::yield_now().await;
            }
        });
        let trunk = tokio::time::timeout(TRUNK_WAIT, handle.await_first_trunk())
            .await
            .expect("the ingest must publish a trunk");
        let mut cursor = trunk.subscribe_from_backlog();
        let saw_sample = loop {
            if let Some(item) = cursor.poll() {
                if matches!(item, SampleCursorItem::Timed { .. }) {
                    break true;
                }
            } else if send_task.is_finished() {
                break false;
            } else {
                tokio::task::yield_now().await;
            }
        };
        cancel.cancel();
        send_task.abort();
        let ended = tokio::time::timeout(TRUNK_WAIT, run)
            .await
            .expect("a cancelled run must stop")
            .expect("run task");
        assert!(saw_sample, "expected at least one sample in the Trunk");
        assert!(matches!(ended, MultimuxError::Connect { .. }), "{ended:?}");
    }

    #[tokio::test]
    async fn a_source_that_goes_silent_fails_at_the_read_timeout() {
        const READ_TIMEOUT: Duration = Duration::from_secs(2);
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let route = TsUdpRoute::with_socket("cam-ts", socket).with_timeouts(
            broadcast_common::timeouts::Timeouts::default()
                .with_connect(Duration::from_secs(5))
                .with_read_idle(READ_TIMEOUT),
        );
        let handle = route_handle();
        let error = tokio::time::timeout(
            Duration::from_secs(10),
            run_ts_udp(
                &route,
                trunk_config(),
                handshake(),
                &handle,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("the stall must end the run through the configured read timeout");
        match error {
            MultimuxError::Connect { reason } => {
                assert!(reason.contains("no data within"), "{reason}");
            }
            other => panic!("expected a stall error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_socket_is_consumed_once() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("local addr");
        let route = TsUdpRoute::with_socket("cam-ts", socket);
        let first = route.take_prebound().expect("the caller's socket");
        assert_eq!(first.local_addr().expect("addr"), addr);
        assert!(route.take_prebound().is_none());
    }

    #[tokio::test]
    async fn with_socket_rejects_a_configured_multicast_group() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let route = TsUdpRoute::with_socket("cam-ts", socket).with_multicast_group("239.1.2.3");
        let error = run_ts_udp(
            &route,
            trunk_config(),
            handshake(),
            &route_handle(),
            CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(
                error,
                MultimuxError::ConfigInvalid { field, .. }
                    if field == "routes.input.multicast_group"
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn a_poisoned_prebound_lock_does_not_panic() {
        let route = TsUdpRoute::new("cam-ts", "127.0.0.1:0", None);
        let prebound = std::sync::Arc::clone(&route.prebound);
        let _ = std::thread::spawn(move || {
            let _guard = prebound.lock().expect("lock");
            panic!("poison the prebound lock");
        })
        .join();
        assert!(route.take_prebound().is_none());
    }

    #[test]
    fn the_session_url_names_the_group_or_the_bind_address() {
        let unicast = TsUdpRoute::new("a", "127.0.0.1:5000", None);
        assert_eq!(unicast.session_url().unwrap(), "udp://127.0.0.1:5000");
        let multicast = TsUdpRoute::new("a", "0.0.0.0:5000", Some("239.1.1.1".into()));
        assert_eq!(multicast.session_url().unwrap(), "udp://239.1.1.1:5000");
        let v6 = TsUdpRoute::new("a", "[::]:5000", Some("ff02::1".into()));
        assert_eq!(v6.session_url().unwrap(), "udp://[ff02::1]:5000");
    }

    #[test]
    fn the_session_config_carries_the_socket_options() {
        let options = crate::source::udp::UdpBindOptions {
            recv_buffer_bytes: Some(1 << 20),
            multicast_interface: Some("192.168.1.10".into()),
            ..Default::default()
        };
        let route = TsUdpRoute::new("a", "0.0.0.0:5000", Some("239.1.1.1".into()))
            .with_socket_options(options.clone());
        let config = route.session_config().unwrap();
        assert_eq!(config.udp.recv_buffer_bytes, Some(1 << 20));
        assert_eq!(config.udp.interface, Some("192.168.1.10".parse().unwrap()));
        let indexed = crate::source::udp::UdpBindOptions {
            multicast_interface: Some("3".into()),
            ..options
        };
        let route =
            TsUdpRoute::new("a", "[::]:5000", Some("ff02::1".into())).with_socket_options(indexed);
        assert_eq!(route.session_config().unwrap().udp.interface_index, Some(3));
    }
}
