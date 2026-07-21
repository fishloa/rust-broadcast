//! Raw RTP-over-UDP ingest source (issue #663 P3a) — no RTSP control plane.
//!
//! The stream's codec/fmtp is described by an SDP supplied out-of-band
//! (inline config text, or an `@path` file reference — see
//! [`crate::source::sdp::load_sdp`]); this module owns only the UDP socket
//! transport (bind + optional multicast join, via the crate-private
//! `crate::source::udp::bind_udp` helper). The SDP parse itself is the *same*
//! [`crate::source::sdp::parse_sdp_tracks`] helper
//! [`crate::source::rtsp::RtspSource`] uses for its DESCRIBE body — there is
//! no parallel SDP implementation between the two ingest paths — and RTP
//! depayload is [`transmux::RtpStreamDepacketiser`] (RFC 6184 H.264 / RFC
//! 3640 AAC), exactly as the RTSP source uses it.
//!
//! # Track routing
//!
//! RTSP distinguishes tracks by interleaved TCP channel (out-of-band framing
//! SETUP negotiates). A raw RTP/UDP source has no such framing — every
//! packet just arrives on the bound socket — so tracks are instead
//! distinguished by the RTP header's payload-type (PT) field (RFC 3550
//! §5.1), matched back to the payload type each SDP media declared (`m=<kind>
//! <port> RTP/AVP <pt>`, captured as [`crate::source::TrackInit::payload_type`]).
//! A packet whose PT does not match any configured track is silently dropped,
//! mirroring [`crate::source::rtsp::RtspSession::next_samples`]'s "unrouted
//! channel -> ignored" handling.

use std::collections::HashMap;
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};
use crate::source::sdp::{load_sdp, parse_sdp_tracks};
use crate::source::udp::bind_udp;
use crate::source::{IngestTimeouts, Source, TrackInit};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{RtpStreamDepacketiser, RtpStreamTrack};

/// Max UDP datagram this source reads in one `recv` — comfortably above the
/// largest legal UDP payload (65 507 bytes over IPv4), so a single `recv`
/// always captures a whole datagram (RFC 768: UDP delivers a datagram
/// atomically or not at all — never a partial read).
const MAX_UDP_DATAGRAM: usize = 65_536;

/// RFC 3550 §5.1 fixed RTP header length (before any CSRC/extension) — the
/// minimum a packet must carry to have a payload-type field at all.
const RTP_MIN_HEADER_LEN: usize = 12;

/// Mask for the 7-bit payload-type field (RTP header byte 1, bit 7 is the
/// marker bit).
const RTP_PT_MASK: u8 = 0x7F;

/// A raw RTP-over-UDP stream to pull: no control plane (DESCRIBE/SETUP/PLAY)
/// — just a bind address and an out-of-band SDP describing the codec/fmtp.
#[derive(Clone)]
pub struct RtpUdpSource {
    name: String,
    addr: String,
    sdp: String,
    multicast_group: Option<String>,
    timeouts: IngestTimeouts,
}

/// Manual `Debug`: the configured SDP may be sizeable and carries no secret,
/// but is not useful verbatim in a log line — only its length is shown
/// (mirrors the tidiness intent of [`crate::source::rtsp::RtspSource`]'s
/// redacted `Debug`, even though nothing here is actually a credential).
impl std::fmt::Debug for RtpUdpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtpUdpSource")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("sdp_len", &self.sdp.len())
            .field("multicast_group", &self.multicast_group)
            .finish()
    }
}

impl RtpUdpSource {
    /// Build a source descriptor. `sdp` is either an inline SDP body or an
    /// `@path` reference to a file containing one (see
    /// [`crate::source::sdp::load_sdp`]), read fresh on every [`connect`](Self::connect)
    /// so an on-disk SDP can be updated between reconnects.
    pub fn new(
        name: impl Into<String>,
        addr: impl Into<String>,
        sdp: impl Into<String>,
        multicast_group: Option<String>,
    ) -> Self {
        RtpUdpSource {
            name: name.into(),
            addr: addr.into(),
            sdp: sdp.into(),
            multicast_group,
            timeouts: IngestTimeouts::default(),
        }
    }

