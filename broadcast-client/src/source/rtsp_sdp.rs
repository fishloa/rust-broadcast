//! DESCRIBE SDP to per-track init (RFC 4566 media sections, RFC 6184 / RFC 3640 fmtp).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rtsp_runtime::PlayPlan;
use sdp_types::Media;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::rtpmap_clock_rate;
use transmux::{TrackId, aac_config_from_fmtp, avc_config_from_fmtp};
use url::Url;

const AVC_CONFIGURATION_VERSION: u8 = 1;
const AVC_LENGTH_SIZE_MINUS_ONE: u8 = 3;
const UNKNOWN_DIMENSION: u16 = 0;
const DEFAULT_CLOCK_RATE_HZ: u32 = 90_000;
const FIRST_CHANNEL: u8 = 0;
const MEDIA_VIDEO: &str = "video";
const MEDIA_AUDIO: &str = "audio";
const ENCODING_H264: &str = "H264";
const ENCODING_AAC: &str = "MPEG4-GENERIC";

/// Most media sections a session plays; further supported sections are ignored.
pub(crate) const MAX_TRACKS: usize = 16;

pub(crate) struct ParsedSdp {
    pub(crate) plan: PlayPlan,
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
/// [`MAX_TRACKS`], planned through [`PlayPlan`] (interleaved RTP channels 0, 2, 4, ..., RTCP the
/// RTP channel plus one, and the aggregate control URL of RFC 2326 C.1.1). A media section the
/// client cannot play (another codec or media type, a missing or unusable fmtp), whose
/// `a=control` is on another authority, or that does not fit the track budget is reported in
/// [`ParsedSdp::skipped`]; no playable section left is [`SdpError::NothingPlayable`], and an SDP
/// that does not parse is [`SdpError::Unparsable`]. Of the payload types a media line lists, the
/// first one whose own `a=rtpmap` and `a=fmtp` describe a supported codec is the one played.
pub(crate) fn parse_tracks(
    sdp: &[u8],
    request_url: &Url,
    content_base: Option<&str>,
) -> Result<ParsedSdp, SdpError> {
    let text = core::str::from_utf8(sdp).map_err(|_| SdpError::Unparsable)?;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut described: Vec<SkippedMedia> = Vec::new();
    let mut skipped: Vec<SkippedMedia> = Vec::new();
    let plan = PlayPlan::from_describe_selecting(
        request_url,
        content_base,
        None,
        text,
        FIRST_CHANNEL,
        |media_index, media| {
            let info = skipped_media(media_index, media);
            described.push(info.clone());
            let playable = if candidates.len() >= MAX_TRACKS {
                None
            } else {
                supported(media)
            };
            match playable {
                Some((kind, config, clock_rate, _ready)) => {
                    candidates.push(Candidate {
                        media_index,
                        kind,
                        config,
                        clock_rate,
                    });
                    true
                }
                None => {
                    skipped.push(info);
                    false
                }
            }
        },
    )
    .map_err(|_| SdpError::Unparsable)?;
    skipped.extend(
        plan.skipped()
            .iter()
            .filter_map(|m| described.get(m.media_index).cloned()),
    );
    skipped.sort_by_key(|m| m.media_index);
    let mut tracks: Vec<TrackInit> = Vec::new();
    for (index, setup) in plan.setups().iter().enumerate() {
        let (Some(candidate), Some((channel, rtcp_channel))) = (
            candidates
                .iter()
                .find(|c| c.media_index == setup.media_index),
            plan.channels(index),
        ) else {
            continue;
        };
        let ordinal = u32::try_from(tracks.len().saturating_add(1)).unwrap_or(u32::MAX);
        tracks.push(TrackInit {
            track_id: TrackId::new(ordinal),
            kind: candidate.kind,
            config: candidate.config.clone(),
            clock_rate: candidate.clock_rate,
            setup_uri: setup.control.clone(),
            channel,
            rtcp_channel,
        });
    }
    if tracks.is_empty() {
        return Err(SdpError::NothingPlayable { skipped });
    }
    Ok(ParsedSdp {
        plan,
        tracks,
        skipped,
    })
}

struct Candidate {
    media_index: usize,
    kind: RtpMediaKind,
    config: CodecConfig,
    clock_rate: u32,
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

fn supported(media: &Media) -> Option<(RtpMediaKind, CodecConfig, u32, bool)> {
    media
        .fmt
        .split_whitespace()
        .find_map(|payload_type| supported_payload(media, payload_type))
}

fn pending_avc_config() -> CodecConfig {
    CodecConfig::Avc {
        config: transmux::AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
            configuration_version: AVC_CONFIGURATION_VERSION,
            profile_indication: 0,
            profile_compatibility: 0,
            level_indication: 0,
            length_size_minus_one: AVC_LENGTH_SIZE_MINUS_ONE,
            sps: Vec::new(),
            pps: Vec::new(),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: Vec::new(),
        }),
        width: UNKNOWN_DIMENSION,
        height: UNKNOWN_DIMENSION,
    }
}

fn supported_payload(
    media: &Media,
    payload_type: &str,
) -> Option<(RtpMediaKind, CodecConfig, u32, bool)> {
    let fmtp = attribute_of_payload(media, "fmtp", payload_type);
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
            let Some(fmtp) = fmtp else {
                let named_h264 = rtpmap.and_then(encoding_name).is_some();
                return named_h264
                    .then(|| (RtpMediaKind::H264, pending_avc_config(), clock_rate, false));
            };
            if let Ok(avc) = avc_config_from_fmtp(fmtp) {
                let (width, height) = avc
                    .config
                    .sps
                    .first()
                    .and_then(|sps| transmux::avc_sps_dimensions(&sps.0))
                    .unwrap_or((UNKNOWN_DIMENSION, UNKNOWN_DIMENSION));
                Some((
                    RtpMediaKind::H264,
                    CodecConfig::Avc {
                        config: avc,
                        width,
                        height,
                    },
                    clock_rate,
                    true,
                ))
            } else if transmux::fmtp_param(fmtp, "sprop-parameter-sets").is_none() {
                Some((RtpMediaKind::H264, pending_avc_config(), clock_rate, false))
            } else {
                None
            }
        }
        MEDIA_AUDIO if encoding_is(ENCODING_AAC) => Some((
            RtpMediaKind::Aac,
            aac_config_from_fmtp(fmtp?).ok()?,
            clock_rate,
            true,
        )),
        _ => None,
    }
}
