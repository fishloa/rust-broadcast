//! SPS-dependent NAL helpers that stay in `transmux` until the parameter-set
//! layer moves to `broadcast-bitstream` (lane bs1's PR 1).
//!
//! These functions decode an SPS to answer a question about it, so they need
//! the parser that still lives in [`crate::sps`]. The NAL classification they
//! build on is [`broadcast_bitstream::nal`].

use broadcast_bitstream::annexb::iter_length_prefixed_nals;
use broadcast_bitstream::nal::{AVC_NAL_PPS, AVC_NAL_SPS, NAL_TYPE_MASK, NalCodec, nal_unit_type};
use isobmff::nalu_types::{HEVC_PPS_NUT, HEVC_SPS_NUT, HEVC_VPS_NUT};

/// Shortest AVC NAL that can carry a body: the 1-byte NAL header plus at
/// least one body byte.
const AVC_NAL_MIN_LEN: usize = 2;
/// Shortest HEVC NAL that can carry a parameter set: the 2-byte NAL header
/// plus at least one body byte.
const HEVC_NAL_MIN_LEN: usize = 2 + 1;

/// Output-rectangle `(width, height)` of an H.264 SPS NAL, clamped to `u16`.
///
/// This is the rule the TS demuxer applies to `TrackSpec` dimensions: the
/// luma size after frame cropping (`decode_avc_sps`), `None` if the SPS does
/// not decode.
pub fn avc_sps_dimensions(sps: &[u8]) -> Option<(u16, u16)> {
    let info = crate::sps::decode_avc_sps(sps).ok()?;
    Some((
        info.width.min(u32::from(u16::MAX)) as u16,
        info.height.min(u32::from(u16::MAX)) as u16,
    ))
}

/// Dimensions from the first decodable SPS (type 7) in a 4-byte length-prefixed
/// H.264 access unit, per [`avc_sps_dimensions`]. `None` if the buffer is
/// malformed or carries no decodable SPS.
pub fn avc_access_unit_dimensions(au: &[u8]) -> Option<(u16, u16)> {
    iter_length_prefixed_nals(au)
        .ok()?
        .into_iter()
        .filter(|nal| nal_unit_type(NalCodec::Avc, nal) == Some(AVC_NAL_SPS))
        .find_map(avc_sps_dimensions)
}

// ── Parameter-set validation (issue #1171) ───────────────────────────────────

/// Largest parameter-set NAL a decoder configuration record can carry: the
/// per-NAL length field is `unsigned int(16)` in both
/// `AVCDecoderConfigurationRecord` (`sequenceParameterSetLength` /
/// `pictureParameterSetLength`, ISO/IEC 14496-15:2017 §5.3.3.1.2) and
/// `HEVCDecoderConfigurationRecord` (`nalUnitLength`, §8.3.3.1.2). A longer
/// "parameter set" cannot be serialized into an `avcC`/`hvcC`.
pub(crate) const MAX_PARAMETER_SET_LEN: usize = u16::MAX as usize;
/// `forbidden_zero_bit` of the first NAL-header byte, H.264 §7.3.1 and H.265
/// §7.3.1.2 — must be 0 in a conforming NAL unit.
const NAL_FORBIDDEN_BIT_MASK: u8 = 0x80;
/// H.265 NAL header byte 1 low 3 bits: `nuh_temporal_id_plus1`, which shall
/// not be 0 (ITU-T H.265 §7.4.2.2).
const HEVC_TEMPORAL_ID_PLUS1_MASK: u8 = 0x07;

/// Which parameter-set role a NAL plays, for [`parameter_set_is_valid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParamKind {
    /// Video parameter set (HEVC only).
    Vps,
    /// Sequence parameter set.
    Sps,
    /// Picture parameter set.
    Pps,
}

