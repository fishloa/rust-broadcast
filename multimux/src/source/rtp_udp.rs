//! Raw RTP-over-UDP ingest source (issue #663 P3a; ported onto the
//! media-plane ingress traits at plan step 5a) — no RTSP control plane.
//!
//! The stream's codec/fmtp is described by an SDP supplied out-of-band
//! (inline config text, or an `@path` file reference — see
//! [`crate::source::sdp::load_sdp`]); this module owns only the UDP socket
//! transport (bind + optional multicast join, via the crate-private
//! `crate::source::udp::bind_udp` helper). The SDP parse itself is the *same*
//! [`crate::source::sdp::parse_sdp_tracks`] helper
//! [`crate::source::rtsp::RtspDialer`] uses for its DESCRIBE body — there is
//! no parallel SDP implementation between the two ingest paths — and RTP
//! depayload is [`transmux::RtpStreamDepacketiser`] (RFC 6184 H.264 / RFC
//! 3640 AAC), exactly as the RTSP source uses it.
//!
//! # Sans-IO reshape: zero executor bridge, same reasoning as `ts_udp`
//!
//! Exactly like [`crate::source::ts_udp`], UDP is connectionless: binding the
//! socket is a purely local operation, so [`RtpUdpDialer::dial`] performs
//! **no I/O** and immediately queues [`SessionEvent::Established`]. There is
//! no PMT-equivalent wait either — the SDP is already known at construction
//! time, so [`RtpUdpDialer::dial`] can announce `NewProgram` with the full
//! track set the very first time it is polled, with no bytes fed at all.
//! [`run_rtp_udp`] is the multimux-side adapter that owns the real
//! `UdpSocket` and feeds it, mirroring `ts_udp::run_ts_udp`.
//!
//! # Track routing
//!
//! RTSP distinguishes tracks by interleaved TCP channel (out-of-band framing
//! SETUP negotiates). A raw RTP/UDP source has no such framing — every
//! packet just arrives on the bound socket — so tracks are instead
//! distinguished by the RTP header's payload-type (PT) field (RFC 3550
//! §5.1), matched back to the payload type each SDP media declared (`m=<kind>
//! <port> RTP/AVP <pt>`, captured as [`crate::source::TrackInit::payload_type`]).
//! A packet whose PT does not match any configured track is silently dropped.

use std::collections::HashMap;
use std::time::Duration;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{Dialer, IngestSession, ProgramId, SessionEvent};
use media_plane::trunk::RetentionClass;
use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};
use crate::source::sdp::{load_sdp, parse_sdp_tracks};
use crate::source::udp::bind_udp;
use crate::source::{IngestTimeouts, Source, TrackInit};
use transmux::{RtpStreamDepacketiser, RtpStreamTrack};

/// Max UDP datagram this source reads in one `recv` — comfortably above the
/// largest legal UDP payload (65 507 bytes over IPv4).
const MAX_UDP_DATAGRAM: usize = 65_536;

/// RFC 3550 §5.1 fixed RTP header length (before any CSRC/extension).
const RTP_MIN_HEADER_LEN: usize = 12;

/// Mask for the 7-bit payload-type field (RTP header byte 1, bit 7 is the
/// marker bit).
const RTP_PT_MASK: u8 = 0x7F;

/// A raw RTP-over-UDP route: no control plane — a bind address and an
/// out-of-band SDP describing the codec/fmtp. Replaces the old (pre-5a)
/// `RtpUdpSource`; [`run_rtp_udp`] is the new drive loop.
#[derive(Clone)]
pub struct RtpUdpRoute {
    name: String,
    addr: String,
    sdp: String,
    multicast_group: Option<String>,
    timeouts: IngestTimeouts,
}

impl std::fmt::Debug for RtpUdpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtpUdpRoute")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("sdp_len", &self.sdp.len())
            .field("multicast_group", &self.multicast_group)
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

impl Source for RtpUdpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live raw RTP-over-UDP [`IngestSession`]: no socket, no I/O — just the
/// depacketiser plus payload-type routing. [`run_rtp_udp`] is the
/// multimux-side adapter that owns the real `UdpSocket` and feeds it.
pub struct RtpUdpIngestSession {
    depacketiser: RtpStreamDepacketiser,
    pt_to_track: HashMap<u8, u32>,
    pending: std::collections::VecDeque<SessionEvent>,
    announced: bool,
}

