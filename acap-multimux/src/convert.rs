//! Pure VDO Annex-B access unit -> transmux IR sample conversion.
//!
//! This module has **no `vdo` dependency** — it turns an Annex B access unit
//! (ITU-T H.264 Annex B / ITU-T H.265 Annex B, both ISO/IEC 14496-15 §5.3.4
//! byte-stream framing) plus a duration/sync flag into a
//! [`transmux::pipeline::Sample`], and extracts the in-band parameter sets
//! (SPS/PPS for H.264; VPS/SPS/PPS for H.265) needed to build the
//! [`transmux::pipeline::TrackSpec`] (`avcC`/`hvcC`) for the CMAF init
//! segment. `vdo_source` (device-gated) is the only caller that knows about
//! VDO; this module is host-buildable and host-testable (see
//! `tests/convert_synthetic.rs`).

use transmux::annexb::iter_annexb_nals;
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use transmux::nalu_types::{AvcPps, AvcSps, HevcNalArray, HevcNalUnit};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::sps::{decode_avc_sps, decode_hevc_sps};

use crate::Result;
use crate::error::AcapError;

// --- H.264 / AVC NAL types (ITU-T H.264 §7.3.1, Table 7-1) ------------------

/// Mask for the H.264 5-bit `nal_unit_type` in header byte 0 (bits `[4:0]`).
const H264_NAL_TYPE_MASK: u8 = 0x1F;
/// H.264 `nal_unit_type` for a sequence parameter set (SPS).
const H264_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for a picture parameter set (PPS).
const H264_NAL_PPS: u8 = 8;

// --- H.265 / HEVC NAL types (ITU-T H.265 §7.3.1.2, Table 7-1) ---------------

/// Right shift to reach the HEVC 6-bit `nal_unit_type` in header byte 0.
const H265_NAL_TYPE_SHIFT: u8 = 1;
/// Mask for the HEVC 6-bit `nal_unit_type` after the shift.
const H265_NAL_TYPE_MASK: u8 = 0x3F;
/// H.265 `nal_unit_type` for a video parameter set (VPS).
const H265_NAL_VPS: u8 = 32;
/// H.265 `nal_unit_type` for a sequence parameter set (SPS).
const H265_NAL_SPS: u8 = 33;
/// H.265 `nal_unit_type` for a picture parameter set (PPS).
const H265_NAL_PPS: u8 = 34;
/// Minimum NAL length (bytes) to read an HEVC 2-byte `nal_unit_header()`.
const H265_NAL_HEADER_LEN: usize = 2;

// --- avcC / hvcC construction constants -------------------------------------

/// `AVCDecoderConfigurationRecord.configurationVersion` — shall be 1
/// (ISO/IEC 14496-15:2017 §5.3.3).
const AVCC_CONFIGURATION_VERSION: u8 = 1;
/// `HEVCDecoderConfigurationRecord.configurationVersion` — shall be 1
/// (ISO/IEC 14496-15:2017 §8.3.3).
const HVCC_CONFIGURATION_VERSION: u8 = 1;
/// `lengthSizeMinusOne = 3` -> 4-byte NAL length prefix, matching
/// [`transmux::annexb::NAL_LENGTH_SIZE`].
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// `min_spatial_segmentation_idc` unspecified (hvcC field not derivable from
/// the SPS alone without full tile/slice analysis).
const HVCC_MIN_SPATIAL_SEGMENTATION_UNSPEC: u16 = 0;
/// `parallelismType` unknown/mixed.
const HVCC_PARALLELISM_TYPE_UNKNOWN: u8 = 0;
/// `avgFrameRate` unspecified.
const HVCC_AVG_FRAME_RATE_UNSPEC: u16 = 0;
/// `constantFrameRate` not indicated.
const HVCC_CONSTANT_FRAME_RATE_UNSPEC: u8 = 0;
/// `numTemporalLayers` — a single (base) temporal layer, the common case for
/// VDO's non-scalable encode.
const HVCC_NUM_TEMPORAL_LAYERS: u8 = 1;

/// Fallback coded dimension when the SPS can't be decoded for width/height
/// (e.g. a malformed/synthetic SPS) — `track_spec` still builds a usable
/// `avcC`/`hvcC` (the parameter sets are what the decoder actually needs);
/// only the auxiliary `CodecConfig` width/height fields are affected.
const UNKNOWN_DIMENSION: u16 = 0;

/// Microseconds per second, for converting a VDO µs timestamp delta into
/// track-timescale ticks.
const MICROS_PER_SECOND: u64 = 1_000_000;

/// `Sample::from_annexb` composition time offset — VDO delivers samples in
/// decode order with no separate PTS/DTS, so `pts == dts` (offset 0).
const COMPOSITION_OFFSET_ZERO: i32 = 0;

/// The video codec family of a VDO stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// H.264 / AVC (ISO/IEC 14496-10).
    H264,
    /// H.265 / HEVC (ISO/IEC 23008-2).
    H265,
}