/// Whether `nal` (a whole NAL unit including its header, as yielded by
/// [`iter_annexb_nals`]) is a parameter set of role `kind` that a decoder
/// configuration may be built from (issue #1171).
///
/// Damaged input regularly yields a NAL whose first byte *looks like* an
/// SPS/PPS header but whose body is garbage that runs to the end of the access
/// unit (an 85 704-byte "SPS" was observed). Such a set must not become a
/// track's config: it can overflow the record's `u16` length field, so the
/// resulting `avcC`/`hvcC` could not be serialized at all. A valid set is
///
/// * non-empty and no longer than [`MAX_PARAMETER_SET_LEN`];
/// * of the right `nal_unit_type` with a clear `forbidden_zero_bit` (and, for
///   HEVC, a non-zero `nuh_temporal_id_plus1`);
/// * terminated by an RBSP stop bit — the last byte of any NAL is non-zero
///   (H.264 §7.4.1.1 / H.265 §7.4.2: `trailing_zero_8bits` are not part of
///   the NAL unit); and
/// * for an SPS, one the SPS decoder parses to non-zero geometry.
///
/// Only [`NalCodec::Avc`] and [`NalCodec::Hevc`] have parameter-set records
/// here; any other codec yields `false`.
pub(crate) fn parameter_set_is_valid(codec: NalCodec, kind: ParamKind, nal: &[u8]) -> bool {
    let (Some(&first), Some(&last)) = (nal.first(), nal.last()) else {
        return false;
    };
    if nal.len() > MAX_PARAMETER_SET_LEN || last == 0 || first & NAL_FORBIDDEN_BIT_MASK != 0 {
        return false;
    }
    match codec {
        NalCodec::Avc => {
            let want = match kind {
                ParamKind::Sps => AVC_NAL_SPS,
                ParamKind::Pps => AVC_NAL_PPS,
                ParamKind::Vps => return false,
            };
            if first & NAL_TYPE_MASK != want || nal.len() < AVC_NAL_MIN_LEN {
                return false;
            }
            kind != ParamKind::Sps
                || crate::sps::decode_avc_sps(nal).is_ok_and(|i| i.width > 0 && i.height > 0)
        }
        NalCodec::Hevc => {
            let want = match kind {
                ParamKind::Vps => HEVC_VPS_NUT,
                ParamKind::Sps => HEVC_SPS_NUT,
                ParamKind::Pps => HEVC_PPS_NUT,
            };
            if nal_unit_type(NalCodec::Hevc, nal) != Some(want)
                || nal.len() < HEVC_NAL_MIN_LEN
                || nal[1] & HEVC_TEMPORAL_ID_PLUS1_MASK == 0
            {
                return false;
            }
            kind != ParamKind::Sps
                || crate::sps::decode_hevc_sps(nal).is_ok_and(|i| i.width > 0 && i.height > 0)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_bitstream::{
        AvcPictureParameterSet, AvcSequenceParameterSet, avc_access_unit_parameter_sets,
    };

    /// Verbatim H.264 High SPS NAL from a real stream (see `sps.rs` tests).
    const REAL_AVC_SPS: &[u8] = &[
        0x67, 0x64, 0x00, 0x0D, 0xAC, 0xD9, 0x41, 0x41, 0xFB, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00,
        0x10, 0x00, 0x00, 0x03, 0x03, 0x20, 0xF1, 0x42, 0x99, 0x60,
    ];
    const REAL_HEVC_SPS: &[u8] = &test_fixtures::vectors::HEVC_MAIN_SPS_320X240;

    #[test]
    fn real_parameter_sets_are_valid() {
        use ParamKind::*;
        assert!(parameter_set_is_valid(NalCodec::Avc, Sps, REAL_AVC_SPS));
        assert!(parameter_set_is_valid(
            NalCodec::Avc,
            Pps,
            &[0x68, 0xCE, 0x3C, 0x80]
        ));
        assert!(parameter_set_is_valid(NalCodec::Hevc, Sps, REAL_HEVC_SPS));
        assert!(parameter_set_is_valid(
            NalCodec::Hevc,
            Vps,
            &[0x40, 0x01, 0x0C]
        ));
        assert!(parameter_set_is_valid(
            NalCodec::Hevc,
            Pps,
            &[0x44, 0x01, 0xC1]
        ));
    }

    /// The #1171 shape: a plausible SPS header followed by a body longer than
    /// the `u16` record length field. Rejected even if it parsed.
    #[test]
    fn oversized_parameter_set_is_invalid() {
        for (codec, kind, good) in [
            (NalCodec::Avc, ParamKind::Sps, REAL_AVC_SPS),
            (NalCodec::Hevc, ParamKind::Sps, REAL_HEVC_SPS),
        ] {
            let mut big = good.to_vec();
            big.resize(MAX_PARAMETER_SET_LEN + 1, 0x55);
            assert!(!parameter_set_is_valid(codec, kind, &big));
            assert!(!parameter_set_is_valid(
                NalCodec::Avc,
                ParamKind::Pps,
                &[&[0x68u8][..], &[0x55; MAX_PARAMETER_SET_LEN][..]].concat()
            ));
        }
        // 85 704 bytes, the length observed in the report.
        let mut garbage = alloc::vec![0x67u8];
        garbage.resize(85_704, 0xA5);
        assert!(!parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Sps,
            &garbage
        ));
    }

    #[test]
    fn empty_wrong_type_and_bad_header_are_invalid() {
        use ParamKind::*;
        for codec in [NalCodec::Avc, NalCodec::Hevc] {
            assert!(!parameter_set_is_valid(codec, Sps, &[]));
        }
        // wrong type for the requested role
        assert!(!parameter_set_is_valid(
            NalCodec::Avc,
            Sps,
            &[0x68, 0xCE, 0x3C, 0x80]
        ));
        assert!(!parameter_set_is_valid(NalCodec::Avc, Pps, REAL_AVC_SPS));
        assert!(!parameter_set_is_valid(
            NalCodec::Avc,
            Vps,
            &[0x40, 0x01, 0x0C]
        ));
        assert!(!parameter_set_is_valid(NalCodec::Hevc, Pps, REAL_HEVC_SPS));
        // forbidden_zero_bit set
        let mut bad = REAL_AVC_SPS.to_vec();
        bad[0] |= 0x80;
        assert!(!parameter_set_is_valid(NalCodec::Avc, Sps, &bad));
        // nuh_temporal_id_plus1 == 0
        let mut bad = REAL_HEVC_SPS.to_vec();
        bad[1] &= !0x07;
        assert!(!parameter_set_is_valid(NalCodec::Hevc, Sps, &bad));
        // trailing zero byte: no RBSP stop bit
        let mut bad = REAL_AVC_SPS.to_vec();
        *bad.last_mut().unwrap() = 0;
        assert!(!parameter_set_is_valid(NalCodec::Avc, Sps, &bad));
        // VVC has no parameter-set record here
        assert!(!parameter_set_is_valid(NalCodec::Vvc, Sps, REAL_AVC_SPS));
    }

    /// Right header, but the body is not a decodable SPS.
    #[test]
    fn unparseable_sps_is_invalid() {
        assert!(!parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Sps,
            &[0x67, 0x42, 0x00, 0x1E, 0xAB, 0x40]
        ));
        let truncated = &REAL_HEVC_SPS[..8];
        assert!(!parameter_set_is_valid(
            NalCodec::Hevc,
            ParamKind::Sps,
            truncated
        ));
    }

    #[test]
    fn parameter_set_length_boundaries_are_inclusive_and_reject_a_bare_header() {
        // Exactly the maximum record length, non-zero stop bit: accepted.
        let max_pps = [&[0x68u8][..], &[0x55; MAX_PARAMETER_SET_LEN - 1][..]].concat();
        assert_eq!(max_pps.len(), MAX_PARAMETER_SET_LEN);
        assert!(parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Pps,
            &max_pps
        ));
        // A 1-byte "PPS" has no body: rejected.
        assert!(!parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Pps,
            &[0x68]
        ));
        // The smallest real PPS (2 bytes) is accepted; a zero trailing byte is not.
        assert!(parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Pps,
            &[0x68, 0x01]
        ));
        assert!(!parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Pps,
            &[0x68, 0x00]
        ));
    }

    /// A Baseline-profile H.264 SPS whose 16x16 picture is cropped by
    /// `(left, right, top, bottom)` chroma-sample offsets.
    fn avc_sps_with_crop(crop: (u64, u64, u64, u64)) -> Vec<u8> {
        let (left, right, top, bottom) = crop;
        let mut nal = alloc::vec![0x67u8, 66, 0x00, 30];
        let mut s = crate::test_bits::BitWriter::new();
        s.ue(0); // seq_parameter_set_id
        s.ue(0); // log2_max_frame_num_minus4
        s.ue(2); // pic_order_cnt_type
        s.ue(1); // max_num_ref_frames
        s.bit(false); // gaps_in_frame_num_value_allowed_flag
        s.ue(0); // pic_width_in_mbs_minus1 -> 16
        s.ue(0); // pic_height_in_map_units_minus1 -> 16
        s.bit(true); // frame_mbs_only_flag
        s.bit(true); // direct_8x8_inference_flag
        s.bit(true); // frame_cropping_flag
        s.ue(left);
        s.ue(right);
        s.ue(top);
        s.ue(bottom);
        s.bit(false); // vui_parameters_present_flag
        s.bit(true); // rbsp_stop_one_bit
        nal.extend_from_slice(&broadcast_bitstream::rbsp::escape(&s.finish()));
        nal
    }

    /// A 4:2:0 HEVC SPS whose 16x16 picture is cropped by
    /// `(left, right, top, bottom)` chroma-sample offsets.
    fn hevc_sps_with_conformance_window(window: (u64, u64, u64, u64)) -> Vec<u8> {
        let (left, right, top, bottom) = window;
        let mut nal = alloc::vec![0x42u8, 0x01];
        let mut s = crate::test_bits::BitWriter::new();
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
        s.bit(true); // conformance_window_flag
        s.ue(left);
        s.ue(right);
        s.ue(top);
        s.ue(bottom);
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
        s.ue(0); // num_short_term_ref_pic_sets
        s.bit(false); // long_term_ref_pics_present_flag
        s.bit(false); // sps_temporal_mvp_enabled_flag
        s.bit(false); // strong_intra_smoothing_enabled_flag
        s.bit(false); // vui_parameters_present_flag
        s.bit(true); // rbsp_stop_one_bit
        nal.extend_from_slice(&broadcast_bitstream::rbsp::escape(&s.finish()));
        nal
    }

    #[test]
    fn parameter_set_that_decodes_to_zero_geometry_is_invalid() {
        for (crop, geometry) in [((8, 0, 0, 0), (0, 16)), ((0, 0, 8, 0), (16, 0))] {
            let avc = avc_sps_with_crop(crop);
            let info = crate::sps::decode_avc_sps(&avc).expect("AVC SPS decodes");
            assert_eq!((info.width, info.height), geometry);
            assert!(!parameter_set_is_valid(NalCodec::Avc, ParamKind::Sps, &avc));

            let hevc = hevc_sps_with_conformance_window(crop);
            let info = crate::sps::decode_hevc_sps(&hevc).expect("HEVC SPS decodes");
            assert_eq!((info.width, info.height), geometry);
            assert!(!parameter_set_is_valid(
                NalCodec::Hevc,
                ParamKind::Sps,
                &hevc
            ));
        }
    }

    /// Concatenate NAL units into the 4-byte big-endian length-prefixed form
    /// an access unit uses in `mdat`.
    fn length_prefixed_au(nals: &[&[u8]]) -> Vec<u8> {
        let mut au = Vec::new();
        for nal in nals {
            au.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            au.extend_from_slice(nal);
        }
        au
    }

    #[test]
    fn avc_access_unit_scan_reports_the_sps_dimensions_and_parameter_sets() {
        let pps = [0x68u8, 0xCE, 0x3C, 0x80];
        let sei = [0x06u8, 0x05, 0x01, 0xAA, 0x80];
        let idr = [0x65u8, 0x88];
        let au = length_prefixed_au(&[&sei, REAL_AVC_SPS, &pps, &idr]);

        assert_eq!(
            avc_access_unit_dimensions(&au),
            Some((320, 240)),
            "the in-band SPS's real geometry"
        );
        assert_eq!(
            avc_access_unit_dimensions(&au),
            avc_sps_dimensions(REAL_AVC_SPS)
        );

        let (sps, pps_out) = avc_access_unit_parameter_sets(&au);
        assert_eq!(
            sps,
            alloc::vec![AvcSequenceParameterSet::new(REAL_AVC_SPS.to_vec())]
        );
        assert_eq!(
            pps_out,
            alloc::vec![AvcPictureParameterSet::new(pps.to_vec())]
        );
    }

    #[test]
    fn avc_access_unit_without_an_sps_has_no_dimensions() {
        let pps = [0x68u8, 0xCE, 0x3C, 0x80];
        let idr = [0x65u8, 0x88];
        let au = length_prefixed_au(&[&pps, &idr]);
        assert_eq!(avc_access_unit_dimensions(&au), None);
        let (sps, pps_out) = avc_access_unit_parameter_sets(&au);
        assert!(sps.is_empty());
        assert_eq!(
            pps_out,
            alloc::vec![AvcPictureParameterSet::new(pps.to_vec())]
        );
    }

    #[test]
    fn malformed_avc_access_unit_yields_none_and_no_parameter_sets() {
        // Declares a 99-byte NAL in a 5-byte buffer.
        let malformed = [0x00u8, 0x00, 0x00, 0x63, 0x67];
        assert_eq!(avc_access_unit_dimensions(&malformed), None);
        let (sps, pps) = avc_access_unit_parameter_sets(&malformed);
        assert!(sps.is_empty());
        assert!(pps.is_empty());
    }
}