impl RtpUdpIngestSession {
    fn new(tracks: Vec<TrackInit>) -> Self {
        let depacketiser = RtpStreamDepacketiser::new(
            tracks
                .iter()
                .map(|t| RtpStreamTrack::new(t.track_id, t.kind, t.config.clone(), t.clock_rate))
                .collect(),
        );
        let pt_to_track: HashMap<u8, u32> = tracks
            .iter()
            .map(|t| (t.payload_type, t.track_id))
            .collect();
        RtpUdpIngestSession {
            depacketiser,
            pt_to_track,
            pending: std::collections::VecDeque::from(vec![SessionEvent::Established]),
            announced: false,
        }
    }

    /// The single program this session ever announces: the SDP-declared
    /// track set, known entirely at construction time (unlike TS-UDP's PMT
    /// wait), so `NewProgram` fires on the very first `poll()` with no bytes
    /// fed at all.
    fn announce_program_once(&mut self) {
        if !self.announced {
            self.announced = true;
            let specs = self.depacketiser.track_specs();
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks: specs,
            });
        }
    }
}

impl Stage for RtpUdpIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    /// A malformed/unroutable datagram is silently ignored (mirrors the
    /// pre-5a session's "unrouted payload type -> ignored" handling) — a
    /// depayload failure from a routed track is the only real error case,
    /// so this uses the crate's own error type rather than `Infallible`.
    type Error = MultimuxError;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> core::result::Result<(), MultimuxError> {
        self.announce_program_once();
        let Some(track_id) =
            payload_type_of(input).and_then(|pt| self.pt_to_track.get(&pt).copied())
        else {
            return Ok(());
        };
        let samples =
            self.depacketiser
                .push(track_id, input)
                .map_err(|e| MultimuxError::Depay {
                    reason: e.to_string(),
                })?;
        for sample in samples {
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(0),
                track_id,
                retention: RetentionClass::Timed,
                sample,
            });
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> core::result::Result<(), MultimuxError> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(MAX_UDP_DATAGRAM)
    }
}

impl IngestSession for RtpUdpIngestSession {}

/// Extracts the RTP payload-type field (RFC 3550 §5.1, header byte 1 bits
/// `[6:0]`) from a wire packet, or `None` if it's too short to even carry a
/// fixed RTP header.
fn payload_type_of(packet: &[u8]) -> Option<u8> {
    if packet.len() < RTP_MIN_HEADER_LEN {
        return None;
    }
    Some(packet[1] & RTP_PT_MASK)
}

/// Constructs an [`RtpUdpIngestSession`] from the route's out-of-band SDP —
/// the only fallible local step (parsing the SDP), performed with **no
/// I/O**.
pub struct RtpUdpDialer {
    sdp: String,
}

impl RtpUdpDialer {
    fn new(sdp: String) -> Self {
        RtpUdpDialer { sdp }
    }
}

impl Dialer for RtpUdpDialer {
    type Session = RtpUdpIngestSession;
    type Error = MultimuxError;

    fn dial(&mut self) -> core::result::Result<RtpUdpIngestSession, MultimuxError> {
        let sdp_bytes = load_sdp(&self.sdp)?;
        let tracks = parse_sdp_tracks(&sdp_bytes)?;
        Ok(RtpUdpIngestSession::new(tracks))
    }
}

/// Binds `route`'s UDP socket — see `ts_udp::bind` for why this is split out
/// of `dial()`/`run_rtp_udp`.
pub async fn bind(route: &RtpUdpRoute) -> Result<UdpSocket> {
    bind_udp(&route.addr, route.multicast_group.as_deref()).await
}