/// Raw in-band parameter-set NAL units extracted from an access unit.
///
/// Each NAL is stored with its header byte(s) intact (as returned by
/// [`iter_annexb_nals`]) — the same form [`AvcSps`]/[`AvcPps`]/[`HevcNalUnit`]
/// expect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSets {
    /// Sequence parameter set NAL unit (H.264 type 7; H.265 type 33).
    pub sps: Vec<u8>,
    /// Picture parameter set NAL unit (H.264 type 8; H.265 type 34).
    pub pps: Vec<u8>,
    /// Video parameter set NAL unit — H.265 only (type 32); always `None`
    /// for H.264, which has no VPS.
    pub vps: Option<Vec<u8>>,
}

/// Scan an Annex B access unit for the codec's in-band parameter sets.
///
/// Returns `None` unless every parameter set the codec requires to build a
/// conformant `avcC`/`hvcC` is present: SPS + PPS for H.264; VPS + SPS + PPS
/// for H.265. When a parameter set type repeats within the access unit, the
/// first occurrence is kept (an encoder that resends parameter sets mid-GOP
/// still describes the same stream).
pub fn extract_param_sets(codec: Codec, annexb_au: &[u8]) -> Option<ParamSets> {
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut vps: Option<Vec<u8>> = None;

    for nal in iter_annexb_nals(annexb_au) {
        match codec {
            Codec::H264 => {
                if nal.is_empty() {
                    continue;
                }
                match nal[0] & H264_NAL_TYPE_MASK {
                    H264_NAL_SPS if sps.is_none() => sps = Some(nal.to_vec()),
                    H264_NAL_PPS if pps.is_none() => pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            Codec::H265 => {
                if nal.len() < H265_NAL_HEADER_LEN {
                    continue;
                }
                match (nal[0] >> H265_NAL_TYPE_SHIFT) & H265_NAL_TYPE_MASK {
                    H265_NAL_VPS if vps.is_none() => vps = Some(nal.to_vec()),
                    H265_NAL_SPS if sps.is_none() => sps = Some(nal.to_vec()),
                    H265_NAL_PPS if pps.is_none() => pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
        }
    }

    match codec {
        Codec::H264 => Some(ParamSets {
            sps: sps?,
            pps: pps?,
            vps: None,
        }),
        Codec::H265 => Some(ParamSets {
            sps: sps?,
            pps: pps?,
            vps: Some(vps?),
        }),
    }
}

/// Build a [`TrackSpec`] (`avcC` for H.264 / `hvcC` for H.265) from the
/// extracted parameter sets.
///
/// H.264's `profile_indication`/`profile_compatibility`/`level_indication`
/// are read directly from SPS bytes `[1]`/`[2]`/`[3]` (ITU-T H.264 §7.3.2.1.1
/// — the same fixed-position scheme `transmux::rtp_sdp::avc_config_from_sprop`
/// uses), so a syntactically short SPS is the only H.264 failure mode.
/// Width/height (auxiliary `CodecConfig` fields) are best-effort: if the SPS
/// doesn't fully bit-decode, they fall back to `0` rather than failing the
/// whole track spec, since the decoder only needs the parameter sets
/// themselves.
///
/// H.265's profile/tier/level fields are bit-packed inside
/// `profile_tier_level()` (ITU-T H.265 §7.3.3), not byte-positioned like
/// AVC's `profile_idc` — there is no shortcut, so a full SPS bit-decode
/// ([`decode_hevc_sps`]) is required and its failure is propagated.
pub fn track_spec(
    codec: Codec,
    params: &ParamSets,
    track_id: u32,
    clock_rate: u32,
) -> Result<TrackSpec> {
    let config = match codec {
        Codec::H264 => avc_codec_config(params)?,
        Codec::H265 => hevc_codec_config(params)?,
    };
    Ok(TrackSpec::new(track_id, clock_rate, config))
}

fn avc_codec_config(params: &ParamSets) -> Result<CodecConfig> {
    if params.sps.len() < 4 {
        return Err(AcapError::Convert(format!(
            "H.264 SPS too short for profile/level bytes: need 4, have {}",
            params.sps.len()
        )));
    }

    let (width, height) = decode_avc_sps(&params.sps)
        .map(|info| (clamp_u16(info.width), clamp_u16(info.height)))
        .unwrap_or((UNKNOWN_DIMENSION, UNKNOWN_DIMENSION));

    let record = AVCDecoderConfigurationRecord {
        configuration_version: AVCC_CONFIGURATION_VERSION,
        profile_indication: params.sps[1],
        profile_compatibility: params.sps[2],
        level_indication: params.sps[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps: vec![AvcSps(params.sps.clone())],
        pps: vec![AvcPps(params.pps.clone())],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };

    Ok(CodecConfig::Avc {
        config: AVCConfigurationBox::new(record),
        width,
        height,
    })
}

fn hevc_codec_config(params: &ParamSets) -> Result<CodecConfig> {
    let vps = params
        .vps
        .as_ref()
        .ok_or_else(|| AcapError::Convert("H.265 track_spec requires a VPS".to_string()))?;

    let info = decode_hevc_sps(&params.sps)?;
    let width = clamp_u16(info.width);
    let height = clamp_u16(info.height);

    let arrays = vec![
        HevcNalArray::new(true, H265_NAL_VPS, vec![HevcNalUnit::new(vps.clone())]),
        HevcNalArray::new(
            true,
            H265_NAL_SPS,
            vec![HevcNalUnit::new(params.sps.clone())],
        ),
        HevcNalArray::new(
            true,
            H265_NAL_PPS,
            vec![HevcNalUnit::new(params.pps.clone())],
        ),
    ];

    let record = HEVCDecoderConfigurationRecord {
        configuration_version: HVCC_CONFIGURATION_VERSION,
        general_profile_space: info.general_profile_space,
        general_tier_flag: info.general_tier_flag,
        general_profile_idc: info.general_profile_idc,
        general_profile_compatibility_flags: info.general_profile_compatibility_flags,
        general_constraint_indicator_flags: info.general_constraint_indicator_flags,
        general_level_idc: info.general_level_idc,
        min_spatial_segmentation_idc: HVCC_MIN_SPATIAL_SEGMENTATION_UNSPEC,
        parallelism_type: HVCC_PARALLELISM_TYPE_UNKNOWN,
        chroma_format_idc: info.chroma_format_idc,
        // hvcC stores bit_depth_{luma,chroma}_minus8; decode_hevc_sps returns
        // the absolute bit depth (minus8 + 8) — subtract 8 back out
        // (saturating: a malformed SPS reporting < 8 would otherwise wrap).
        bit_depth_luma_minus8: info.bit_depth_luma.saturating_sub(8),
        bit_depth_chroma_minus8: info.bit_depth_chroma.saturating_sub(8),
        avg_frame_rate: HVCC_AVG_FRAME_RATE_UNSPEC,
        constant_frame_rate: HVCC_CONSTANT_FRAME_RATE_UNSPEC,
        num_temporal_layers: HVCC_NUM_TEMPORAL_LAYERS,
        temporal_id_nested: false,
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        arrays,
    };

    Ok(CodecConfig::Hevc {
        config: HEVCConfigurationBox::new(record),
        width,
        height,
    })
}

/// Clamp a decoded SPS dimension (luma samples, up to `u32`) into the `u16`
/// `CodecConfig` width/height fields.
fn clamp_u16(v: u32) -> u16 {
    v.min(u16::MAX as u32) as u16
}

/// Convert an Annex B access unit into a [`Sample`] (length-prefixed NALs via
/// [`Sample::from_annexb`]). `codec` is accepted for API symmetry with
/// [`extract_param_sets`]/[`track_spec`] — H.264 and H.265 access units both
/// use the same Annex B -> length-prefixed conversion, so it isn't needed to
/// select behaviour here.
pub fn au_to_sample(_codec: Codec, annexb_au: &[u8], duration_ticks: u32, is_sync: bool) -> Sample {
    Sample::from_annexb(annexb_au, duration_ticks, is_sync, COMPOSITION_OFFSET_ZERO)
}

/// Convert a VDO µs-timestamp delta into track-timescale ticks.
///
/// `clock_rate` is the track's media timescale (90000 for H.264/H.265 video).
/// Saturates rather than panicking on a backwards timestamp (`ts_us <
/// prev_ts_us`, e.g. the first sample or a VDO timestamp glitch) or on an
/// overflow into a value too large for `u32`.
pub fn duration_ticks(prev_ts_us: u64, ts_us: u64, clock_rate: u32) -> u32 {
    let delta_us = ts_us.saturating_sub(prev_ts_us);
    let ticks = delta_us.saturating_mul(u64::from(clock_rate)) / MICROS_PER_SECOND;
    ticks.min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: [u8; 5] = [0x67, 0x42, 0xC0, 0x1E, 0xAB];
    const PPS: [u8; 4] = [0x68, 0xCE, 0x3C, 0x80];

    #[test]
    fn avc_codec_config_rejects_short_sps() {
        let params = ParamSets {
            sps: vec![0x67, 0x42],
            pps: PPS.to_vec(),
            vps: None,
        };
        assert!(track_spec(Codec::H264, &params, 1, 90_000).is_err());
    }

    #[test]
    fn hevc_codec_config_requires_vps() {
        let params = ParamSets {
            sps: SPS.to_vec(),
            pps: PPS.to_vec(),
            vps: None,
        };
        assert!(track_spec(Codec::H265, &params, 1, 90_000).is_err());
    }

    #[test]
    fn clamp_u16_saturates() {
        assert_eq!(clamp_u16(u32::MAX), u16::MAX);
        assert_eq!(clamp_u16(1920), 1920);
    }
}
