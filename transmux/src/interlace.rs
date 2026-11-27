//! Video interlace facts for live-player consumers (issue #1162, R3), built
//! from a track's codec config and its coded samples.
//!
//! The coded facts themselves — the sequence scan mode, a picture's
//! `pic_struct`/`source_scan_type` and the frame/field structure of one access
//! unit — are [`broadcast_bitstream::interlace`]'s. This module joins them to
//! the container IR:
//!
//! 1. **Per track** — [`video_scan`] / [`CodecConfig::video_scan`]: how the
//!    SPS says the sequence is coded ([`VideoScan`]).
//! 2. **Per sample** — [`video_picture_facts`]: for one sample of a track (as
//!    [`StreamingTsDemux`](crate::StreamingTsDemux) emits it, i.e. a
//!    length-prefixed access unit) the coded structure of its picture
//!    ([`PictureFacts`]): frame vs field, first-field parity, and the
//!    `pic_struct` / `source_scan_type` of its picture timing SEI when the
//!    stream carries one.
//!
//! Why a free function over `(config, sample bytes)` rather than a field on
//! [`Sample`](crate::Sample): `Sample` is the hot, widely constructed IR
//! type and is shared with every demuxer and muxer; the facts need the
//! track's parameter sets anyway, are only wanted by display-side consumers,
//! and costing a slice-header parse per sample is not something every
//! consumer should pay. The function is additive and touches no existing API.
//!
//! Specs (transcribed in `private/specs-verified/mpeg/transmux/codec/h264-vui-pic-timing.md`,
//! `private/specs-verified/mpeg/transmux/codec/h265-vui-pic-timing.md`, `private/specs-verified/mpeg/transmux/codec/h264-slice-header.md`):
//! H.264 §7.3.2.1.1, §7.3.3, §D.1.3/D.2.3 + Table D-1, §E.1.1-E.1.2;
//! H.265 §7.3.7, §D.2.3/D.3.3 + Table D.2, §E.2.1-E.3.1.

use crate::ir::CodecConfig;
use broadcast_bitstream::annexb::iter_length_prefixed_nals_with;
use broadcast_bitstream::interlace::{PictureFacts, VideoScan, avc_facts, hevc_facts};
use broadcast_bitstream::nal::{AVC_NAL_PPS, AVC_NAL_SPS, NAL_TYPE_MASK, NalCodec, nal_unit_type};
use broadcast_bitstream::slice_header::AvcParamSets;
use broadcast_bitstream::sps::{decode_avc_slice_sps, decode_hevc_interlace_sps};
use isobmff::hevc_config::HEVCDecoderConfigurationRecord;
use isobmff::nalu_types::HEVC_SPS_NUT;

impl CodecConfig {
    /// How this track's video is coded, from its SPS ([`video_scan`]); `None`
    /// for non-AVC/HEVC configs or an unparseable SPS.
    pub fn video_scan(&self) -> Option<VideoScan> {
        video_scan(self)
    }
}

/// The first SPS NAL of an `hvcC` record.
fn hevc_config_sps(rec: &HEVCDecoderConfigurationRecord) -> Option<&[u8]> {
    rec.arrays
        .iter()
        .filter(|a| a.nal_unit_type == HEVC_SPS_NUT)
        .flat_map(|a| a.nalus.iter())
        .next()
        .map(|n| n.0.as_slice())
}

/// The first SPS NAL of an AVC/HEVC config.
fn first_sps(config: &CodecConfig) -> Option<&[u8]> {
    match config {
        CodecConfig::Avc { config, .. } => config.config.sps.first().map(|s| s.0.as_slice()),
        CodecConfig::Hevc { config, .. } => hevc_config_sps(&config.config),
        _ => None,
    }
}

/// How the track's SPS says its video is coded ([`VideoScan`]); `None` for a
/// non-AVC/HEVC config, or when the SPS does not parse.
pub fn video_scan(config: &CodecConfig) -> Option<VideoScan> {
    let sps = first_sps(config)?;
    match config {
        CodecConfig::Avc { .. } => {
            let s = decode_avc_slice_sps(sps).ok()?;
            Some(if s.frame_mbs_only() {
                VideoScan::Progressive
            } else if s.mb_adaptive_frame_field() {
                VideoScan::Mbaff
            } else {
                VideoScan::Paff
            })
        }
        CodecConfig::Hevc { .. } => {
            let s = decode_hevc_interlace_sps(sps).ok()?;
            Some(if s.field_seq() {
                VideoScan::FieldSequence
            } else {
                VideoScan::Progressive
            })
        }
        _ => None,
    }
}