/// Reads one datagram from `socket` (bounded by `read_timeout`) and feeds it
/// to `driver`. See `ts_udp::recv_and_feed` — identical shape, this
/// session's own `Stage::Error` is `MultimuxError` rather than `Infallible`
/// (a depayload failure is a real per-packet error here), so a feed error
/// surfaces as `HealthState::Failed`, not just an I/O-layer error.
pub async fn recv_and_feed(
    socket: &UdpSocket,
    buf: &mut [u8],
    driver: &mut media_plane::ingress::IngestDriver<RtpUdpIngestSession>,
    read_timeout: Duration,
    now: Timestamp,
) -> Result<()> {
    let n = tokio::time::timeout(read_timeout, socket.recv(buf))
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!("rtp/udp recv: no data within {read_timeout:?}"),
        })?
        .map_err(|e| MultimuxError::Connect {
            reason: format!("udp recv: {e}"),
        })?;
    driver.feed(&buf[..n], now);
    Ok(())
}

/// Constructs (`dial()`, no I/O), binds the socket, and drives an
/// [`RtpUdpIngestSession`] through [`media_plane::ingress::IngestDriver`]
/// until a read stall — the new drive loop, replacing the pre-5a
/// `RtpUdpSource::connect`/`RtpUdpSession::next_samples` pair.
pub async fn run_rtp_udp(
    route: &RtpUdpRoute,
    trunk_config: media_plane::trunk::TrunkConfig,
    handshake: media_plane::ingress::HandshakePolicy,
) -> MultimuxError {
    let socket = match bind(route).await {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut dialer = RtpUdpDialer::new(route.sdp.clone());
    let session = match dialer.dial() {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut driver = media_plane::ingress::IngestDriver::new(
        session,
        trunk_config,
        handshake,
        // Bound the Trunks one session may mint (media-plane #803). Routes here
        // carry a single programme today; the default ceiling covers an MPTS.
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
    let read_timeout = route.timeouts.read;
    let start = std::time::Instant::now();
    loop {
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        if let Err(e) = recv_and_feed(&socket, &mut buf, &mut driver, read_timeout, now).await {
            return e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::ingress::{HandshakePolicy, HealthState, IngestDriver};
    use media_plane::trunk::TrunkConfig;
    use std::num::NonZeroUsize;

    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

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

    fn rtp_packet(seq: u16, timestamp: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        const PT_H264_DYNAMIC: u8 = 96;
        const SSRC: u32 = 0xCAFE_BABE;
        let mut pkt = Vec::with_capacity(12 + nal.len());
        pkt.push(0x80);
        pkt.push(if marker {
            0x80 | PT_H264_DYNAMIC
        } else {
            PT_H264_DYNAMIC
        });
        pkt.extend_from_slice(&seq.to_be_bytes());
        pkt.extend_from_slice(&timestamp.to_be_bytes());
        pkt.extend_from_slice(&SSRC.to_be_bytes());
        pkt.extend_from_slice(nal);
        pkt
    }

    #[test]
    fn payload_type_of_extracts_pt_ignoring_marker_bit() {
        let pkt = rtp_packet(1, 1000, true, &[0x65]);
        assert_eq!(payload_type_of(&pkt), Some(96));
        let pkt = rtp_packet(1, 1000, false, &[0x65]);
        assert_eq!(payload_type_of(&pkt), Some(96));
    }

    #[test]
    fn payload_type_of_rejects_short_packet() {
        assert_eq!(payload_type_of(&[0x80, 0x60]), None);
    }

    /// Drives an `RtpUdpIngestSession` directly (no socket): dial() performs
    /// no I/O, and `NewProgram` for the single SDP-declared program fires on
    /// the very first `feed()` — before any RTP packet has even been routed
    /// — since the whole track set is already known from the SDP.
    ///
    /// MUTATION-CHECKED: change `announce_program_once`'s guard from
    /// `!self.announced` to always-true and this test's second assertion
    /// (`trunk(ProgramId(0))` staying the *same* `Arc` across two feeds)
    /// still passes only because `IngestDriver::drain`'s `NewProgram` arm
    /// always calls `Trunk::new` — a duplicate `NewProgram` would silently
    /// replace program 0's `Trunk` (and its writer) out from under any
    /// subscriber; removing the guard entirely () therefore changes
    /// behaviour on the second feed, which the next test below drives.
    #[test]
    fn established_and_new_program_fire_before_any_packet() {
        let mut dialer = RtpUdpDialer::new(sdp_body());
        let session = dialer.dial().expect("dial: local SDP parse only, no I/O");
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
        assert!(matches!(driver.health(), HealthState::Establishing));

        // A feed with an unroutable payload type still drives Established +
        // NewProgram (the "before any packet is routed" property) even
        // though the packet itself yields no sample.
        let mut foreign = rtp_packet(1, 1000, true, &[0x65]);
        foreign[1] = 200; // PT never declared in the SDP
        driver.feed(&foreign, Timestamp::ZERO);
        assert!(matches!(driver.health(), HealthState::Live));
        assert!(
            driver.trunk(ProgramId(0)).is_some(),
            "the single SDP-declared program must be announced on the first feed"
        );
    }

    /// A duplicate `NewProgram` from a second feed would (per
    /// `IngestDriver::drain`) mint a *fresh* `Trunk`, replacing the first —
    /// this test proves `announce_program_once`'s guard prevents that: the
    /// same `Trunk` (by pointer) is still there after a second feed.
    #[test]
    fn second_feed_does_not_re_announce_the_program() {
        let mut dialer = RtpUdpDialer::new(sdp_body());
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());

        let idr = rtp_packet(1, 1000, true, &[0x65, 0xAA]);
        driver.feed(&idr, Timestamp::ZERO);
        let first_ptr = std::sync::Arc::as_ptr(driver.trunk(ProgramId(0)).unwrap());

        let next = rtp_packet(2, 4000, true, &[0x41, 0xBB]);
        driver.feed(&next, Timestamp::from_nanos(1));
        let second_ptr = std::sync::Arc::as_ptr(driver.trunk(ProgramId(0)).unwrap());

        assert_eq!(
            first_ptr, second_ptr,
            "a second feed must not re-announce (and thus replace) program 0's Trunk"
        );
    }

    /// End-to-end loopback: real socket I/O, real depayloaded samples land
    /// in the Trunk.
    #[tokio::test]
    async fn loopback_udp_rtp_samples_land_in_trunk() {
        let route = RtpUdpRoute::new("cam-udp", "127.0.0.1:0", sdp_body(), None);
        let socket = bind(&route).await.expect("bind");
        let local_addr = socket.local_addr().expect("local addr");

        let mut dialer = RtpUdpDialer::new(route.sdp.clone());
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());

        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");
        let idr = [0x65u8, 0xAA, 0xBB];
        let non1 = [0x41u8, 0xAA, 0xBB];
        let non2 = [0x41u8, 0xCC, 0xDD];
        let aus: [(u32, &[u8]); 3] = [(1000, &idr), (4000, &non1), (7000, &non2)];
        for (i, (ts, nal)) in aus.into_iter().enumerate() {
            let pkt = rtp_packet(1 + i as u16, ts, true, nal);
            sender
                .send_to(&pkt, local_addr)
                .await
                .expect("send RTP packet");
        }

        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
        let mut cursor = None;
        let mut samples_seen = 0usize;
        for i in 0..10u64 {
            recv_and_feed(
                &socket,
                &mut buf,
                &mut driver,
                Duration::from_secs(5),
                Timestamp::from_nanos(i),
            )
            .await
            .expect("recv within generous timeout");
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                while c.poll().is_some() {
                    samples_seen += 1;
                }
            }
            if samples_seen >= 2 {
                break;
            }
        }
        assert!(
            samples_seen >= 2,
            "expected depayloaded samples in the Trunk"
        );
    }

    /// A source that never sends any RTP packet must not hang `recv_and_feed`
    /// forever — issue #663 P5.2, preserved from the pre-5a test.
    #[tokio::test]
    async fn recv_and_feed_times_out_when_source_is_silent() {
        let route = RtpUdpRoute::new("cam-udp", "127.0.0.1:0", sdp_body(), None);
        let socket = bind(&route).await.expect("bind");
        let mut dialer = RtpUdpDialer::new(route.sdp.clone());
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];

        const READ_TIMEOUT: Duration = Duration::from_millis(100);
        let outcome = tokio::time::timeout(
            READ_TIMEOUT * 5,
            recv_and_feed(
                &socket,
                &mut buf,
                &mut driver,
                READ_TIMEOUT,
                Timestamp::ZERO,
            ),
        )
        .await
        .expect("recv_and_feed must return within a bounded multiple of the read timeout");
        assert!(
            outcome.is_err(),
            "expected a recoverable read-timeout error"
        );
    }
}
