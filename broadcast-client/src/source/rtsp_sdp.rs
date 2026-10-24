//! DESCRIBE SDP to per-track init (RFC 4566 media sections, RFC 6184 / RFC 3640 fmtp).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sdp_types::Session;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::rtpmap_clock_rate;
use transmux::{TrackId, aac_config_from_fmtp, avc_config_from_fmtp};

const UNKNOWN_DIMENSION: u16 = 0;
const DEFAULT_CLOCK_RATE_HZ: u32 = 90_000;
const CHANNEL_STEP: u8 = 2;
const MEDIA_VIDEO: &str = "video";
const MEDIA_AUDIO: &str = "audio";

pub(crate) struct TrackInit {
    pub(crate) track_id: TrackId,
    pub(crate) kind: RtpMediaKind,
    pub(crate) config: CodecConfig,
    pub(crate) clock_rate: u32,
    pub(crate) control: Option<String>,
    pub(crate) channel: u8,
}

/// Parses the SDP into the supported (H.264 video, AAC audio) tracks in media order, assigning
/// interleaved RTP channels 0, 2, 4, ... (RTCP is the RTP channel plus one). An unsupported
/// media section, a missing fmtp, or an SDP without media is an error.
pub(crate) fn parse_tracks(sdp: &[u8]) -> Result<Vec<TrackInit>, String> {
    let session = Session::parse(sdp).map_err(|e| e.to_string())?;
    let mut tracks = Vec::new();
    let mut channel = 0u8;
    for (track_id, media) in (1u32..).zip(session.medias.iter()) {
        let fmtp = media.get_first_attribute_value("fmtp").flatten();
        let rtpmap = media.get_first_attribute_value("rtpmap").flatten();
        let control = media
            .get_first_attribute_value("control")
            .flatten()
            .map(ToString::to_string);
        let clock_rate = rtpmap
            .and_then(rtpmap_clock_rate)
            .unwrap_or(DEFAULT_CLOCK_RATE_HZ);
        let (kind, config) = match media.media.as_str() {
            MEDIA_VIDEO => {
                let fmtp = fmtp.ok_or("video media without fmtp")?;
                let avc = avc_config_from_fmtp(fmtp).map_err(|e| e.to_string())?;
                (
                    RtpMediaKind::H264,
                    CodecConfig::Avc {
                        config: avc,
                        width: UNKNOWN_DIMENSION,
                        height: UNKNOWN_DIMENSION,
                    },
                )
            }
            MEDIA_AUDIO => {
                let fmtp = fmtp.ok_or("audio media without fmtp")?;
                (
                    RtpMediaKind::Aac,
                    aac_config_from_fmtp(fmtp).map_err(|e| e.to_string())?,
                )
            }
            other => return Err(alloc::format!("unsupported media {other}")),
        };
        tracks.push(TrackInit {
            track_id: TrackId::new(track_id),
            kind,
            config,
            clock_rate,
            control,
            channel,
        });
        channel = channel.saturating_add(CHANNEL_STEP);
    }
    if tracks.is_empty() {
        return Err("SDP without media".to_string());
    }
    Ok(tracks)
}
