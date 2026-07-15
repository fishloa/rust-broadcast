//! SDP fmtp/rtpmap → transmux `CodecConfig` (RFC 4566 §5.14/§6, RFC 6184
//! §8.1, RFC 3640 §4.1).
//!
//! Turns the media-format parameters carried in an RTSP DESCRIBE SDP into the
//! codec configuration transmux muxers need: H.264 `sprop-parameter-sets`
//! (base64 SPS/PPS) → `avcC`, AAC `config` (hex AudioSpecificConfig) → `esds`.
//! The caller (e.g. multimux) extracts the raw `a=fmtp`/`a=rtpmap` attribute
//! strings via an SDP parser; this module owns the fmtp *parameter-list*
//! parsing (a proper anchored `key=value` parser, [`fmtp_param`]) and the
//! codec-config construction, because transmux owns
//! `AVCConfigurationBox`/`EsdsBox`.
//!
//! Two entry points per codec: a full-fmtp-line function
//! ([`avc_config_from_fmtp`]/[`aac_config_from_fmtp`]) that extracts the
//! relevant parameter via [`fmtp_param`], and a value-level building block
//! ([`avc_config_from_sprop`]/[`aac_config_from_asc_hex`]) that takes just
//! that parameter's already-extracted value. [`rtpmap_clock_rate`] parses the
//! companion `a=rtpmap` attribute's clock rate.
//!
//! See [`transmux/docs/rtp/rtp-payload-formats.md`](../rtp/rtp-payload-formats.md)
//! for the RFC background and SDP fmtp→CodecConfig mapping specification.

use crate::aac_asc::{AudioSpecificConfig, SamplingFrequencyIndex};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType,
};
use crate::nal::{NalCodec, nal_unit_type};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::CodecConfig;
use crate::rtp::{base64_decode, hex_decode};
use alloc::vec::Vec;
use broadcast_common::Parse;

/// Length prefix size transmux uses for coded NALs (4-byte).
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// H.264 `nal_unit_type` for a sequence parameter set (SPS).
const AVC_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for a picture parameter set (PPS).
const AVC_NAL_PPS: u8 = 8;

/// MPEG-4 Audio object-type indication (ISO/IEC 14496-1 §7.2.6.6 Table 5).
const OTI_AUDIO_ISO14496_3: u8 = 0x40;
/// MPEG-4 audio stream type (ISO/IEC 14496-1 §7.2.6.6 Table 6).
const STREAM_TYPE_AUDIO: u8 = 5;
/// `SLConfigDescriptor` `predefined = 2` (MP4 storage) — ISO/IEC 14496-14 §3.1.2.
const SL_CONFIG_PREDEFINED_MP4: u8 = 2;
/// AAC sample size is always 16 bits in the sample entry (fMP4/CMAF convention).
const AAC_SAMPLE_SIZE_BITS: u16 = 16;

/// `samplingFrequencyIndex` → Hz — ISO/IEC 14496-3:2001 §1.6.3.3 Table 1.10.
const SAMPLING_FREQUENCY_TABLE_HZ: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// Look up the sampling rate for a `samplingFrequencyIndex` per Table 1.10.
/// Indices 13-14 are reserved and 15 is the explicit-rate escape (neither has
/// a table entry).
fn sampling_frequency_table_hz(index: &SamplingFrequencyIndex) -> Option<u32> {
    let raw = index.raw() as usize;
    SAMPLING_FREQUENCY_TABLE_HZ.get(raw).copied()
}

/// Sample rate from the ASC: the explicit escape value if present, otherwise
/// the ISO/IEC 14496-3 Table 1.10 frequency for the index.
fn asc_sample_rate(asc: &AudioSpecificConfig) -> Result<u32> {
    if let Some(freq) = asc.sampling_frequency {
        return Ok(freq);
    }
    sampling_frequency_table_hz(&asc.sampling_frequency_index).ok_or(Error::InvalidValue {
        field: "sampling_frequency_index",
        value: u64::from(asc.sampling_frequency_index.raw()),
        reason: "no frequency for index",
    })
}

/// Parse an SDP AAC `config` fmtp value (RFC 3640 §4.1: hex-encoded
/// `AudioSpecificConfig`) into `CodecConfig::Aac`, recovering sample rate and
/// channel count from the ASC and carrying the ASC bytes in the `esds`.
///
/// Takes the raw hex VALUE of the `config` parameter (not the full `a=fmtp`
/// line) — for the full-line entry point see [`aac_config_from_fmtp`].
pub fn aac_config_from_asc_hex(config_hex: &str) -> Result<CodecConfig> {
    let asc_bytes = hex_decode(config_hex)?;
    let asc = AudioSpecificConfig::parse(&asc_bytes)?;
    let sample_rate = asc_sample_rate(&asc)?;
    let channel_count = u16::from(asc.channel_configuration.raw());

    let esds = EsdsBox::new(ESDescriptor {
        es_id: 0,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(OTI_AUDIO_ISO14496_3),
            stream_type: StreamType(STREAM_TYPE_AUDIO),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo { data: asc_bytes }),
        }),
        sl_config: Some(SLConfigDescriptor {
            body: alloc::vec![SL_CONFIG_PREDEFINED_MP4],
        }),
    });

    Ok(CodecConfig::Aac {
        esds,
        channel_count,
        sample_rate,
        sample_size: AAC_SAMPLE_SIZE_BITS,
    })
}

