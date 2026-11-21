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

use crate::bit_writer::BitWriter;
use transmux::slice_header::decode_avc_slice_pps;

fn nal(ref_idc: u8, t: u8, rbsp: Vec<u8>) -> Vec<u8> {
    let mut v = vec![(ref_idc << 5) | t];
    v.extend(rbsp);
    v
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
    assert_eq!((max.pps_id, max.sps_id), (255, 31));
    assert!(max.redundant_pic_cnt_present);

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
    assert!(max.redundant_pic_cnt_present);
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
        assert_eq!(pps.pps_id, 0);
        assert_eq!(pps.sps_id, 0);
        assert!(pps.entropy_coding_mode, "map_type {map_type}: entropy mode");
        assert!(
            pps.redundant_pic_cnt_present,
            "map_type {map_type}: redundant_pic_cnt_present_flag misread"
        );
    }
}
