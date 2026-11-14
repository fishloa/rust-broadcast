//! One SDP media section read as a playable track (RFC 4566 §5.14/§6, RFC 3551 §3, RFC 6184 §8,
//! RFC 3640 §4.1); `std` because the section is an `sdp_types::Media`.

use super::{
    NAL_LENGTH_SIZE_MINUS_ONE, aac_config_from_fmtp, avc_config_from_fmtp, fmtp_param,
    rtpmap_clock_rate,
};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use broadcast_common::timescale::Timescale;
use crate::pipeline::CodecConfig;
use alloc::vec::Vec;

/// `configurationVersion` of the placeholder `avcC` of a track whose parameter sets arrive in-band.
const AVC_CONFIGURATION_VERSION: u8 = 1;
/// Coded dimension of a track whose SPS the SDP does not carry.
const UNKNOWN_DIMENSION: u16 = 0;
/// Clock rate of an RTP video payload format (RFC 6184 §8.2.1) used when `a=rtpmap` is absent or
/// unparsable.
const DEFAULT_CLOCK_RATE_HZ: u32 = 90_000;
const MEDIA_VIDEO: &str = "video";
const MEDIA_AUDIO: &str = "audio";
const ENCODING_H264: &str = "H264";
const ENCODING_AAC: &str = "MPEG4-GENERIC";

/// One SDP media section read as a playable track: the single interpretation of an `m=` line plus
/// its `a=rtpmap`/`a=fmtp`/`a=control` attributes shared by every RTSP pull, raw RTP/UDP and
/// DESCRIBE consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpTrack {
    /// Payload format of the chosen payload type.
    pub kind: crate::rtp::RtpMediaKind,
    /// Codec configuration from the chosen payload type's `a=fmtp`; a placeholder `avcC` with no
    /// parameter sets when an H.264 section carries none (RFC 6184 §8.2.1 allows them in-band).
    pub config: CodecConfig,
    /// RTP clock rate in Hz, from the chosen payload type's `a=rtpmap` (RFC 4566 §6).
    pub clock_rate: Timescale,
    /// The chosen payload type (RFC 3551 §3: 7 bits; dynamic types 96-127); `None` when the
    /// `m=` line's token is not a number in `0..=255`.
    pub payload_type: Option<u8>,
    /// The section's first `a=control` value (RFC 2326 C.1.1), unresolved.
    pub control: Option<alloc::string::String>,
}

/// The value of the first `a=<name>:<payload type> ...` attribute of `media` whose format token
/// is `payload_type` (RFC 4566 §6: `a=rtpmap` and `a=fmtp` each name the payload type they
/// describe, so lookup is per payload type, never "the first attribute").
pub fn attribute_of_payload<'a>(
    media: &'a sdp_types::Media,
    name: &str,
    payload_type: &str,
) -> Option<&'a str> {
    media
        .attributes
        .iter()
        .filter(|a| a.attribute == name)
        .filter_map(|a| a.value.as_deref())
        .find(|value| value.split_whitespace().next() == Some(payload_type))
}

/// The encoding name of an `a=rtpmap:<pt> <encoding name>/<clock rate>[/<parameters>]` value
/// (RFC 4566 §6).
pub fn rtpmap_encoding_name(rtpmap: &str) -> Option<&str> {
    rtpmap
        .split_whitespace()
        .nth(1)
        .and_then(|encoding| encoding.split('/').next())
}

/// The encoding name the first payload type of `media`'s `m=` line is mapped to, if it has an
/// `a=rtpmap`.
pub fn first_encoding_name(media: &sdp_types::Media) -> Option<&str> {
    media
        .fmt
        .split_whitespace()
        .next()
        .and_then(|pt| attribute_of_payload(media, "rtpmap", pt))
        .and_then(rtpmap_encoding_name)
}

