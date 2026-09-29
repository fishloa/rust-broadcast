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

use crate::aac_asc::AudioSpecificConfig;
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
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
/// AAC sample size is always 16 bits in the sample entry (fMP4/CMAF convention).
const AAC_SAMPLE_SIZE_BITS: u16 = 16;

/// Sample rate from the ASC: the explicit escape value if present, otherwise
/// the ISO/IEC 14496-3 Table 1.10 frequency for the index — taken from the
/// crate's single copy of that table
/// ([`SamplingFrequencyIndex::table_hz`], in `aac_asc`), never re-listed here.
fn asc_sample_rate(asc: &AudioSpecificConfig) -> Result<u32> {
    if let Some(freq) = asc.sampling_frequency {
        return Ok(freq);
    }
    asc.sampling_frequency_index
        .table_hz()
        .ok_or(Error::InvalidValue {
            field: "sampling_frequency_index",
            value: u64::from(asc.sampling_frequency_index.raw()),
            reason: "no frequency for index",
        })
}

/// Parse an SDP AAC `config` fmtp value (RFC 3640 §4.1: hex-encoded
/// `AudioSpecificConfig`) into `CodecConfig::Aac`, recovering sample rate and
/// channel count from the ASC and carrying the ASC bytes in the `esds`.
///
/// The channel count comes from ISO/IEC 14496-3 Table 1.19 — the same
/// `ChannelConfiguration::channel_count` mapping the FLV demuxers use, with 0
/// marking "not derived" — **not** from the raw `channelConfiguration` field:
/// configuration 7 is 8 channels (7.1), not 7, and configuration 0 means the
/// mapping is carried in-band by a `program_config_element` in the raw data
/// stream, so the count is not known from the ASC at all.
///
/// Takes the raw hex VALUE of the `config` parameter (not the full `a=fmtp`
/// line) — for the full-line entry point see [`aac_config_from_fmtp`]. Hex
/// decoding delegates to [`aac_config_from_asc_bytes`], the byte-level
/// building block a non-hex caller (e.g. `smooth_parse`'s Smooth
/// `CodecPrivateData`, already decoded to bytes) can use directly.
pub fn aac_config_from_asc_hex(config_hex: &str) -> Result<CodecConfig> {
    let asc_bytes = hex_decode(config_hex)?;
    aac_config_from_asc_bytes(asc_bytes)
}

