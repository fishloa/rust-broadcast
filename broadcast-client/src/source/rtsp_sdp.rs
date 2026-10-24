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
    pub(crate) skipped: Vec<SkippedMedia>,
}

/// A media section the client does not play, by its position among all `m=` lines. `codec` is the
/// first listed payload's `a=rtpmap` encoding name; `codec_unsupported` is set when that names a
/// codec other than the ones played (as opposed to a played codec with an unusable `a=fmtp`, or a
/// section that names no codec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedMedia {
    pub(crate) media_index: usize,
    pub(crate) codec: Option<String>,
    pub(crate) codec_unsupported: bool,
}

pub(crate) enum SdpError {
    Unparsable,
    NothingPlayable { skipped: Vec<SkippedMedia> },
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
/// unusable fmtp), whose `a=control` `resolve` rejects, or that does not fit the track or channel
/// budget is reported in [`ParsedSdp::skipped`]; no playable section left is
/// [`SdpError::NothingPlayable`], and an SDP that does not parse is [`SdpError::Unparsable`]. Of
/// the payload types a media line lists, the first one whose own `a=rtpmap` and `a=fmtp` describe
/// a supported codec is the one played. A session-level `a=control` that `resolve` accepts is the
/// aggregate control URL (RFC 2326 C.1.1).
pub(crate) fn parse_tracks(
    sdp: &[u8],
    mut resolve: impl FnMut(Option<&str>) -> Option<String>,
) -> Result<ParsedSdp, SdpError> {
    let session = Session::parse(sdp).map_err(|_| SdpError::Unparsable)?;
    let aggregate = session
        .get_first_attribute_value("control")
        .flatten()
        .and_then(|control| resolve(Some(control)));
    let mut tracks: Vec<TrackInit> = Vec::new();
    let mut skipped: Vec<SkippedMedia> = Vec::new();
    for (media_index, media) in session.medias.iter().enumerate() {
        let skip = |skipped: &mut Vec<SkippedMedia>| skipped.push(skipped_media(media_index, media));
        if tracks.len() >= MAX_TRACKS {
            skip(&mut skipped);
            continue;
        }
        let Some((kind, config, clock_rate)) = supported(media) else {
            skip(&mut skipped);
            continue;
        };
        let control = media.get_first_attribute_value("control").flatten();
        let Some(setup_uri) = resolve(control) else {
            skip(&mut skipped);
            continue;
        };
        let channels = tracks
            .len()
            .checked_mul(CHANNEL_STEP)
            .and_then(|c| u8::try_from(c).ok())
            .and_then(|c| c.checked_add(1).map(|r| (c, r)));
        let Some((channel, rtcp_channel)) = channels else {
            skip(&mut skipped);
            continue;
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
        return Err(SdpError::NothingPlayable { skipped });
    }
    Ok(ParsedSdp {
        aggregate,
        tracks,
        skipped,
    })
}

fn skipped_media(media_index: usize, media: &Media) -> SkippedMedia {
    let codec = media
        .fmt
        .split_whitespace()
        .next()
        .and_then(|pt| attribute_of_payload(media, "rtpmap", pt))
        .and_then(encoding_name)
        .map(str::to_string);
    let played = |name: &str| match media.media.as_str() {
        MEDIA_VIDEO => name.eq_ignore_ascii_case(ENCODING_H264),
        MEDIA_AUDIO => name.eq_ignore_ascii_case(ENCODING_AAC),
        _ => false,
    };
    let codec_unsupported = codec.as_deref().is_some_and(|name| !played(name));
    SkippedMedia {
        media_index,
        codec,
        codec_unsupported,
    }
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