/// Whether `encoding_name` is the payload format [`sdp_track`] plays for `media_type` (`video`:
/// H.264, RFC 6184; `audio`: `MPEG4-GENERIC`, RFC 3640). Encoding names are case-insensitive
/// (RFC 4566 §6).
pub fn is_played_encoding(media_type: &str, encoding_name: &str) -> bool {
    match media_type {
        MEDIA_VIDEO => encoding_name.eq_ignore_ascii_case(ENCODING_H264),
        MEDIA_AUDIO => encoding_name.eq_ignore_ascii_case(ENCODING_AAC),
        _ => false,
    }
}

fn pending_avc_config() -> CodecConfig {
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: AVC_CONFIGURATION_VERSION,
            profile_indication: 0,
            profile_compatibility: 0,
            level_indication: 0,
            length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
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

fn track_of_payload(media: &sdp_types::Media, payload_type: &str) -> Option<SdpTrack> {
    use crate::rtp::{AAC_INDEX_DELTA_LENGTH, AAC_INDEX_LENGTH, AAC_SIZE_LENGTH, RtpMediaKind};

    let fmtp = attribute_of_payload(media, "fmtp", payload_type);
    let rtpmap = attribute_of_payload(media, "rtpmap", payload_type);
    let encoding = rtpmap.and_then(rtpmap_encoding_name);
    let rtpmap_present = rtpmap.is_some();
    let parsed_clock = rtpmap.and_then(rtpmap_clock_rate);
    let encoding_is =
        |expected: &str| encoding.is_none_or(|name| name.eq_ignore_ascii_case(expected));
    let (kind, config, clock_rate) = match media.media.as_str() {
        MEDIA_VIDEO if encoding_is(ENCODING_H264) => {
            // RFC 6184 §6.2-6.4: modes 0 (single NAL unit) and 1 (non-interleaved) only.
            if let Some(fmtp) = fmtp
                && let Some(mode) = fmtp_param(fmtp, "packetization-mode")
                && mode != "0"
                && mode != "1"
            {
                return None;
            }
            let config = match fmtp {
                None => encoding.map(|_| pending_avc_config())?,
                Some(fmtp) => match avc_config_from_fmtp(fmtp) {
                    Ok(avc) => {
                        let (width, height) = avc
                            .config
                            .sps
                            .first()
                            .and_then(|sps| crate::nal::avc_sps_dimensions(&sps.0))
                            .unwrap_or((UNKNOWN_DIMENSION, UNKNOWN_DIMENSION));
                        CodecConfig::Avc {
                            config: avc,
                            width,
                            height,
                        }
                    }
                    Err(_) if fmtp_param(fmtp, "sprop-parameter-sets").is_none() => {
                        pending_avc_config()
                    }
                    Err(_) => return None,
                },
            };
            let clock = match parsed_clock {
                Some(c) => c,
                None if rtpmap_present => return None,
                None => DEFAULT_CLOCK_RATE_HZ,
            };
            (RtpMediaKind::H264, config, clock)
        }
        MEDIA_AUDIO if encoding_is(ENCODING_AAC) => {
            let fmtp = fmtp?;
            let config = aac_config_from_fmtp(fmtp).ok()?;
            let mode = fmtp_param(fmtp, "mode").unwrap_or("AAC-hbr");
            if !mode.eq_ignore_ascii_case("AAC-hbr") {
                return None;
            }
            let size_length: u32 = fmtp_param(fmtp, "sizeLength")?.parse().ok()?;
            let index_length: u32 = fmtp_param(fmtp, "indexLength")?.parse().ok()?;
            let index_delta_length: u32 = fmtp_param(fmtp, "indexDeltaLength")?.parse().ok()?;
            if size_length != AAC_SIZE_LENGTH
                || index_length != AAC_INDEX_LENGTH
                || index_delta_length != AAC_INDEX_DELTA_LENGTH
            {
                return None;
            }
            let clock = parsed_clock?; // AAC requires an rtpmap clock rate
            (RtpMediaKind::Aac, config, clock)
        }
        _ => return None,
    };
    Some(SdpTrack {
        kind,
        config,
        clock_rate: Timescale::new(clock_rate)?,
        payload_type: payload_type.parse::<u8>().ok().filter(|&pt| pt <= 127),
        control: media
            .get_first_attribute_value("control")
            .flatten()
            .map(alloc::string::ToString::to_string),
    })
}

/// Reads one SDP media section as a playable track.
///
/// Of the payload types the `m=` line lists (RFC 4566 §5.14), the first whose own `a=rtpmap` and
/// `a=fmtp` (RFC 4566 §6; matched by payload type, never "the first attribute of the section")
/// describe a supported payload format is the one played: H.264 video (RFC 6184 §8.1) or
/// `MPEG4-GENERIC` AAC audio (RFC 3640 §4.1). A payload type with no `a=rtpmap` is a static or
/// unannounced one (RFC 3551 §3) and is taken to be the section's own media type. H.264 without
/// `sprop-parameter-sets` yields a placeholder `avcC` (parameter sets arrive in-band); AAC needs
/// its `config` and an explicit `indexDeltaLength=3` (RFC 3640 §3.3.6 fixes AAC-hbr at 13/3/3; the
/// §4.1 default of 0 would make every AU after the first in a packet 13 bits wide, which the
/// depacketiser's 16-bit AU headers cannot read). `None` when no listed payload type is playable.
pub fn sdp_track(media: &sdp_types::Media) -> Option<SdpTrack> {
    media
        .fmt
        .split_whitespace()
        .find_map(|payload_type| track_of_payload(media, payload_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp::RtpMediaKind;
    use sdp_types::Session;

    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    const AAC_FMTP: &str =
        "mode=AAC-hbr; config=1208; sizeLength=13; indexLength=3; indexDeltaLength=3";

    fn media(body: &str) -> sdp_types::Media {
        let text = format!("v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n{body}");
        Session::parse(text.as_bytes()).unwrap().medias.remove(0)
    }

    #[test]
    fn the_first_listed_payload_type_with_a_supported_mapping_is_played() {
        let m = media(&format!(
            "m=video 0 RTP/AVP 97 96\r\na=rtpmap:97 H265/90000\r\na=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 packetization-mode=1; sprop-parameter-sets={SPROP}\r\n"
        ));
        let track = sdp_track(&m).unwrap();
        assert_eq!(track.payload_type, Some(96));
        assert_eq!(track.kind, RtpMediaKind::H264);
        assert!(matches!(
            track.config,
            CodecConfig::Avc {
                width: 640,
                height: 480,
                ..
            }
        ));
    }

    #[test]
    fn a_payload_type_is_matched_to_its_own_attributes() {
        let m = media(&format!(
            "m=audio 0 RTP/AVP 97\r\na=rtpmap:96 H264/90000\r\na=rtpmap:97 mpeg4-generic/48000/2\r\n\
             a=fmtp:96 packetization-mode=1\r\na=fmtp:97 {AAC_FMTP}\r\n"
        ));
        let track = sdp_track(&m).unwrap();
        assert_eq!(track.kind, RtpMediaKind::Aac);
        assert_eq!(track.clock_rate, Timescale::from_nonzero(48_000));
    }

    #[test]
    fn h264_without_parameter_sets_is_a_placeholder_and_aac_without_config_is_not_playable() {
        let h264 = media("m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n");
        let Some(SdpTrack {
            config: CodecConfig::Avc { config, .. },
            ..
        }) = sdp_track(&h264)
        else {
            panic!("expected a placeholder avcC");
        };
        assert!(config.config.sps.is_empty());
        let aac = media("m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/48000/2\r\n");
        assert_eq!(sdp_track(&aac), None);
    }

    #[test]
    fn another_codec_or_media_type_is_not_playable() {
        for body in [
            "m=audio 0 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n",
            "m=video 0 RTP/AVP 98\r\na=rtpmap:98 H265/90000\r\n",
            "m=text 0 RTP/AVP 98\r\na=rtpmap:98 t140/1000\r\n",
        ] {
            assert_eq!(sdp_track(&media(body)), None, "{body}");
        }
    }

    #[test]
    fn a_non_numeric_payload_type_has_no_payload_type_number() {
        let m = media(&format!(
            "m=video 0 RTP/AVP dyn\r\na=rtpmap:dyn H264/90000\r\n\
             a=fmtp:dyn sprop-parameter-sets={SPROP}\r\n"
        ));
        assert_eq!(sdp_track(&m).unwrap().payload_type, None);
    }

    #[test]
    fn fmtp_param_names_are_case_insensitive() {
        let m = media(
            "m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/48000/2\r\n\
             a=fmtp:97 streamtype=5; Mode=AAC-hbr; Config=1190; SizeLength=13; IndexLength=3; IndexDeltaLength=3\r\n",
        );
        let track = sdp_track(&m).unwrap();
        assert_eq!(track.kind, RtpMediaKind::Aac);
    }

    #[test]
    fn aac_lbr_mode_is_not_playable() {
        let m = media(
            "m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/48000/2\r\n\
             a=fmtp:97 mode=AAC-lbr; config=1190; sizeLength=6; indexLength=2; indexDeltaLength=2\r\n",
        );
        assert_eq!(sdp_track(&m), None);
    }

    #[test]
    fn aac_without_index_delta_length_is_not_playable() {
        let m = media(
            "m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/44100/2\r\n\
             a=fmtp:97 streamtype=5;profile-level-id=1;mode=AAC-hbr;sizelength=13;indexlength=3;config=1190\r\n",
        );
        assert_eq!(sdp_track(&m), None);
    }

    #[test]
    fn aac_wrong_sizes_are_not_playable() {
        let m = media(
            "m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/48000/2\r\n\
             a=fmtp:97 mode=AAC-hbr; config=1190; sizeLength=6; indexLength=2; indexDeltaLength=2\r\n",
        );
        assert_eq!(sdp_track(&m), None);
    }

    #[test]
    fn h264_packetization_mode_2_is_not_playable() {
        let m = media(&format!(
            "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 packetization-mode=2; sprop-parameter-sets={SPROP}\r\n"
        ));
        assert_eq!(sdp_track(&m), None);
    }

    #[test]
    fn h264_packetization_modes_0_and_1_are_playable() {
        for mode in ["0", "1"] {
            let m = media(&format!(
                "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
                 a=fmtp:96 packetization-mode={mode}; sprop-parameter-sets={SPROP}\r\n"
            ));
            assert!(sdp_track(&m).is_some(), "mode {mode}");
        }
        let m = media(&format!(
            "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 sprop-parameter-sets={SPROP}\r\n"
        ));
        assert!(sdp_track(&m).is_some());
    }

    #[test]
    fn zero_clock_rate_is_invalid() {
        let m = media("m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/0\r\n");
        assert_eq!(sdp_track(&m), None);
    }

    #[test]
    fn aac_without_rtpmap_is_not_playable() {
        let m = media(
            "m=audio 0 RTP/AVP 97\r\na=fmtp:97 mode=AAC-hbr; config=1190; sizeLength=13; indexLength=3; indexDeltaLength=3\r\n",
        );
        assert_eq!(sdp_track(&m), None);
    }

    #[test]
    fn payload_type_above_127_has_no_number() {
        let m = media(&format!(
            "m=video 0 RTP/AVP 200\r\na=rtpmap:200 H264/90000\r\n\
             a=fmtp:200 sprop-parameter-sets={SPROP}\r\n"
        ));
        let track = sdp_track(&m).unwrap();
        assert_eq!(track.payload_type, None);
    }
}
