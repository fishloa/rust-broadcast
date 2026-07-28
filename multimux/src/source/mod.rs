//! Ingest sources feeding the segmentation pipeline. `RtspSource` (RTSP
//! pull), `RtpUdpSource` (raw RTP over UDP, uni/multicast), `TsUdpSource`
//! (MPEG-2 TS over UDP, uni/multicast), `ts_http::TsHttpSource` (MPEG-2 TS
//! over HTTP), `hls_pull::HlsPullRoute` (pull a remote (LL-)HLS origin),
//! `dash_pull::DashPullRoute` (pull a remote MPEG-DASH origin, issue #758),
//! `smooth_pull::SmoothPullRoute` (pull a remote Microsoft Smooth Streaming
//! origin, issue #759), `rtmp::RtmpSource` (RTMP push ingest, issue #738 — a
//! *push* source; every non-SRT source above dials out), and `srt::SrtSource`
//! (SRT-carried MPEG-2 TS ingest, issue #739 — listener *or* caller mode) all
//! implement the `Source` marker trait plus the `pipeline::SampleSource`
//! contract (see `crate::pipeline`), keeping ingest swappable (and letting
//! tests drive a mock). `http_auth` is shared auth glue for the HTTP-based
//! sources (issue #663 P3c).

pub mod dash_pull;
pub mod hls_pull;
pub mod http_auth;
pub mod rtmp;
pub mod rtp_udp;
pub mod rtsp;
pub mod sdp;
pub mod smooth_pull;
pub mod srt;
pub mod ts_http;
pub mod ts_program;
pub mod ts_udp;
pub(crate) mod udp;

use std::time::Duration;

/// Read-size hint every MPEG-2 TS transport reports via
/// [`broadcast_common::Stage::demand`], and the read-buffer size the
/// datagram transports allocate — comfortably above a typical 7×188-byte
/// (1316-byte) TS-over-UDP payload and any legal UDP datagram (65 507 bytes
/// over IPv4), so a single `recv` always captures a whole datagram.
pub const MAX_TS_READ: usize = 65_536;

/// Hard cap on concurrently in-flight HTTP fetches a pull source
/// (`hls_pull`/`dash_pull`/`smooth_pull`, plan step 5a round 3) keeps open at
/// once.
///
/// A pull source's sans-IO session can hand back many `poll_transmit`
/// requests in one drain — an LL-HLS playlist reload can reveal a dozen
/// already-available parts at once; a DASH/Smooth manifest refresh can extend
/// several Representations'/StreamIndexes' plans simultaneously — with
/// nothing in the session itself limiting how many the driver launches as
/// concurrent requests. This project has already shipped five
/// unbounded-allocation vectors in code driven by remote input (see
/// `media_plane::ingress`'s own `max_programs`/`max_sessions` docs); an
/// uncapped fan-out of concurrent fetches against a single origin is exactly
/// that class of bug (a hostile or malformed playlist/manifest could turn one
/// route into an unbounded number of open sockets), so each pull source's own
/// tokio drive loop launches at most this many fetches at once, queuing the
/// rest until a slot frees up — never blocking the sans-IO session from
/// producing more requests, only how many the IO side acts on concurrently.
pub const MAX_INFLIGHT_FETCHES: usize = 8;

/// `true` while a pull source's drive loop may launch one more concurrent
/// fetch — i.e. `inflight` is still below [`MAX_INFLIGHT_FETCHES`].
///
/// A named predicate rather than an inline `<` in each of the three loops so
/// the bound is one testable decision instead of three copies of a comparison
/// (the shape that lets one of them silently drift). Every
/// `source::{hls_pull, dash_pull, smooth_pull}` loop gates its
/// `JoinSet::spawn` on this.
pub fn may_spawn_fetch(inflight: usize) -> bool {
    inflight < MAX_INFLIGHT_FETCHES
}

#[cfg(test)]
mod inflight_tests {
    use super::{MAX_INFLIGHT_FETCHES, may_spawn_fetch};

    /// The in-flight cap actually caps. Bites on the two mutations that
    /// matter: dropping the gate (making this always `true`) and inverting
    /// the comparison.
    #[test]
    fn may_spawn_fetch_stops_exactly_at_the_cap() {
        assert!(may_spawn_fetch(0), "an idle loop must be able to spawn");
        assert!(
            may_spawn_fetch(MAX_INFLIGHT_FETCHES - 1),
            "one slot short of the cap must still spawn"
        );
        assert!(
            !may_spawn_fetch(MAX_INFLIGHT_FETCHES),
            "at the cap, no further fetch may be launched"
        );
        assert!(
            !may_spawn_fetch(MAX_INFLIGHT_FETCHES + 1),
            "past the cap (a caller that over-spawned) must not spawn more"
        );
    }
}

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
/// out-of-band SDP configured for [`rtp_udp::RtpUdpRoute`]).
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
    /// [`rtp_udp::RtpUdpRoute`], which has no control plane).
    pub control: Option<String>,
    /// Interleaved RTP channel assigned to this media (RTCP = channel + 1).
    /// RTSP-only framing; unused by [`rtp_udp::RtpUdpRoute`].
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
