//! PPS and slice-header prefix bounds (ITU-T H.264 §7.3.2.2, §7.3.3,
//! §7.4.2.2): the `pps_id`/`sps_id`/`num_slice_groups_minus1` inclusive
//! maxima, the slice-group map-type walk that decides where
//! `redundant_pic_cnt_present_flag` is read, `slice_type`'s bound, the
//! per-slice PPS selection (`peek_pps_id`), and the AU classification of a
//! repeated/reordered field picture.
//!
//! PROVENANCE: hand-built RBSP bit streams (via the shared test `BitWriter`).
//! The committed captures carry one PPS and no FMO slice groups, so the
//! boundaries and every map type have to be constructed.

use broadcast_bitstream::slice_header::{
    AvcFieldInfo, AvcParamSets, AvcPicture, decode_avc_slice_pps, parse_avc_slice_header,
};
use broadcast_bitstream::sps::decode_avc_slice_sps;
use test_fixtures::bits::BitWriter;

fn nal(ref_idc: u8, t: u8, rbsp: Vec<u8>) -> Vec<u8> {
    let mut v = vec![(ref_idc << 5) | t];
    v.extend(rbsp);
    v
}

/// A Main-profile SPS whose `frame_mbs_only_flag` is `frame_mbs_only`.
fn sps_nal(frame_mbs_only: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(8, 77);
    w.u(8, 0);
    w.u(8, 30);
    w.ue(0); // seq_parameter_set_id
    w.ue(0); // log2_max_frame_num_minus4 -> 4 bits
    w.ue(0); // pic_order_cnt_type
    w.ue(2); // log2_max_pic_order_cnt_lsb_minus4 -> 6 bits
    w.ue(1); // max_num_ref_frames
    w.u(1, 0); // gaps_in_frame_num_value_allowed_flag
    w.ue(3); // pic_width_in_mbs_minus1
    w.ue(1); // pic_height_in_map_units_minus1
    w.u(1, u64::from(frame_mbs_only));
    if !frame_mbs_only {
        w.u(1, 0); // mb_adaptive_frame_field_flag
    }
    w.u(1, 1); // direct_8x8_inference_flag
    w.u(1, 0); // frame_cropping_flag
    w.u(1, 0); // vui_parameters_present_flag
    nal(3, 7, w.trailing())
}

/// A PPS with every prefix field the slice-group walk needs spelled out.
fn pps_nal(
    pps_id: u32,
    sps_id: u32,
    entropy: bool,
    slice_groups_minus1: u32,
    map_type: u32,
    nrl0: u32,
    redundant: bool,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ue(u64::from(pps_id));
    w.ue(u64::from(sps_id));
    w.u(1, u64::from(entropy));
    w.u(1, 0); // bottom_field_pic_order_in_frame_present_flag
    w.ue(u64::from(slice_groups_minus1));
    if slice_groups_minus1 > 0 {
        w.ue(u64::from(map_type));
        match map_type {
            0 => {
                for _ in 0..=slice_groups_minus1 {
                    w.ue(0); // run_length_minus1
                }
            }
            2 => {
                for _ in 0..slice_groups_minus1 {
                    w.ue(0); // top_left
                    w.ue(0); // bottom_right
                }
            }
            3..=5 => {
                w.u(1, 0); // slice_group_change_direction_flag
                w.ue(0); // slice_group_change_rate_minus1
            }
            6 => {
                w.ue(0); // pic_size_in_map_units_minus1 -> one id
                let bits = (slice_groups_minus1 + 1)
                    .next_power_of_two()
                    .trailing_zeros();
                w.u(bits, 0); // slice_group_id
            }
            _ => {}
        }
    }
    w.ue(u64::from(nrl0)); // num_ref_idx_l0_default_active_minus1
    w.ue(0); // num_ref_idx_l1_default_active_minus1
    w.u(1, 0); // weighted_pred_flag
    w.u(2, 0); // weighted_bipred_idc
    w.se(0); // pic_init_qp_minus26
    w.se(0); // pic_init_qs_minus26
    w.se(0); // chroma_qp_index_offset
    w.u(1, 1); // deblocking_filter_control_present_flag
    w.u(1, 0); // constrained_intra_pred_flag
    w.u(1, u64::from(redundant));
    nal(3, 8, w.trailing())
}

/// `pps_id` 255 and `sps_id` 31 are the inclusive maxima (§7.4.2.2); 256 and
/// 32 are out. The two checks are independent, so one id out of range must be
/// rejected even when the other is in range.
#[test]
fn pps_and_sps_ids_are_bounded_inclusively() {
    let max =
        decode_avc_slice_pps(&pps_nal(255, 31, false, 0, 0, 0, true)).expect("255/31 allowed");
    assert_eq!((max.pps_id(), max.sps_id()), (255, 31));
    assert!(max.redundant_pic_cnt_present());

    for (pps_id, sps_id) in [(256, 0), (0, 32), (300, 0), (0, 40)] {
        assert!(
            decode_avc_slice_pps(&pps_nal(pps_id, sps_id, false, 0, 0, 0, false)).is_err(),
            "pps_id {pps_id} / sps_id {sps_id} must be rejected"
        );
    }
}