    /// Overrides the default [`IngestTimeouts`] — see `RtspSource::with_timeouts`
    /// for the pattern this mirrors.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Binds the UDP socket (joining `multicast_group` if configured) and
    /// parses the configured SDP into per-track init — the raw-RTP analogue
    /// of [`crate::source::rtsp::RtspSource::connect`]'s DESCRIBE step, just
    /// with the SDP supplied out-of-band instead of fetched from the source.
    // TODO(P5.3): RTCP SR wallclock A/V sync — this source binds only the RTP
    // port; the RTCP companion port (conventionally RTP port + 1, RFC 3550
    // §11) is never bound, so no Sender Report ever reaches this crate for
    // raw RTP/UDP ingest. Wiring it would mean binding a second socket here,
    // racing it against the RTP socket's `recv` in
    // `RtpUdpSession::next_samples`, and feeding the resulting NTP/RTP
    // mapping into the `Track`/`Sample` timing model — see the identical
    // note on `crate::source::rtsp::route_channel` (the RTSP-interleaved
    // case of the same gap); deferred as a large, multi-crate lift (issue
    // #663 P5).
    pub async fn connect(&self) -> Result<RtpUdpSession> {
        let sdp_bytes = load_sdp(&self.sdp)?;
        let tracks = parse_sdp_tracks(&sdp_bytes)?;

        let socket = bind_udp(&self.addr, self.multicast_group.as_deref()).await?;

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

        Ok(RtpUdpSession {
            tracks,
            socket,
            depacketiser,
            pt_to_track,
            buf: vec![0u8; MAX_UDP_DATAGRAM],
            read_timeout: self.timeouts.read,
        })
    }
}

impl Source for RtpUdpSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live raw RTP-over-UDP session: bound socket, depayloading whatever
/// arrives.
pub struct RtpUdpSession {
    /// Per-track init derived from the configured SDP.
    pub tracks: Vec<TrackInit>,
    socket: UdpSocket,
    depacketiser: RtpStreamDepacketiser,
    /// RTP payload-type -> track id, built from the SDP's declared payload
    /// types — see the module doc's "Track routing" section.
    pt_to_track: HashMap<u8, u32>,
    buf: Vec<u8>,
    /// Bound on each [`Self::next_samples`] read — see [`IngestTimeouts::read`].
    read_timeout: Duration,
}

impl RtpUdpSession {
    /// The `TrackSpec`s (timescale = RTP clock rate) for init-segment
    /// construction.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.depacketiser.track_specs()
    }

    /// Receives one UDP datagram (one RTP packet) and depayloads it. Returns
    /// the samples emitted (zero or more, paired with track id); a packet
    /// too short to carry an RTP header, or whose payload type does not
    /// match any configured track, is silently ignored (empty `Vec`).
    ///
    /// Never returns `Ok(None)`: UDP is connectionless, so there is no
    /// transport-level end-of-stream signal (unlike RTSP's TCP close) — a
    /// dead source is only detected by the supervisor's health/backoff, not
    /// by this method.
    ///
    /// Bounded by [`IngestTimeouts::read`] (issue #663 P5.2, audit-ingest
    /// #3): a source that stops sending datagrams (dropped multicast feed,
    /// wedged encoder) would otherwise leave this `.await` pending forever —
    /// a timed-out read surfaces as a [`MultimuxError::Connect`], reconnected
    /// by [`crate::origin::supervisor::supervise`] exactly like any other
    /// read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let read_timeout = self.read_timeout;
        let n = tokio::time::timeout(read_timeout, self.socket.recv(&mut self.buf))
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: format!("rtp/udp recv: no data within {read_timeout:?}"),
            })?
            .map_err(|e| MultimuxError::Connect {
                reason: format!("udp recv: {e}"),
            })?;
        let packet = &self.buf[..n];
        let Some(track_id) =
            payload_type_of(packet).and_then(|pt| self.pt_to_track.get(&pt).copied())
        else {
            return Ok(Some(Vec::new()));
        };
        let samples =
            self.depacketiser
                .push(track_id, packet)
                .map_err(|e| MultimuxError::Depay {
                    reason: e.to_string(),
                })?;
        Ok(Some(samples.into_iter().map(|s| (track_id, s)).collect()))
    }
}

