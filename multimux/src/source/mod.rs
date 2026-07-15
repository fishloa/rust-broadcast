//! Ingest sources feeding the segmentation pipeline. v1 ships `RtspSource`;
//! the `Source` trait keeps ingest swappable (and lets tests drive a mock).

pub mod rtsp;
pub mod sdp;

use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;

/// Per-track init derived from the DESCRIBE SDP.
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
    /// Per-media `a=control` URL suffix for SETUP.
    pub control: Option<String>,
    /// Interleaved RTP channel assigned to this media (RTCP = channel + 1).
    pub channel: u8,
}

/// An ingest source that can be identified by name (e.g. for logging/metrics).
///
/// Kept minimal here; Task 5's `RtspSource` extends the ingest surface with
/// the actual RTSP session driving.
pub trait Source {
    /// Human-readable stream name (e.g. the RTSP URL or config-file key).
    fn stream_name(&self) -> &str;
}