/// The coded interlace structure of one sample of an AVC/HEVC track.
///
/// `sample` is the length-prefixed access unit exactly as the demuxers emit it
/// (the prefix width comes from the config's `lengthSizeMinusOne`). Parameter
/// sets are taken from `config` and from any in-band SPS/PPS in the sample.
/// Returns `None` for a non-AVC/HEVC config, a malformed sample, a sample with
/// no parseable picture (e.g. an H.264 slice whose SPS/PPS is unknown), or an
/// HEVC track whose SPS does not parse.
pub fn video_picture_facts(config: &CodecConfig, sample: &[u8]) -> Option<PictureFacts> {
    match config {
        CodecConfig::Avc { config, .. } => {
            let rec = &config.config;
            let nals =
                iter_length_prefixed_nals_with(sample, usize::from(rec.length_size_minus_one) + 1)
                    .ok()?;
            let mut ps = AvcParamSets::new();
            for sps in &rec.sps {
                ps.observe_nal(&sps.0);
            }
            for pps in &rec.pps {
                ps.observe_nal(&pps.0);
            }
            for nal in &nals {
                if matches!(
                    nal.first().map(|b| b & NAL_TYPE_MASK),
                    Some(AVC_NAL_SPS | AVC_NAL_PPS)
                ) {
                    ps.observe_nal(nal);
                }
            }
            avc_facts(&ps, &nals)
        }
        CodecConfig::Hevc { config, .. } => {
            let rec = &config.config;
            let nals =
                iter_length_prefixed_nals_with(sample, usize::from(rec.length_size_minus_one) + 1)
                    .ok()?;
            let mut sps_nal = hevc_config_sps(rec)?;
            // An in-band SPS supersedes the config's.
            for nal in &nals {
                if nal_unit_type(NalCodec::Hevc, nal) == Some(HEVC_SPS_NUT) {
                    sps_nal = nal;
                }
            }
            hevc_facts(sps_nal, &nals)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CodecConfig;
    use alloc::vec;
    use alloc::vec::Vec;
    use broadcast_bitstream::interlace::{PicStruct, SourceScanType};
    use isobmff::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
    use isobmff::nalu_types::{HevcNalArray, HevcNalUnit};
    use test_fixtures::bits::BitWriter;

    fn hevc_nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![nal_type << 1, 0x01];
        v.extend_from_slice(body);
        v
    }

    /// An HEVC SPS whose VUI carries `frame_field_info_present_flag = 1` and
    /// `field_seq_flag = 0` (H.265 §7.3.2.3 / §E.2.1), written field by field.
    fn hevc_sps_frame_field_info() -> Vec<u8> {
        let mut nal = vec![0x42u8, 0x01];
        let mut s = BitWriter::new();
        s.u(4, 0); // sps_video_parameter_set_id
        s.u(3, 0); // sps_max_sub_layers_minus1
        s.bit(true); // sps_temporal_id_nesting_flag
        s.u(2, 0); // general_profile_space
        s.bit(false); // general_tier_flag
        s.u(5, 1); // general_profile_idc
        s.u(32, 0); // general_profile_compatibility_flags
        s.u(48, 0); // general_constraint_indicator_flags
        s.u(8, 93); // general_level_idc
        s.ue(0); // sps_seq_parameter_set_id
        s.ue(1); // chroma_format_idc
        s.ue(16); // pic_width_in_luma_samples
        s.ue(16); // pic_height_in_luma_samples
        s.bit(false); // conformance_window_flag
        s.ue(0); // bit_depth_luma_minus8
        s.ue(0); // bit_depth_chroma_minus8
        s.ue(3); // log2_max_pic_order_cnt_lsb_minus4
        s.bit(false); // sps_sub_layer_ordering_info_present_flag
        s.ue(0); // sps_max_dec_pic_buffering_minus1
        s.ue(0); // sps_max_num_reorder_pics
        s.ue(0); // sps_max_latency_increase_plus1
        s.ue(0); // log2_min_luma_coding_block_size_minus3
        s.ue(3); // log2_diff_max_min_luma_coding_block_size
        s.ue(0); // log2_min_luma_transform_block_size_minus2
        s.ue(3); // log2_diff_max_min_luma_transform_block_size
        s.ue(1); // max_transform_hierarchy_depth_inter
        s.ue(1); // max_transform_hierarchy_depth_intra
        s.bit(false); // scaling_list_enabled_flag
        s.bit(false); // amp_enabled_flag
        s.bit(false); // sample_adaptive_offset_enabled_flag
        s.bit(false); // pcm_enabled_flag
        s.ue(3); // num_short_term_ref_pic_sets
        // set 0: explicit, DeltaPocS0 = [-1], DeltaPocS1 = [+2]
        s.ue(1);
        s.ue(1);
        s.ue(0);
        s.bit(true);
        s.ue(1);
        s.bit(true);
        // set 1: inter from set 0
        s.bit(true);
        s.bit(true);
        s.ue(2);
        s.bit(true);
        s.bit(false);
        s.bit(false);
        s.bit(true);
        // set 2: inter from set 1
        s.bit(true);
        s.bit(false);
        s.ue(0);
        for _ in 0..3 {
            s.bit(true);
        }
        s.bit(false); // long_term_ref_pics_present_flag
        s.bit(false); // sps_temporal_mvp_enabled_flag
        s.bit(false); // strong_intra_smoothing_enabled_flag
        s.bit(true); // vui_parameters_present_flag
        s.bit(false); // aspect_ratio_info_present_flag
        s.bit(false); // overscan_info_present_flag
        s.bit(false); // video_signal_type_present_flag
        s.bit(false); // chroma_loc_info_present_flag
        s.bit(false); // neutral_chroma_indication_flag
        s.bit(false); // field_seq_flag
        s.bit(true); // frame_field_info_present_flag
        s.bit(false); // default_display_window_flag
        s.bit(false); // vui_timing_info_present_flag
        s.bit(false); // bitstream_restriction_flag
        s.bit(true); // rbsp_stop_one_bit
        nal.extend(s.finish());
        nal
    }

    /// A prefix SEI NAL carrying a `pic_timing` (§D.2.3) with the given
    /// `pic_struct` and `source_scan_type`.
    fn hevc_pic_timing_sei(pic_struct: u64, source_scan_type: u64) -> Vec<u8> {
        let mut p = BitWriter::new();
        p.u(4, pic_struct);
        p.u(2, source_scan_type);
        p.bit(false); // duplicate_flag
        let payload = p.finish();
        let mut body = vec![1u8, payload.len() as u8];
        body.extend_from_slice(&payload);
        body.push(0x80); // rbsp_trailing_bits
        hevc_nal(39, &body)
    }

    /// An hvcC carrying a VPS array before the SPS array (the order in a real
    /// record) and 4-byte NAL length prefixes.
    fn hevc_config() -> CodecConfig {
        let record = HEVCDecoderConfigurationRecord {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0,
            general_constraint_indicator_flags: 0,
            general_level_idc: 93,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            length_size_minus_one: 3,
            arrays: vec![
                HevcNalArray::new(
                    true,
                    32,
                    vec![HevcNalUnit::new(hevc_nal(32, &[0x0C, 0x01]))],
                ),
                HevcNalArray::new(
                    true,
                    33,
                    vec![HevcNalUnit::new(hevc_sps_frame_field_info())],
                ),
            ],
        };
        CodecConfig::Hevc {
            config: HEVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        }
    }

    fn access_unit(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        out
    }

    #[test]
    fn hevc_facts_come_from_the_config_sps_and_prefix_sei() {
        let config = hevc_config();

        // A sample with only a VPS (nal_unit_type 32) is not a picture.
        let vps = hevc_nal(32, &[0x0C, 0x01]);
        assert_eq!(video_picture_facts(&config, &access_unit(&[vps])), None);

        // A 4-byte length prefix (lengthSizeMinusOne 3) over a VCL NAL and the
        // pic_timing prefix SEI. The VCL NAL is longer than 64 KiB, so reading
        // the prefixes with a 2-byte width desynchronizes the whole sample.
        let mut vcl = vec![0x02, 0x01];
        vcl.resize(65_536, 0xFF);
        let sei = hevc_pic_timing_sei(3, 0);
        let facts = video_picture_facts(&config, &access_unit(&[vcl, sei]))
            .expect("VCL + pic_timing SEI yields facts");
        assert!(!facts.field_pic());
        assert_eq!(facts.fields(), 0);
        assert_eq!(facts.pic_struct(), Some(PicStruct::TopBottom));
        assert_eq!(facts.bottom_field_first(), Some(false));
        assert_eq!(facts.source_scan_type(), Some(SourceScanType::Interlaced));
    }

    #[test]
    fn hevc_only_prefix_sei_nals_are_read_as_sei() {
        let config = hevc_config();
        // A VCL NAL whose body happens to look like a pic_timing message
        // (payloadType 1, payload 0x20 = pic_struct 2) must not be read as SEI.
        let disguised_vcl = hevc_nal(1, &[0x01, 0x01, 0x20, 0x80]);
        let sei = hevc_pic_timing_sei(3, 0);
        let facts = video_picture_facts(&config, &access_unit(&[disguised_vcl, sei.clone()]))
            .expect("the real prefix SEI still decides");
        assert_eq!(facts.pic_struct(), Some(PicStruct::TopBottom));

        // A header-only (2-byte) prefix SEI NAL carries no message and is skipped.
        let vcl = hevc_nal(1, &[0x80]);
        let empty_sei = hevc_nal(39, &[]);
        let facts = video_picture_facts(&config, &access_unit(&[vcl, empty_sei, sei]))
            .expect("the real prefix SEI still decides");
        assert_eq!(facts.pic_struct(), Some(PicStruct::TopBottom));
    }
}
