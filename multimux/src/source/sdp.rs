//! Parse an RTSP DESCRIBE SDP body into per-track init (codec config + clock
//! rate + control URL + assigned interleaved channel).

use crate::error::{MultimuxError, Result};
use crate::source::TrackInit;
use sdp_types::Session;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::rtpmap_clock_rate;
use transmux::{aac_config_from_fmtp, avc_config_from_fmtp};

/// Unknown coded dimensions from SDP alone (matches transmux's own
/// placeholder convention for `CodecConfig::Avc`).
const UNKNOWN_DIMENSION: u16 = 0;

/// Default RTP clock rate when `a=rtpmap` is missing or unparsable.
const DEFAULT_CLOCK_RATE_HZ: u32 = 90_000;

/// Interleaved channels are assigned in steps of 2 (RTP even, RTCP = +1).
const CHANNEL_STEP: u8 = 2;

/// Parse the DESCRIBE SDP body into per-track init, assigning interleaved
/// channels 0,2,4,… in media order.
pub fn parse_sdp_tracks(sdp: &[u8]) -> Result<Vec<TrackInit>> {
    let session = Session::parse(sdp).map_err(|e| MultimuxError::Sdp {
        reason: format!("parse: {e}"),
    })?;
    let mut tracks = Vec::new();
    let mut track_id = 1u32;
    let mut channel = 0u8;
    for media in &session.medias {
        let fmtp = media.get_first_attribute_value("fmtp").ok().flatten();
        let rtpmap = media.get_first_attribute_value("rtpmap").ok().flatten();
        let control = media
            .get_first_attribute_value("control")
            .ok()
            .flatten()
            .map(|s| s.to_string());
        let clock_rate = rtpmap
            .and_then(rtpmap_clock_rate)
            .unwrap_or(DEFAULT_CLOCK_RATE_HZ);
        // `m=<media> <port> <proto> <fmt>` (RFC 4566 §5.14): `fmt` is the
        // space-separated payload-type list; this crate assumes one codec
        // per media (matches the fmtp/rtpmap lookups above, which take the
        // *first* value regardless of which payload type they're tagged
        // with), so only the first listed payload type is kept. It is the
        // only signal a raw RTP/UDP source (no RTSP interleaved-channel
        // framing) has to route an incoming packet to its track — see
        // `crate::source::rtp_udp`.
        let payload_type: u8 = media
            .fmt
            .split_whitespace()
            .next()
            .and_then(|pt| pt.parse().ok())
            .ok_or_else(|| MultimuxError::Sdp {
                reason: format!(
                    "media {:?} has no payload type in its fmt field",
                    media.media
                ),
            })?;

        let (kind, config): (RtpMediaKind, CodecConfig) = match media.media.as_str() {
            "video" => {
                let fmtp = fmtp.ok_or_else(|| MultimuxError::Sdp {
                    reason: "video media missing fmtp".into(),
                })?;
                let avc = avc_config_from_fmtp(fmtp)?;
                (
                    RtpMediaKind::H264,
                    CodecConfig::Avc {
                        config: avc,
                        width: UNKNOWN_DIMENSION,
                        height: UNKNOWN_DIMENSION,
                    },
                )
            }
            "audio" => {
                let fmtp = fmtp.ok_or_else(|| MultimuxError::Sdp {
                    reason: "audio media missing fmtp".into(),
                })?;
                (RtpMediaKind::Aac, aac_config_from_fmtp(fmtp)?)
            }
            other => {
                return Err(MultimuxError::Sdp {
                    reason: format!("unsupported media {other:?}"),
                });
            }
        };
        tracks.push(TrackInit {
            track_id,
            kind,
            config,
            clock_rate,
            control,
            channel,
            payload_type,
        });
        track_id += 1;
        channel = channel.saturating_add(CHANNEL_STEP);
    }
    if tracks.is_empty() {
        return Err(MultimuxError::Sdp {
            reason: "SDP has no supported media".into(),
        });
    }
    Ok(tracks)
}

/// Loads an SDP body from `spec`: either the literal inline text, or —
/// prefixed with `@` — a file path read fresh on every call, so an on-disk
/// SDP can be updated between reconnects without a process restart. Used by
/// [`crate::source::rtp_udp::RtpUdpSource::connect`], the raw-RTP-over-UDP
/// ingest source that has no RTSP DESCRIBE to fetch its SDP from.
pub fn load_sdp(spec: &str) -> Result<Vec<u8>> {
    match spec.strip_prefix('@') {
        Some(path) => std::fs::read(path).map_err(|e| MultimuxError::Sdp {
            reason: format!("failed to read SDP file {path:?}: {e}"),
        }),
        None => Ok(spec.as_bytes().to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal H.264 SDP (one video media) with a real sprop-parameter-sets
    // (SPS: nal_unit_type 7, 18 bytes; PPS: nal_unit_type 8, 4 bytes).
    const SDP: &[u8] = b"v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 0 RTP/AVP 96\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\n\
a=control:streamid=0\r\n";

    #[test]
    fn parses_h264_video_track() {
        let tracks = parse_sdp_tracks(SDP).unwrap();
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert!(matches!(t.kind, RtpMediaKind::H264));
        assert_eq!(t.clock_rate, 90_000);
        assert_eq!(t.channel, 0, "first media gets RTP channel 0");
        assert_eq!(t.control.as_deref(), Some("streamid=0"));
        assert_eq!(t.track_id, 1);
        assert_eq!(
            t.payload_type, 96,
            "the m=video line's fmt payload type must be captured"
        );
    }

    #[test]
    fn load_sdp_returns_inline_text_verbatim() {
        let loaded = load_sdp(std::str::from_utf8(SDP).unwrap()).unwrap();
        assert_eq!(loaded, SDP);
    }

    /// Biting test: `@path` must read the file's bytes, not treat the whole
    /// `@path` string as literal SDP text (which would then fail to parse).
    #[test]
    fn load_sdp_at_prefix_reads_file() {
        let dir = std::env::temp_dir().join(format!("multimux-sdp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sdp");
        std::fs::write(&path, SDP).unwrap();
        let loaded = load_sdp(&format!("@{}", path.display())).unwrap();
        assert_eq!(loaded, SDP);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_sdp_at_prefix_missing_file_errors() {
        let err = load_sdp("@/no/such/path/multimux-missing.sdp");
        assert!(err.is_err());
    }

    #[test]
    fn rejects_sdp_without_media() {
        let sdp = b"v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
        assert!(parse_sdp_tracks(sdp).is_err());
    }
}
