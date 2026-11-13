//! Raw RTP-over-UDP ingest source (issue #663 P3a) — no RTSP control plane.
//!
//! The stream's codec/fmtp is described by an SDP supplied out-of-band (inline config text, or an
//! `@path` file reference — see [`crate::source::sdp::load_sdp`]). The SDP goes to a relay-mode
//! [`broadcast_client::Session`] (`UdpConfig::sdp`) through the shared session host
//! (`crate::source::client_host`): the session reads it with `transmux::sdp_track`, routes each
//! packet to its track by RTP payload type (RFC 3550 §5.1) and depayloads it
//! (RFC 6184 H.264 / RFC 3640 AAC); the host binds the socket (with an optional multicast join)
//! and feeds it datagrams.

use std::convert::Infallible;

use broadcast_client::{Config, SourceKind};
use tokio::net::UdpSocket;

use crate::error::MultimuxError;
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep, UdpHost};
use crate::source::sdp::load_sdp;
use crate::source::{IngestTimeouts, Source};

/// A raw RTP-over-UDP route: no control plane — a bind address and an
/// out-of-band SDP describing the codec/fmtp. Replaces the old (pre-5a)
/// `RtpUdpSource`; [`run_rtp_udp`] is the new drive loop.
#[derive(Clone)]
pub struct RtpUdpRoute {
    name: String,
    addr: String,
    sdp: String,
    multicast_group: Option<String>,
    /// Socket options for the bind (SP1.6); the default reproduces the
    /// pre-SP1.6 bind.
    socket: crate::source::udp::UdpBindOptions,
    timeouts: IngestTimeouts,
    /// A socket bound by the caller: used once by the first attempt, so a test can own the exact
    /// port without a reserve-then-rebind race. `None` (the production shape) binds `addr`.
    prebound: std::sync::Arc<std::sync::Mutex<Option<UdpSocket>>>,
}

impl std::fmt::Debug for RtpUdpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtpUdpRoute")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("sdp_len", &self.sdp.len())
            .field("multicast_group", &self.multicast_group)
            .field("socket", &self.socket)
            .finish()
    }
}