/// Parse an SDP `sprop-parameter-sets` value (RFC 6184 §8.1: comma-separated
/// base64 parameter-set NAL units) into an `avcC` configuration box.
///
/// SPS units (nal_unit_type 7) supply `profile_indication` /
/// `profile_compatibility` / `level_indication` (SPS bytes `[1..4]` after the
/// NAL header). At least one SPS is required.
pub fn avc_config_from_sprop(sprop_parameter_sets: &str) -> Result<AVCConfigurationBox> {
    let mut sps: Vec<AvcSps> = Vec::new();
    let mut pps: Vec<AvcPps> = Vec::new();
    for token in sprop_parameter_sets.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let nal = base64_decode(token)?;
        if nal.is_empty() {
            return Err(Error::InvalidInput("empty sprop parameter set"));
        }
        match nal_unit_type(NalCodec::Avc, &nal) {
            Some(AVC_NAL_SPS) => sps.push(AvcSps(nal)),
            Some(AVC_NAL_PPS) => pps.push(AvcPps(nal)),
            _ => return Err(Error::InvalidInput("sprop NAL is neither SPS nor PPS")),
        }
    }
    let first_sps = sps
        .first()
        .ok_or(Error::InvalidInput("sprop-parameter-sets contained no SPS"))?;
    if first_sps.0.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: first_sps.0.len(),
            what: "SPS profile/level bytes",
        });
    }
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: first_sps.0[1],
        profile_compatibility: first_sps.0[2],
        level_indication: first_sps.0[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps,
        pps,
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };
    Ok(AVCConfigurationBox::new(record))
}

/// Strip a leading `<pt> ` payload-type token from an SDP `a=fmtp`/`a=rtpmap`
/// attribute value, if present (RFC 4566 §5.14 `a=fmtp:<format> <format
/// specific parameters>` / §6 `a=rtpmap:<payload type> <encoding name>/...` —
/// the payload type is a token of its own, not part of the parameter list or
/// encoding name). The token is only recognised when it is all ASCII digits
/// followed by whitespace, so a parameter-list-only input (no leading token)
/// is returned unchanged.
fn strip_leading_pt_token(value: &str) -> &str {
    let trimmed = value.trim_start();
    match trimmed.split_once(char::is_whitespace) {
        Some((pt, rest)) if !pt.is_empty() && pt.bytes().all(|b| b.is_ascii_digit()) => {
            rest.trim_start()
        }
        _ => trimmed,
    }
}