/// `num_slice_groups_minus1` 7 is the maximum (§7.4.2.2); 8 is refused.
#[test]
fn num_slice_groups_minus1_is_bounded_inclusively() {
    let max = decode_avc_slice_pps(&pps_nal(0, 0, false, 7, 6, 0, true)).expect("7 groups allowed");
    assert!(max.redundant_pic_cnt_present());
    let err = decode_avc_slice_pps(&pps_nal(0, 0, false, 8, 6, 0, false)).expect_err("8 groups");
    assert!(
        format!("{err:?}").contains("num_slice_groups_minus1"),
        "unexpected error: {err:?}"
    );
}

/// Each slice-group map type consumes a different number of bits before
/// `num_ref_idx_l0_default_active_minus1`. A skipped or short arm misreads
/// every later field, including `redundant_pic_cnt_present_flag`.
#[test]
fn every_slice_group_map_type_reaches_the_same_later_fields() {
    for map_type in [0u32, 2, 3, 5, 6] {
        let nrl0 = if matches!(map_type, 3 | 5) { 1 } else { 0 };
        let pps = decode_avc_slice_pps(&pps_nal(0, 0, true, 2, map_type, nrl0, true))
            .unwrap_or_else(|e| panic!("map_type {map_type}: {e:?}"));
        assert_eq!(pps.pps_id(), 0);
        assert_eq!(pps.sps_id(), 0);
        assert!(
            pps.entropy_coding_mode(),
            "map_type {map_type}: entropy mode"
        );
        assert!(
            pps.redundant_pic_cnt_present(),
            "map_type {map_type}: redundant_pic_cnt_present_flag misread"
        );
    }
}

/// A slice NAL prefix: `first_mb_in_slice` 0, `slice_type`, `pps_id`,
/// 4-bit `frame_num`, an optional field pair, an optional `idr_pic_id`, and a
/// 6-bit `pic_order_cnt_lsb`.
fn slice_nal(
    ref_idc: u8,
    idr: bool,
    slice_type: u32,
    pps_id: u32,
    frame_num: u32,
    field: Option<bool>,
    poco: u32,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ue(0); // first_mb_in_slice
    w.ue(u64::from(slice_type));
    w.ue(u64::from(pps_id));
    w.u(4, u64::from(frame_num));
    if let Some(bottom) = field {
        w.u(1, 1); // field_pic_flag
        w.u(1, u64::from(bottom));
    } else {
        w.u(1, 0); // field_pic_flag
    }
    if idr {
        w.ue(0); // idr_pic_id
    }
    w.u(6, u64::from(poco));
    nal(ref_idc, if idr { 5 } else { 1 }, w.trailing())
}

fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in nals {
        v.extend([0, 0, 0, 1]);
        v.extend(n);
    }
    v
}

/// `slice_type` 0..=9 are Table 7-6 values; 10 is out of range.
#[test]
fn slice_type_is_bounded_at_nine() {
    let sps = decode_avc_slice_sps(&sps_nal(false)).unwrap();
    let pps = decode_avc_slice_pps(&pps_nal(0, 0, false, 0, 0, 0, false)).unwrap();
    for slice_type in [0u32, 9] {
        let n = slice_nal(2, false, slice_type, 0, 0, None, 0);
        let h = parse_avc_slice_header(&n, &sps, &pps)
            .unwrap_or_else(|e| panic!("slice_type {slice_type}: {e:?}"));
        assert_eq!(h.slice_type(), slice_type as u8);
    }
    let n = slice_nal(2, false, 10, 0, 0, None, 0);
    let err = parse_avc_slice_header(&n, &sps, &pps).expect_err("slice_type 10");
    assert!(
        format!("{err:?}").contains("slice_type"),
        "unexpected: {err:?}"
    );
}

/// A slice naming PPS 3 must be parsed against PPS 3, not PPS 0: a field
/// picture parses with PPS 3 (its `pps_id` matches) and would be rejected as
/// a PPS mismatch against 0, classifying as `Unknown`.
#[test]
fn classification_selects_the_pps_named_by_the_slice() {
    let s = sps_nal(false);
    let p0 = pps_nal(0, 0, false, 0, 0, 0, false);
    let p3 = pps_nal(3, 0, true, 0, 0, 0, false);
    let slice = slice_nal(1, false, 7, 3, 5, Some(true), 2);
    let mut ps = AvcParamSets::new();
    let got = ps.classify_nals([&s[..], &p0[..], &p3[..], &slice[..]]);
    assert_eq!(
        got,
        AvcPicture::Field(AvcFieldInfo::new(true, 5, true, false)),
        "the slice names PPS 3"
    );
}

/// Two field slices that agree are one field picture — the AU is not a frame
/// just because it carries more than one slice.
#[test]
fn repeated_identical_field_slices_stay_a_field() {
    let s = sps_nal(false);
    let p = pps_nal(0, 0, false, 0, 0, 0, false);
    let first = slice_nal(1, false, 7, 0, 5, Some(true), 2);
    let second = slice_nal(1, false, 7, 0, 5, Some(true), 2);
    let mut ps = AvcParamSets::new();
    let got = ps.classify_au(&annexb(&[s.clone(), p.clone(), first.clone(), second]));
    assert_eq!(
        got,
        AvcPicture::Field(AvcFieldInfo::new(true, 5, true, false))
    );
    // Mixed parity is still a frame (two coded pictures in one AU).
    let other = slice_nal(1, false, 7, 0, 5, Some(false), 3);
    assert_eq!(
        ps.classify_au(&annexb(&[s, p, first, other])),
        AvcPicture::Frame
    );
}