impl RtpUdpRoute {
    /// Build a route descriptor. `sdp` is either an inline SDP body or an
    /// `@path` reference to a file containing one (see
    /// [`crate::source::sdp::load_sdp`]).
    pub fn new(
        name: impl Into<String>,
        addr: impl Into<String>,
        sdp: impl Into<String>,
        multicast_group: Option<String>,
    ) -> Self {
        RtpUdpRoute {
            name: name.into(),
            addr: addr.into(),
            sdp: sdp.into(),
            multicast_group,
            socket: crate::source::udp::UdpBindOptions::default(),
            timeouts: IngestTimeouts::default(),
            prebound: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Builds a route over an already-bound UDP socket (the mirror of
    /// `TsUdpRoute::with_socket`). The socket is used verbatim, so a multicast group cannot be
    /// joined on it: [`run_rtp_udp`] fails if one is set.
    pub fn with_socket(name: impl Into<String>, sdp: impl Into<String>, socket: UdpSocket) -> Self {
        let addr = socket
            .local_addr()
            .map_or_else(|_| "127.0.0.1:0".to_string(), |a| a.to_string());
        RtpUdpRoute {
            name: name.into(),
            addr,
            sdp: sdp.into(),
            multicast_group: None,
            socket: crate::source::udp::UdpBindOptions::default(),
            timeouts: IngestTimeouts::default(),
            prebound: std::sync::Arc::new(std::sync::Mutex::new(Some(socket))),
        }
    }

    /// Joins `group` on the route's own bind.
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

impl Source for RtpUdpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

impl RtpUdpRoute {
    fn session_config(&self) -> Result<Config, MultimuxError> {
        let sdp = load_sdp(&self.sdp)?;
        let mut config = Config::relay();
        config.kind = SourceKind::Rtp;
        config.max_queued_bytes = crate::source::LIVE_EVENT_QUEUE_BYTES;
        config.udp.sdp = Some(String::from_utf8(sdp).map_err(|e| MultimuxError::Sdp {
            reason: format!("parse: {e}"),
        })?);
        crate::source::udp::apply_socket_options(
            &mut config,
            self.multicast_group.as_deref(),
            &self.socket,
        )?;
        Ok(config)
    }
}

/// Opens `route`'s socket through a relay-mode [`broadcast_client::Session`] carrying the route's
/// SDP and drives it until a read stall (bounded by [`IngestTimeouts::read`]). Returns the error
/// that ended the loop; the caller (the route supervisor) reconnects on it.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 — see
/// `rtsp::run_rtsp`'s own doc for what `crate::source::report_driver_progress` does with it each
/// iteration.
pub async fn run_rtp_udp(
    route: &RtpUdpRoute,
    trunk_config: media_plane::trunk::TrunkConfig,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> MultimuxError {
    let prebound = crate::source::udp::take_prebound(&route.prebound);
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
    let (url, config) = match (
        crate::source::udp::session_url("rtp", &route.addr, route.multicast_group.as_deref()),
        route.session_config(),
    ) {
        (Ok(url), Ok(config)) => (url, config),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let step = SessionStep::open(
        &url,
        config,
        None,
        SessionLabels::new("rtp/udp", route.timeouts),
        std::sync::Arc::clone(route_handle),
        false,
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
    let mut progress = crate::source::DriverProgress::new();
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
    use crate::source::ts_program::test_support::{handshake, trunk_config};
    use media_plane::trunk::SampleCursorItem;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    const TRUNK_WAIT: Duration = Duration::from_secs(10);

    fn sdp_body() -> String {
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 127.0.0.1\r\n\
             s=-\r\n\
             t=0 0\r\n\
             m=video 0 RTP/AVP 96\r\n\
             a=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 packetization-mode=1;sprop-parameter-sets={SPROP}\r\n"
        )
    }

    fn h264_rtp_packet(seq: u16, timestamp: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        const PT_H264_DYNAMIC: u8 = 96;
        const SSRC: u32 = 0xCAFE_BABE;
        let wire = rtp_packet::RtpPacket {
            marker,
            payload_type: PT_H264_DYNAMIC,
            sequence_number: seq,
            timestamp,
            ssrc: SSRC,
            csrc: Vec::new(),
            extension: None,
            padding: None,
            payload: nal,
        };
        let mut pkt = vec![0u8; broadcast_common::Serialize::serialized_len(&wire)];
        broadcast_common::Serialize::serialize_into(&wire, &mut pkt).expect("serialize RTP");
        pkt
    }

    fn route_handle() -> std::sync::Arc<crate::route::RouteHandle> {
        std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4))
    }

    /// Runs `route` until `want` samples are in its Trunk (or the senders finish), then cancels.
    async fn run_until(
        route: RtpUdpRoute,
        addr: std::net::SocketAddr,
        datagrams: Vec<Vec<u8>>,
        want: usize,
    ) -> (usize, MultimuxError) {
        let handle = route_handle();
        let cancel = CancellationToken::new();
        let run = tokio::spawn({
            let handle = std::sync::Arc::clone(&handle);
            let cancel = cancel.clone();
            async move { run_rtp_udp(&route, trunk_config(), handshake(), &handle, cancel).await }
        });
        let trunk = tokio::time::timeout(TRUNK_WAIT, handle.await_first_trunk())
            .await
            .expect("the SDP-declared programme is announced before any packet");
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");
        let send = tokio::spawn(async move {
            for _ in 0..200 {
                for d in &datagrams {
                    sender.send_to(d, addr).await.expect("send RTP packet");
                }
                tokio::task::yield_now().await;
            }
        });
        let mut cursor = trunk.subscribe_from_backlog();
        let mut seen = 0usize;
        while seen < want && !send.is_finished() {
            match cursor.poll() {
                Some(SampleCursorItem::Timed { .. }) => seen += 1,
                Some(_) => {}
                None => tokio::task::yield_now().await,
            }
        }
        cancel.cancel();
        send.abort();
        let ended = tokio::time::timeout(TRUNK_WAIT, run)
            .await
            .expect("a cancelled run must stop")
            .expect("run task");
        (seen, ended)
    }

    #[tokio::test]
    async fn loopback_udp_rtp_samples_land_in_trunk() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("local addr");
        let route = RtpUdpRoute::with_socket("cam-udp", sdp_body(), socket);
        let idr = [0x65u8, 0xAA, 0xBB];
        let non1 = [0x41u8, 0xAA, 0xBB];
        let non2 = [0x41u8, 0xCC, 0xDD];
        let aus: [(u32, &[u8]); 3] = [(1000, &idr), (4000, &non1), (7000, &non2)];
        let datagrams = aus
            .into_iter()
            .enumerate()
            .map(|(i, (ts, nal))| h264_rtp_packet(1 + i as u16, ts, true, nal))
            .collect();
        let (seen, ended) = run_until(route, addr, datagrams, 2).await;
        assert!(seen >= 2, "expected depayloaded samples in the Trunk");
        assert!(matches!(ended, MultimuxError::Connect { .. }), "{ended:?}");
    }

    #[tokio::test]
    async fn a_malformed_datagram_does_not_stop_the_stream() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("local addr");
        let route = RtpUdpRoute::with_socket("cam-udp", sdp_body(), socket);
        let mut malformed = h264_rtp_packet(99, 4000, true, &[0x41, 0xBB]);
        malformed[0] = (malformed[0] & 0x3F) | 0x40;
        let datagrams = vec![
            h264_rtp_packet(1, 1000, true, &[0x65, 0xAA]),
            malformed,
            h264_rtp_packet(2, 4000, true, &[0x41, 0xBB]),
            h264_rtp_packet(3, 7000, true, &[0x41, 0xCC]),
        ];
        let (seen, _) = run_until(route, addr, datagrams, 2).await;
        assert!(
            seen >= 2,
            "packets after the malformed datagram must still be depayloaded and delivered"
        );
    }

    #[tokio::test]
    async fn a_source_that_never_sends_fails_at_the_read_timeout() {
        const READ_TIMEOUT: Duration = Duration::from_millis(300);
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let route =
            RtpUdpRoute::with_socket("cam-udp", sdp_body(), socket).with_timeouts(IngestTimeouts {
                connect: Duration::from_secs(5),
                read: READ_TIMEOUT,
            });
        let error = tokio::time::timeout(
            Duration::from_secs(10),
            run_rtp_udp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle(),
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
    async fn a_bad_sdp_is_an_invalid_route_config() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let route = RtpUdpRoute::with_socket("cam-udp", "not an sdp", socket);
        let error = run_rtp_udp(
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
                MultimuxError::ConfigInvalid { field, .. } if field == "udp.sdp"
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn with_socket_rejects_a_configured_multicast_group() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let route = RtpUdpRoute::with_socket("cam-udp", sdp_body(), socket)
            .with_multicast_group("239.1.2.3");
        let error = run_rtp_udp(
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
}
