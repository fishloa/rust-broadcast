//! DESCRIBE SDP to per-track init (RFC 4566 media sections, RFC 6184 / RFC 3640 fmtp).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sdp_types::{Media, Session};
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::rtpmap_clock_rate;
use transmux::{TrackId, aac_config_from_fmtp, avc_config_from_fmtp};

const UNKNOWN_DIMENSION: u16 = 0;
const DEFAULT_CLOCK_RATE_HZ: u32 = 90_000;
const CHANNEL_STEP: usize = 2;
const MEDIA_VIDEO: &str = "video";
const MEDIA_AUDIO: &str = "audio";
const ENCODING_H264: &str = "H264";
const ENCODING_AAC: &str = "MPEG4-GENERIC";

/// Most media sections a session plays; further supported sections are ignored.
pub(crate) const MAX_TRACKS: usize = 16;

pub(crate) struct ParsedSdp {
    pub(crate) aggregate: Option<String>,
    pub(crate) tracks: Vec<TrackInit>,
}

pub(crate) struct TrackInit {
    pub(crate) track_id: TrackId,
    pub(crate) kind: RtpMediaKind,
    pub(crate) config: CodecConfig,
    pub(crate) clock_rate: u32,
    pub(crate) setup_uri: String,
    pub(crate) channel: u8,
    pub(crate) rtcp_channel: u8,
}

/// Parses the SDP into the supported (H.264 video, AAC audio) tracks in media order, at most
/// [`MAX_TRACKS`], assigning interleaved RTP channels 0, 2, 4, ... (RTCP is the RTP channel plus
/// one). A media section the client cannot play (another codec or media type, a missing or
/// unusable fmtp) or whose `a=control` `resolve` rejects is skipped; no playable section left,
/// or an SDP that does not parse, is an error. Of the payload types a media line lists, the first
/// one whose own `a=rtpmap` and `a=fmtp` describe a supported codec is the one played. A
/// session-level `a=control` that `resolve` accepts is the aggregate control URL (RFC 2326
/// C.1.1).
pub(crate) fn parse_tracks(
    sdp: &[u8],
    mut resolve: impl FnMut(Option<&str>) -> Option<String>,
) -> Result<ParsedSdp, String> {
    let session = Session::parse(sdp).map_err(|e| e.to_string())?;
    let aggregate = session
        .get_first_attribute_value("control")
        .flatten()
        .and_then(|control| resolve(Some(control)));
    let mut tracks: Vec<TrackInit> = Vec::new();
    for media in &session.medias {
        if tracks.len() >= MAX_TRACKS {
            break;
        }
        let Some((kind, config, clock_rate)) = supported(media) else {
            continue;
        };
        let control = media.get_first_attribute_value("control").flatten();
        let Some(setup_uri) = resolve(control) else {
            continue;
        };
        let Some(channel) = tracks
            .len()
            .checked_mul(CHANNEL_STEP)
            .and_then(|c| u8::try_from(c).ok())
        else {
            break;
        };
        let Some(rtcp_channel) = channel.checked_add(1) else {
            break;
        };
        let ordinal = u32::try_from(tracks.len().saturating_add(1)).unwrap_or(u32::MAX);
        tracks.push(TrackInit {
            track_id: TrackId::new(ordinal),
            kind,
            config,
            clock_rate,
            setup_uri,
            channel,
            rtcp_channel,
        });
    }
    if tracks.is_empty() {
        return Err("SDP without supported media".to_string());
    }
    Ok(ParsedSdp { aggregate, tracks })
}

fn encoding_name(rtpmap: &str) -> Option<&str> {
    rtpmap
        .split_whitespace()
        .nth(1)
        .and_then(|encoding| encoding.split('/').next())
}

fn attribute_of_payload<'a>(media: &'a Media, name: &str, payload_type: &str) -> Option<&'a str> {
    media
        .attributes
        .iter()
        .filter(|a| a.attribute == name)
        .filter_map(|a| a.value.as_deref())
        .find(|value| value.split_whitespace().next() == Some(payload_type))
}

fn supported(media: &Media) -> Option<(RtpMediaKind, CodecConfig, u32)> {
    media
        .fmt
        .split_whitespace()
        .find_map(|payload_type| supported_payload(media, payload_type))
}

fn supported_payload(
    media: &Media,
    payload_type: &str,
) -> Option<(RtpMediaKind, CodecConfig, u32)> {
    let fmtp = attribute_of_payload(media, "fmtp", payload_type)?;
    let rtpmap = attribute_of_payload(media, "rtpmap", payload_type);
    let clock_rate = rtpmap
        .and_then(rtpmap_clock_rate)
        .unwrap_or(DEFAULT_CLOCK_RATE_HZ);
    let encoding_is = |expected: &str| {
        rtpmap
            .and_then(encoding_name)
            .is_none_or(|name| name.eq_ignore_ascii_case(expected))
    };
    match media.media.as_str() {
        MEDIA_VIDEO if encoding_is(ENCODING_H264) => {
            let avc = avc_config_from_fmtp(fmtp).ok()?;
            Some((
                RtpMediaKind::H264,
                CodecConfig::Avc {
                    config: avc,
                    width: UNKNOWN_DIMENSION,
                    height: UNKNOWN_DIMENSION,
                },
                clock_rate,
            ))
        }
        MEDIA_AUDIO if encoding_is(ENCODING_AAC) => Some((
            RtpMediaKind::Aac,
            aac_config_from_fmtp(fmtp).ok()?,
            clock_rate,
        )),
        _ => None,
    }
}
