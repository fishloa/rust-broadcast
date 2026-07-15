//! Parse an RTSP DESCRIBE SDP body into per-track init (codec config + clock
//! rate + control URL + assigned interleaved channel).

use crate::error::{MultimuxError, Result};
use crate::source::TrackInit;
use sdp_types::Session;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::{aac_config_from_fmtp, avc_config_from_sprop};

/// Unknown coded dimensions from SDP alone (matches transmux's own
/// placeholder convention for `CodecConfig::Avc`).
const UNKNOWN_DIMENSION: u16 = 0;

/// Default RTP clock rate when `a=rtpmap` is missing or unparsable.
const DEFAULT_CLOCK_RATE_HZ: u32 = 90_000;

/// Interleaved channels are assigned in steps of 2 (RTP even, RTCP = +1).
const CHANNEL_STEP: u8 = 2;

/// Extract `key=<value>` from an fmtp attribute string (`;`/space separated).
fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let idx = fmtp.find(&needle)? + needle.len();
    let rest = &fmtp[idx..];
    let end = rest.find([';', ' ', '\r', '\n']).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// clock rate from `a=rtpmap:<pt> <enc>/<rate>[/<ch>]`.
fn rtpmap_clock_rate(rtpmap: &str) -> Option<u32> {
    let after_slash = rtpmap.split('/').nth(1)?;
    after_slash.split(['/', ' ']).next()?.trim().parse().ok()
}

/// Parse the DESCRIBE SDP body into per-track init, assigning interleaved
/// channels 0,2,4,… in media order.
pub fn parse_sdp_tracks(sdp: &[u8]) -> Result<Vec<TrackInit>> {
    let session =
        Session::parse(sdp).map_err(|e| MultimuxError::Source(format!("sdp parse: {e}")))?;
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

        let (kind, config): (RtpMediaKind, CodecConfig) = match media.media.as_str() {
            "video" => {
                let fmtp =
                    fmtp.ok_or_else(|| MultimuxError::Source("video media missing fmtp".into()))?;
                let sprop = fmtp_param(fmtp, "sprop-parameter-sets")
                    .ok_or_else(|| MultimuxError::Source("no sprop-parameter-sets".into()))?;
                let avc = avc_config_from_sprop(sprop)?;
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
                let fmtp =
                    fmtp.ok_or_else(|| MultimuxError::Source("audio media missing fmtp".into()))?;
                let cfg_hex = fmtp_param(fmtp, "config")
                    .ok_or_else(|| MultimuxError::Source("no AAC config=".into()))?;
                (RtpMediaKind::Aac, aac_config_from_fmtp(cfg_hex)?)
            }
            other => {
                return Err(MultimuxError::Source(format!(
                    "unsupported media {other:?}"
                )));
            }
        };
        tracks.push(TrackInit {
            track_id,
            kind,
            config,
            clock_rate,
            control,
            channel,
        });
        track_id += 1;
        channel = channel.saturating_add(CHANNEL_STEP);
    }
    if tracks.is_empty() {
        return Err(MultimuxError::Source("SDP has no supported media".into()));
    }
    Ok(tracks)
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
    }

    #[test]
    fn rejects_sdp_without_media() {
        let sdp = b"v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
        assert!(parse_sdp_tracks(sdp).is_err());
    }
}