/// Build `CodecConfig::Aac` from an already-decoded `AudioSpecificConfig`
/// byte buffer (the value-level building block behind
/// [`aac_config_from_asc_hex`]), recovering sample rate and channel count
/// from the ASC and carrying the ASC bytes verbatim in the `esds`.
pub fn aac_config_from_asc_bytes(asc_bytes: Vec<u8>) -> Result<CodecConfig> {
    let asc = AudioSpecificConfig::parse(&asc_bytes)?;
    let sample_rate = asc_sample_rate(&asc)?;
    let channel_count = crate::flv::aac_channel_count(&asc);

    let esds = EsdsBox::new(ESDescriptor::new(
        0,
        0,
        Some(DecoderConfigDescriptor::new(
            OTI_AUDIO_ISO14496_3,
            STREAM_TYPE_AUDIO,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(asc_bytes)),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));

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
/// NAL header). At least one SPS is required. Base64-decodes and classifies
/// each token, then delegates to [`avc_config_from_sps_pps`], the byte-level
/// building block a non-base64 caller (e.g. `smooth_parse`'s Annex B
/// `CodecPrivateData`) can use directly.
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
    avc_config_from_sps_pps(sps, pps)
}

/// Build an `avcC` configuration box from already-classified SPS/PPS NAL
/// units (the value-level building block behind [`avc_config_from_sprop`]).
///
/// `profile_indication`/`profile_compatibility`/`level_indication` are taken
/// from the first SPS's bytes `[1..4]` (after the NAL header) — at least one
/// SPS is required.
///
/// For a High profile SPS the record's `chroma_format`/
/// `bit_depth_luma_minus8`/`bit_depth_chroma_minus8` trailer is filled in
/// from the SPS itself: ISO/IEC 14496-15 §5.3.3.1.2 makes those fields
/// **conditionally present** on `AVCProfileIndication ∈ {100, 110, 122, 244}`,
/// so an `avcC` built for a High profile stream without them is a
/// configuration record that a strict reader parses differently from the wire
/// (and an encoder's own SPS carries the values — `decode_avc_sps` already
/// decodes them). RTSP cameras are overwhelmingly High profile
/// (§8.1's `profile-level-id` examples notwithstanding), so every avcC built
/// from their SDP lacked the trailer before this. A non-High profile keeps
/// `None` fields (the trailer is absent from the wire).
pub fn avc_config_from_sps_pps(sps: Vec<AvcSps>, pps: Vec<AvcPps>) -> Result<AVCConfigurationBox> {
    let first_sps = sps
        .first()
        .ok_or(Error::InvalidInput("no SPS NAL unit supplied"))?;
    if first_sps.0.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: first_sps.0.len(),
            what: "SPS profile/level bytes",
        });
    }
    let profile_indication = first_sps.0[1];
    // Gate on the serializer's own emission set (the ISO/IEC 14496-15
    // §5.3.3.1.2 condition for `profile_idc` 100/110/122/244), reached through
    // the shared `sps::is_high_profile` source of truth so the record and the
    // wire can never disagree about the trailer's presence. The SPS is decoded
    // rather than ignored on failure: it *is* the only place those three
    // values exist, so a decode error means the trailer cannot be written, and
    // writing the record without it would produce a configuration that a
    // strict reader parses differently from the stream (the defect the gate
    // exists to prevent). `is_high_profile` is the wider H.264 Table A-1 list
    // that governs *parsing* the SPS syntax; the trailer's own condition is
    // the narrower §5.3.3.1.2 set, so a profile in the wider list but outside
    // the trailer set keeps `None` without decoding anything.
    let ext = if crate::avc_config::AVCDecoderConfigurationRecord::has_high_profile_ext(
        profile_indication,
    ) {
        Some(crate::sps::decode_avc_sps(&first_sps.0)?)
    } else {
        None
    };
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication,
        profile_compatibility: first_sps.0[2],
        level_indication: first_sps.0[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps,
        pps,
        chroma_format: ext.as_ref().map(|i| i.chroma_format_idc),
        bit_depth_luma_minus8: ext.as_ref().map(|i| i.bit_depth_luma.saturating_sub(8)),
        bit_depth_chroma_minus8: ext.as_ref().map(|i| i.bit_depth_chroma.saturating_sub(8)),
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
    use broadcast_common::Serialize;

    /// The ffmpeg-produced SDP for `fixtures/ts/h264/high.ts` (PROVENANCE:
    /// `ffmpeg -i fixtures/ts/h264/high.ts -c copy -f rtp -sdp_file high.sdp
    /// rtp://127.0.0.1:5999`, ffmpeg 8.1) — a real High profile (profile_idc
    /// 0x64 = 100) camera-style stream.
    const HIGH_SDP_FMTP: &str = "packetization-mode=1; sprop-parameter-sets=Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA==,aOvjyyLA; profile-level-id=64000D";

    /// The `avcC` ffmpeg itself wrote for the same stream
    /// (`-c copy out.mp4`; the box's bytes after its 8-byte header):
    /// `01 64 00 0d ff e1 0019 <sps> 01 0004 <pps> fd f8 f8 00`, whose
    /// trailing `fd f8 f8 00` is ISO/IEC 14496-15 §5.3.3.1.2's High profile
    /// extension: chroma_format 1 (4:2:0), bit depths minus 8 of 0, and no
    /// SPS-ext NALs.
    ///
    /// Bites: pre-fix this function always wrote `chroma_format: None`, so
    /// the record stopped after the PPS and a strict reader — or ffmpeg's own
    /// `avcC` — disagreed with what the stream actually is.
    #[test]
    fn high_profile_sprop_gets_the_chroma_bit_depth_trailer() {
        let config = avc_config_from_fmtp(HIGH_SDP_FMTP).expect("real ffmpeg High SDP");
        let r = &config.config;
        assert_eq!(r.profile_indication, 0x64, "High profile, per the SDP");
        assert_eq!(r.level_indication, 0x0D, "level 1.3, per the SDP");
        assert_eq!(
            r.chroma_format,
            Some(1),
            "4:2:0 — the profile_idc-100 trailer must be present"
        );
        assert_eq!(r.bit_depth_luma_minus8, Some(0), "8-bit luma");
        assert_eq!(r.bit_depth_chroma_minus8, Some(0), "8-bit chroma");

        // Ground truth: ffmpeg's own avcC for the same stream carries exactly
        // this trailer, so the record round-trips against a real muxer.
        let mut out = alloc::vec![0u8; r.serialized_len()];
        r.serialize_into(&mut out).expect("serialize avcC");
        let sps_len = r.sps[0].0.len();
        let pps_len = r.pps[0].0.len();
        let trailer_start = out.len() - 4;
        assert_eq!(
            &out[trailer_start..],
            &[0xFD, 0xF8, 0xF8, 0x00],
            "the serialized trailer must match ffmpeg's own avcC bytes              (chroma 1, depths 0, no SPS-ext); sps/pps lens {sps_len}/{pps_len}"
        );
    }

    /// A non-High profile must **not** gain the trailer: §5.3.3.1.2 makes it
    /// conditional, so writing it for Baseline/Main would describe a record
    /// the wire does not have (and a strict reader would mis-parse the SPS
    /// count that follows).
    #[test]
    fn non_high_profile_sprop_has_no_trailer() {
        // ffmpeg's SDP for `fixtures/ts/h264/baseline.ts` (PROVENANCE: the
        // same `-c copy -f rtp -sdp_file` invocation as the High one above):
        // a real Baseline (`profile_idc` 0x42) stream.
        const BASELINE: &str = "packetization-mode=1; sprop-parameter-sets=Z0LADdkBQfsBEAAAAwAQAAADAyDxQqSA,aMuDyyA=; profile-level-id=42C00D";
        let config = avc_config_from_fmtp(BASELINE).expect("real ffmpeg Baseline SDP");
        let r = &config.config;
        assert_eq!(r.profile_indication, 0x42, "Baseline, per the SDP");
        assert_eq!(
            r.chroma_format, None,
            "no trailer for a non-High profile_idc"
        );
        assert_eq!(r.bit_depth_luma_minus8, None);
        assert_eq!(r.bit_depth_chroma_minus8, None);
    }

    /// An AAC `config` whose `channelConfiguration` is 7 means **8** channels
    /// (7.1) per ISO/IEC 14496-3 Table 1.19, not 7 (audit r04-W33), and the
    /// undetermined configurations (0 = PCE in-band, 8..=15 reserved) yield
    /// the crate's "not derived" marker rather than a fabricated count.
    #[test]
    fn aac_channel_count_uses_table_1_19() {
        // 0x11B856E500: AAC-LC, samplingFrequencyIndex 3 (48000 Hz),
        // channelConfiguration 7 — the ASC `fixtures/flv/aac-7_1.flv` carries
        // (ffmpeg `-f lavfi -i "anullsrc=r=48000:cl=7.1" -c:a aac -f flv`).
        let cfg = aac_config_from_asc_hex("11B856E500").expect("real 7.1 ASC");
        match cfg {
            CodecConfig::Aac {
                channel_count,
                sample_rate,
                ..
            } => {
                assert_eq!(sample_rate, 48_000, "sfi 3 = 48000 Hz");
                assert_eq!(channel_count, 8, "Table 1.19: configuration 7 = 8 ch");
            }
            other => panic!("expected AAC, got {other:?}"),
        }

        // channelConfiguration 0 (PCE in-band) and a reserved value 12: no
        // count is derivable from the ASC, so the marker is used instead.
        for asc_hex in ["1180", "11E0"] {
            let cfg = aac_config_from_asc_hex(asc_hex).expect("parseable ASC");
            match cfg {
                CodecConfig::Aac { channel_count, .. } => assert_eq!(
                    channel_count,
                    crate::flv::AAC_CHANNEL_COUNT_UNKNOWN,
                    "ASC {asc_hex}: an undetermined channel count is marked, not fabricated"
                ),
                other => panic!("expected AAC, got {other:?}"),
            }
        }
    }

    /// A High profile SPS whose body cannot be decoded must be an **error**,
    /// not a record silently missing §5.3.3.1.2's trailer: the trailer's three
    /// values exist only in the SPS, and a record written without them is one
    /// a strict reader parses differently from the stream.
    #[test]
    fn high_profile_sps_that_cannot_be_decoded_is_an_error() {
        // A High profile SPS (profile_idc 100) truncated immediately after its
        // profile/level bytes: everything the *record* copies is present, so
        // the record could be built — but the chroma/bit-depth fields after it
        // cannot be read.
        let sps = alloc::vec![0x67u8, 100, 0x00, 0x1E];
        let pps = alloc::vec![0x68u8, 0xCE, 0x3C, 0x80];
        let err = avc_config_from_sps_pps(alloc::vec![AvcSps(sps)], alloc::vec![AvcPps(pps)])
            .expect_err("a High profile SPS with no decodable body must not yield a record");
        assert!(
            matches!(
                err,
                Error::BufferTooShort { .. } | Error::InvalidValue { .. }
            ),
            "must be a structured decode error, got {err:?}"
        );
    }

    /// The trailer's condition is the record's own §5.3.3.1.2 set, which
    /// `AVCDecoderConfigurationRecord::has_high_profile_ext` owns, and which is
    /// deliberately narrower than the `sps::is_high_profile` list used to
    /// *parse* SPS syntax: a profile in the wider list but outside the trailer
    /// set writes no trailer, and does not need the SPS decoded at all.
    #[test]
    fn trailer_gate_is_the_avcc_profile_set_not_the_sps_parse_set() {
        use crate::avc_config::AVCDecoderConfigurationRecord as Record;
        for idc in [100u8, 110, 122, 244] {
            assert!(
                Record::has_high_profile_ext(idc),
                "profile {idc} carries the trailer (ISO/IEC 14496-15 §5.3.3.1.2)"
            );
        }
        for idc in [66u8, 77, 88, 44, 83, 86, 118, 128, 138, 139, 134, 135] {
            assert!(
                !Record::has_high_profile_ext(idc),
                "profile {idc} is outside the record's trailer set"
            );
        }
        // 44/83/86/118/… are in the wider SPS-parse list but not the trailer
        // set, so the two sets genuinely differ.
        assert!(
            crate::sps::is_high_profile(44) && !Record::has_high_profile_ext(44),
            "the parse gate and the trailer gate must be different sets"
        );
    }

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
    fn avc_config_from_sps_pps_matches_sprop_path() {
        // The byte-level building block must produce the same box as the
        // base64/sprop entry point for the same underlying NAL bytes.
        let sps = alloc::vec![0x67u8, 0x42, 0xC0, 0x1E, 0xAB];
        let pps = alloc::vec![0x68u8, 0xCE, 0x3C, 0x80];
        let via_sprop = avc_config_from_sprop(&alloc::format!(
            "{},{}",
            base64_encode(&sps),
            base64_encode(&pps)
        ))
        .unwrap();
        let via_bytes =
            avc_config_from_sps_pps(alloc::vec![AvcSps(sps)], alloc::vec![AvcPps(pps)]).unwrap();
        assert_eq!(via_sprop.config.sps, via_bytes.config.sps);
        assert_eq!(via_sprop.config.pps, via_bytes.config.pps);
        assert_eq!(
            via_sprop.config.profile_indication,
            via_bytes.config.profile_indication
        );
    }

    #[test]
    fn avc_config_from_sps_pps_rejects_no_sps() {
        assert!(avc_config_from_sps_pps(Vec::new(), Vec::new()).is_err());
    }

    #[test]
    fn aac_config_from_asc_bytes_matches_hex_path() {
        let asc_bytes = alloc::vec![0x12u8, 0x10];
        let via_hex = aac_config_from_asc_hex("1210").unwrap();
        let via_bytes = aac_config_from_asc_bytes(asc_bytes).unwrap();
        match (via_hex, via_bytes) {
            (
                crate::pipeline::CodecConfig::Aac {
                    sample_rate: sr1,
                    channel_count: ch1,
                    ..
                },
                crate::pipeline::CodecConfig::Aac {
                    sample_rate: sr2,
                    channel_count: ch2,
                    ..
                },
            ) => {
                assert_eq!(sr1, sr2);
                assert_eq!(ch1, ch2);
            }
            _ => panic!("expected CodecConfig::Aac from both paths"),
        }
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
    fn fmtp_param_preserves_base64_padding() {
        // Split at the FIRST `=` only: a base64 value's trailing `==` padding
        // must survive verbatim, not be truncated at the padding `=`.
        let fmtp = "96 sprop-parameter-sets=Zm9v,YmFy==;x=1";
        assert_eq!(
            fmtp_param(fmtp, "sprop-parameter-sets"),
            Some("Zm9v,YmFy==")
        );
    }

    #[test]
    fn fmtp_param_empty_value_is_none() {
        // A present-but-empty parameter (`key=`) is treated as no-match.
        assert_eq!(fmtp_param("96 config=;x=1", "config"), None);
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