/// Extracts the RTP payload-type field (RFC 3550 §5.1, header byte 1 bits
/// `[6:0]`) from a wire packet, or `None` if it's too short to even carry a
/// fixed RTP header.
///
/// Pure transport-layer routing (deciding which track a datagram belongs
/// to) — the packet's actual depayload (marker bit, sequence number,
/// timestamp, payload bytes, AU reassembly) is entirely
/// `transmux::RtpStreamDepacketiser`'s job; multimux reads only the one
/// field it needs to route bytes to the right track.
fn payload_type_of(packet: &[u8]) -> Option<u8> {
    if packet.len() < RTP_MIN_HEADER_LEN {
        return None;
    }
    Some(packet[1] & RTP_PT_MASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

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

    /// Builds a minimal single-NAL-unit-mode H.264 RTP packet (RFC 3550 §5.1
    /// fixed 12-byte header + one NAL unit verbatim, RFC 6184 §5.1) —
    /// mirrors `multimux/tests/rtsp_ingest.rs`'s `rtp_packet` helper.
    fn rtp_packet(seq: u16, timestamp: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        const PT_H264_DYNAMIC: u8 = 96;
        const SSRC: u32 = 0xCAFE_BABE;
        let mut pkt = Vec::with_capacity(12 + nal.len());
        pkt.push(0x80); // V=2, P=0, X=0, CC=0
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

    /// End-to-end loopback test: bind a real UDP socket via `connect()`,
    /// send synthetic RTP packets to it from a second socket, and assert
    /// real depayloaded `Sample`s come out of `next_samples()` — biting on
    /// the real socket I/O + SDP-driven payload-type routing + depayload
    /// path, not just the pure helper functions above.
    #[tokio::test]
    async fn loopback_udp_rtp_yields_depayloaded_samples() {
        let source = RtpUdpSource::new("cam-udp", "127.0.0.1:0", sdp_body(), None);
        let mut session = source.connect().await.expect("connect");

        let specs = session.track_specs();
        assert_eq!(specs.len(), 1, "one video track from the SDP");
        assert_eq!(specs[0].timescale, 90_000);

        let local_addr = session.socket.local_addr().expect("local addr");
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");

        // AU0 @1000 (IDR), AU1 @4000 (non-IDR), AU2 @7000 (non-IDR): mirrors
        // `rtsp_ingest.rs`'s timing — 2 completed samples are yielded once
        // AU2 arrives (the depacketiser needs the *next* AU's timestamp to
        // finalise a duration).
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

        let mut samples = Vec::new();
        while samples.len() < 2 {
            let batch =
                tokio::time::timeout(std::time::Duration::from_secs(5), session.next_samples())
                    .await
                    .expect("next_samples timed out")
                    .expect("next_samples")
                    .expect("udp source never signals EOF");
            samples.extend(batch);
        }

        assert_eq!(samples.len(), 2);
        let (track_id0, sample0) = &samples[0];
        assert_eq!(*track_id0, 1);
        assert!(sample0.is_sync, "first AU was the IDR");
        let (track_id1, sample1) = &samples[1];
        assert_eq!(*track_id1, 1);
        assert!(!sample1.is_sync, "second AU was non-IDR");
    }

    /// A datagram whose payload type matches no configured track (e.g. RTCP
    /// riding the same port, or a foreign stream) must be silently ignored,
    /// not routed to the lone video track or surfaced as an error.
    #[tokio::test]
    async fn unrouted_payload_type_is_ignored() {
        let source = RtpUdpSource::new("cam-udp", "127.0.0.1:0", sdp_body(), None);
        let mut session = source.connect().await.expect("connect");
        let local_addr = session.socket.local_addr().expect("local addr");
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");

        // Payload type 97 was never declared in the SDP (only 96 was).
        let mut pkt = rtp_packet(1, 1000, true, &[0x65]);
        pkt[1] = 97;
        sender.send_to(&pkt, local_addr).await.expect("send");

        let batch = tokio::time::timeout(std::time::Duration::from_secs(5), session.next_samples())
            .await
            .expect("next_samples timed out")
            .expect("next_samples")
            .expect("udp source never signals EOF");
        assert!(
            batch.is_empty(),
            "an unrouted payload type must yield no samples"
        );
    }

    /// A source that never sends any RTP packet (dead multicast feed, wedged
    /// camera) must not hang `next_samples()` forever (issue #663 P5.2,
    /// audit-ingest #3): with a short configured [`IngestTimeouts::read`],
    /// the call must return an `Err` within that bound, not block
    /// indefinitely.
    #[tokio::test]
    async fn next_samples_times_out_when_source_is_silent() {
        const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);
        let source = RtpUdpSource::new("cam-udp", "127.0.0.1:0", sdp_body(), None).with_timeouts(
            crate::source::IngestTimeouts {
                connect: std::time::Duration::from_secs(5),
                read: READ_TIMEOUT,
            },
        );
        let mut session = source.connect().await.expect("connect");

        // Nothing is ever sent to this socket: the read must time out (as an
        // `Err`, not a hang) within a small bounded multiple of
        // `READ_TIMEOUT` — never left pending forever.
        let outcome = tokio::time::timeout(READ_TIMEOUT * 5, session.next_samples())
            .await
            .expect(
                "next_samples must return within a bounded multiple of the read timeout, not hang",
            );
        assert!(
            outcome.is_err(),
            "expected a recoverable read-timeout error for a silent source"
        );
    }
}