/// Look up a single parameter by key in an SDP `a=fmtp:<pt> <parameters>`
/// value (RFC 4566 §5.14; the `<parameters>` grammar is per-format, but every
/// RTP payload format in this module uses the common `;`-separated
/// `key=value` convention, e.g. RFC 6184 §8.1 `sprop-parameter-sets` / RFC
/// 3640 §4.1 `config`).
///
/// Accepts either shape as input:
/// - the full attribute value including the leading `<pt> ` payload-type
///   token (e.g. `"96 packetization-mode=1;sprop-parameter-sets=..."`), or
/// - just the `;`-separated parameter list with no leading token (e.g.
///   `"packetization-mode=1;sprop-parameter-sets=..."`).
///
/// `key` is matched as a whole parameter name, never a substring — matching
/// is anchored by splitting each `;`-separated pair at its *first* `=` and
/// comparing the trimmed left-hand side to `key` exactly. Returns the
/// trimmed value of the first match, or `None` if `key` is absent or its
/// value is empty after trimming.
///
/// Operates on `&str` throughout (`split`/`split_once`/`trim` all walk `char`
/// boundaries, never raw byte offsets), so multibyte UTF-8 parameter values
/// are preserved verbatim.
pub fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    let params = strip_leading_pt_token(fmtp);
    for pair in params.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        // A pair without '=' is not a key=value parameter (malformed, or a
        // bare flag some formats allow) — skip it rather than aborting the
        // whole scan, so a later valid pair can still match `key`.
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if k.trim() == key {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Parse a full SDP `a=fmtp:<pt> <parameters>` H.264 value (RFC 6184 §8.1)
/// into an `avcC` configuration box, extracting `sprop-parameter-sets` via
/// [`fmtp_param`] and delegating to [`avc_config_from_sprop`].
pub fn avc_config_from_fmtp(fmtp: &str) -> Result<AVCConfigurationBox> {
    let sprop = fmtp_param(fmtp, "sprop-parameter-sets").ok_or(Error::InvalidInput(
        "fmtp has no sprop-parameter-sets parameter",
    ))?;
    avc_config_from_sprop(sprop)
}

/// Parse a full SDP `a=fmtp:<pt> <parameters>` AAC value (RFC 3640 §4.1) into
/// `CodecConfig::Aac`, extracting `config` via [`fmtp_param`] and delegating
/// to [`aac_config_from_asc_hex`].
pub fn aac_config_from_fmtp(fmtp: &str) -> Result<CodecConfig> {
    let config_hex =
        fmtp_param(fmtp, "config").ok_or(Error::InvalidInput("fmtp has no config parameter"))?;
    aac_config_from_asc_hex(config_hex)
}

/// Parse an SDP `a=rtpmap:<pt> <encoding name>/<clock rate>[/<encoding
/// parameters>]` value and return the clock rate.
///
/// Handles the leading `<pt> ` payload-type token before the encoding name;
/// the optional `/<encoding parameters>` suffix (e.g. channel count) is
/// ignored here. Returns `None` on any malformed input (missing `/`,
/// non-numeric clock rate, or a value that doesn't fit `u32`) rather than
/// panicking.
pub fn rtpmap_clock_rate(rtpmap: &str) -> Option<u32> {
    let encoding = strip_leading_pt_token(rtpmap);
    // "<encoding name>/<clock rate>[/<encoding parameters>]" — the clock
    // rate is always the second '/'-separated field; a missing '/' leaves
    // no second field, so this returns `None` rather than misreading the
    // encoding name as a rate.
    let mut fields = encoding.split('/');
    let _name = fields.next()?;
    let clock_str = fields.next()?;
    clock_str.trim().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp::base64_encode;

    #[test]
    fn sprop_round_trips_sps_pps_and_profile() {
        // A minimal but real SPS (type 7) and PPS (type 8).
        // SPS bytes after the NAL header byte: profile_idc, constraints, level_idc.
        let sps = alloc::vec![0x67u8, 0x42, 0xC0, 0x1E, 0xAB]; // profile 0x42, level 0x1E
        let pps = alloc::vec![0x68u8, 0xCE, 0x3C, 0x80];
        let sprop = alloc::format!("{},{}", base64_encode(&sps), base64_encode(&pps));

        let boxed = avc_config_from_sprop(&sprop).unwrap();
        let r = &boxed.config;
        assert_eq!(r.sps.len(), 1);
        assert_eq!(r.pps.len(), 1);
        assert_eq!(r.sps[0].0, sps);
        assert_eq!(r.pps[0].0, pps);
        assert_eq!(r.profile_indication, 0x42);
        assert_eq!(r.profile_compatibility, 0xC0);
        assert_eq!(r.level_indication, 0x1E);
        assert_eq!(r.length_size_minus_one, 3);
    }

    #[test]
    fn sprop_rejects_when_no_sps() {
        // Only a PPS (type 8) present → no SPS → error.
        let pps = alloc::vec![0x68u8, 0xCE];
        let sprop = base64_encode(&pps);
        assert!(avc_config_from_sprop(&sprop).is_err());
    }

    #[test]
    fn sprop_rejects_invalid_base64() {
        // Invalid base64 token → returns Err, not panic.
        let sprop = "not-valid-base64!!!";
        assert!(avc_config_from_sprop(sprop).is_err());
    }

    #[test]
    fn sprop_rejects_sps_too_short() {
        // An SPS shorter than 4 bytes (need bytes [1..4] for profile/level).
        let sps = alloc::vec![0x67u8, 0x42]; // type 7 (SPS), but only 2 bytes
        let sprop = base64_encode(&sps);
        let result = avc_config_from_sprop(&sprop);
        assert!(result.is_err());
        // Verify it's a BufferTooShort error.
        if let Err(Error::BufferTooShort { need, have, what }) = result {
            assert_eq!(need, 4);
            assert_eq!(have, 2);
            assert!(what.contains("SPS"));
        }
    }

    #[test]
    fn sprop_rejects_non_sps_non_pps_nal() {
        // A non-SPS/non-PPS NAL (e.g., SEI type 6) → returns Err.
        let sei = alloc::vec![0x06u8, 0x00]; // type 6 (SEI), not SPS or PPS
        let sprop = base64_encode(&sei);
        let result = avc_config_from_sprop(&sprop);
        assert!(result.is_err());
    }

    #[test]
    fn aac_asc_hex_recovers_rate_channels_and_asc() {
        // AudioSpecificConfig for AAC-LC, 44100 Hz (freq index 4), stereo (2ch):
        // audioObjectType=2 (5 bits), samplingFreqIndex=4 (4 bits),
        // channelConfig=2 (4 bits) => bits: 00010 0100 0010 000 = 0x12 0x10
        let config_hex = "1210";
        let cfg = aac_config_from_asc_hex(config_hex).unwrap();
        match cfg {
            crate::pipeline::CodecConfig::Aac {
                sample_rate,
                channel_count,
                esds,
                ..
            } => {
                assert_eq!(sample_rate, 44100);
                assert_eq!(channel_count, 2);
                // The ASC bytes must survive into the esds decoder-specific info.
                let dsi = esds
                    .es_descriptor
                    .decoder_config
                    .as_ref()
                    .unwrap()
                    .decoder_specific_info
                    .as_ref()
                    .unwrap();
                assert_eq!(dsi.data, alloc::vec![0x12u8, 0x10]);
            }
            _ => panic!("expected CodecConfig::Aac"),
        }
    }

    #[test]
    fn fmtp_param_matches_key_anchored() {
        let fmtp =
            "96 packetization-mode=1; sprop-parameter-sets=Zm9v,YmFy; profile-level-id=42e01e";
        assert_eq!(fmtp_param(fmtp, "sprop-parameter-sets"), Some("Zm9v,YmFy"));
        assert_eq!(fmtp_param(fmtp, "profile-level-id"), Some("42e01e"));
        // "mode" is a suffix of "packetization-mode" but must NOT false-match.
        assert_eq!(fmtp_param(fmtp, "mode"), None);
    }

    #[test]
    fn fmtp_param_trims_whitespace() {
        let fmtp = "97   streamtype = 5 ;  config =1210  ;sizeLength=13";
        assert_eq!(fmtp_param(fmtp, "config"), Some("1210"));
        assert_eq!(fmtp_param(fmtp, "streamtype"), Some("5"));
        assert_eq!(fmtp_param(fmtp, "sizeLength"), Some("13"));
    }

    #[test]
    fn fmtp_param_skips_pair_without_equals() {
        // A malformed/bare-flag segment before the target key must not
        // abort the scan of later, well-formed pairs.
        let fmtp = "96 bareflag; config=1210";
        assert_eq!(fmtp_param(fmtp, "config"), Some("1210"));
    }

    #[test]
    fn fmtp_param_charset_preserves_multibyte() {
        // A parameter value with a multibyte UTF-8 char must round-trip
        // verbatim, proving the parser never slices on a raw byte offset.
        let fmtp = "96 sprop-description=caf\u{e9}; other=1";
        assert_eq!(fmtp_param(fmtp, "sprop-description"), Some("caf\u{e9}"));
    }

    #[test]
    fn avc_config_from_fmtp_extracts_sprop() {
        let fmtp =
            "96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
        let boxed = avc_config_from_fmtp(fmtp).unwrap();
        assert!(!boxed.config.sps.is_empty());
        assert!(!boxed.config.pps.is_empty());
    }

    #[test]
    fn aac_config_from_fmtp_extracts_config() {
        let fmtp = "97 streamtype=5; mode=AAC-hbr; config=1210; sizeLength=13";
        let cfg = aac_config_from_fmtp(fmtp).unwrap();
        match cfg {
            crate::pipeline::CodecConfig::Aac {
                sample_rate,
                channel_count,
                ..
            } => {
                assert_eq!(sample_rate, 44100);
                assert_eq!(channel_count, 2);
            }
            _ => panic!("expected CodecConfig::Aac"),
        }
    }

    #[test]
    fn aac_config_from_fmtp_missing_config_errors() {
        let fmtp = "97 streamtype=5; mode=AAC-hbr";
        assert!(aac_config_from_fmtp(fmtp).is_err());
    }

    #[test]
    fn avc_config_from_fmtp_missing_sprop_errors() {
        let fmtp = "96 packetization-mode=1";
        assert!(avc_config_from_fmtp(fmtp).is_err());
    }

    #[test]
    fn rtpmap_clock_rate_parses() {
        assert_eq!(rtpmap_clock_rate("96 H264/90000"), Some(90000));
        assert_eq!(rtpmap_clock_rate("97 mpeg4-generic/48000/2"), Some(48000));
        assert_eq!(rtpmap_clock_rate("H264/90000"), Some(90000));
        assert_eq!(rtpmap_clock_rate("malformed"), None);
        assert_eq!(rtpmap_clock_rate("96 H264/notanumber"), None);
        assert_eq!(rtpmap_clock_rate(""), None);
    }
}
