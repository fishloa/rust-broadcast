//! Ingest sources feeding the segmentation pipeline. `RtspSource` (RTSP
//! pull), `RtpUdpSource` (raw RTP over UDP, uni/multicast), `TsUdpSource`
//! (MPEG-2 TS over UDP, uni/multicast), `ts_http::TsHttpSource` (MPEG-2 TS
//! over HTTP), and `hls_pull::HlsPullSource` (pull a remote (LL-)HLS origin)
//! all implement the `Source` marker trait plus the `pipeline::SampleSource`
//! contract (see `crate::pipeline`), keeping ingest swappable (and letting
//! tests drive a mock). `http_auth` is shared auth glue for the two HTTP-
//! based sources (issue #663 P3c).

pub mod hls_pull;
pub mod http_auth;
pub mod rtp_udp;
pub mod rtsp;
pub mod sdp;
pub mod ts_http;
pub mod ts_udp;
pub(crate) mod udp;

use std::time::Duration;

use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;

/// Default bound on how long a source's `connect()` waits for the ingest
/// handshake to complete (TCP/TLS connect, plus any protocol handshake —
/// RTSP DESCRIBE/SETUP/PLAY, or waiting for the first PMT/init segment) —
/// issue #663 P5 (audit-ingest #3): a stalled/half-open server (accepts the
/// TCP connection but never replies) must not hang `connect()` forever,
/// starving [`crate::origin::supervisor::supervise`]'s backoff of a chance
/// to retry.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default bound on how long a source's per-read step (one RTSP interleaved
/// frame, one HTTP body chunk, one UDP datagram, one HLS-pull client output)
/// waits before the read is treated as a stall — issue #663 P5 (audit-ingest
/// #3): the supervisor already reconnects on an `Err`, but only if one is
/// ever produced; without a read timeout a source that goes silent (wedged
/// server, dropped multicast feed) never signals anything and the route
/// silently stops advancing forever. Generous relative to any real source's
/// normal packet cadence (even a low-bitrate stream sends *something* well
/// within 30 s) while still bounding a genuinely dead connection.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Ingest connect/read timeout bounds (issue #663 P5, audit-ingest #3),
/// shared by every source kind so [`crate::config::Config`] only needs two
/// process-wide knobs rather than one pair per input type — mirrors
/// [`crate::origin::HttpLimits`]'s "one config-surfaced struct, sane
/// [`Default`], per-source `with_timeouts` builder" shape.
///
/// A source's `connect()` wraps its whole connect handshake in
/// [`Self::connect`]; its `next_samples()`/read loop wraps each individual
/// read in [`Self::read`]. Either expiring surfaces as a
/// [`crate::error::MultimuxError`], which
/// [`crate::origin::supervisor::supervise`] treats exactly like any other
/// ingest error — log, mark the route reconnecting, retry with backoff —
/// never a silent hang.
#[derive(Debug, Clone, Copy)]
pub struct IngestTimeouts {
    /// Bound on the whole connect handshake.
    pub connect: Duration,
    /// Bound on a single read/receive step once connected.
    pub read: Duration,
}

impl Default for IngestTimeouts {
    fn default() -> Self {
        IngestTimeouts {
            connect: DEFAULT_CONNECT_TIMEOUT,
            read: DEFAULT_READ_TIMEOUT,
        }
    }
}

impl From<&crate::config::Config> for IngestTimeouts {
    fn from(cfg: &crate::config::Config) -> Self {
        IngestTimeouts {
            connect: Duration::from_secs_f64(cfg.ingest_connect_timeout_secs),
            read: Duration::from_secs_f64(cfg.ingest_read_timeout_secs),
        }
    }
}

/// Per-track init derived from an SDP (RTSP's DESCRIBE body, or the
/// out-of-band SDP configured for [`rtp_udp::RtpUdpSource`]).
#[derive(Debug, Clone)]
pub struct TrackInit {
    /// 1-based track id used across the segmenter + playlist URIs.
    pub track_id: u32,
    /// Payload kind (H.264 / AAC).
    pub kind: RtpMediaKind,
    /// Codec config built from the SDP fmtp.
    pub config: CodecConfig,
    /// RTP clock rate (Hz) = IR timescale.
    pub clock_rate: u32,
    /// Per-media `a=control` URL suffix for SETUP (RTSP only; unused by
    /// [`rtp_udp::RtpUdpSource`], which has no control plane).
    pub control: Option<String>,
    /// Interleaved RTP channel assigned to this media (RTCP = channel + 1).
    /// RTSP-only framing; unused by [`rtp_udp::RtpUdpSource`].
    pub channel: u8,
    /// The media's declared RTP payload type (`m=<kind> <port> <proto>
    /// <fmt>`, RFC 4566 §5.14) — the only signal a raw RTP/UDP source has to
    /// route an incoming packet to its track (there is no interleaved
    /// channel framing outside RTSP). RTSP ignores this field today (it
    /// routes by interleaved channel instead) but it is populated
    /// identically for both ingest paths since both go through the same
    /// [`sdp::parse_sdp_tracks`].
    pub payload_type: u8,
}

/// An ingest source that can be identified by name (e.g. for logging/metrics).
///
/// Kept minimal here; Task 5's `RtspSource` extends the ingest surface with
/// the actual RTSP session driving.
pub trait Source {
    /// Human-readable stream name (e.g. the RTSP URL or config-file key).
    fn stream_name(&self) -> &str;
}
