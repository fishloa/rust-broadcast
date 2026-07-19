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

use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;

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
