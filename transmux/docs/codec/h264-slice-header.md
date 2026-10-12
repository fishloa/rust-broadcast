# H.264 slice header, PPS prefix and field-pair rules

Source: ITU-T H.264 (08/2021) (`private/specs/itu_t_h264_avc.pdf`), transcribed with
`pdf2md --engine textlayer` (verified text layer, exit 0) from PDF pages 32-33
(§3.35-3.37), 73 (§7.3.2.2), 77 (§7.3.3), 96-98 (§7.4.1.2.4), 114-120 (§7.4.3).
Only the fields `transmux` reads are used; everything after `redundant_pic_cnt` in the
slice header is NOT parsed (it is entropy/reference-list specific and not needed to
detect field pairs).

## 7.3.3 Slice header syntax (prefix read by `slice_header.rs`)

| slice_header() { | C | Descriptor |
|---|---|---|
| first_mb_in_slice | 2 | ue(v) |
| slice_type | 2 | ue(v) |
| pic_parameter_set_id | 2 | ue(v) |
| if( separate_colour_plane_flag == 1) |  |  |
| colour_plane_id | 2 | u(2) |
| frame_num | 2 | u(v) |
| if( !frame_mbs_only_flag) { |  |  |
| field_pic_flag | 2 | u(1) |
| if( field_pic_flag) |  |  |
| bottom_field_flag | 2 | u(1) |
| } |  |  |
| if( IdrPicFlag) |  |  |
| idr_pic_id | 2 | ue(v) |
| if( pic_order_cnt_type == 0) { |  |  |
| pic_order_cnt_lsb | 2 | u(v) |
| if( bottom_field_pic_order_in_frame_present_flag && !field_pic_flag) |  |  |
| delta_pic_order_cnt_bottom | 2 | se(v) |
| } |  |  |
| if( pic_order_cnt_type == 1 && !delta_pic_order_always_zero_flag) { |  |  |
| delta_pic_order_cnt[0] | 2 | se(v) |
| if( bottom_field_pic_order_in_frame_present_flag && !field_pic_flag) |  |  |
| delta_pic_order_cnt[1] | 2 | se(v) |
| } |  |  |
| if( redundant_pic_cnt_present_flag) |  |  |
| redundant_pic_cnt | 2 | ue(v) |
| ... (not parsed) |  |  |

`IdrPicFlag` = (`nal_unit_type` == 5). The slice-header RBSP follows the 1-byte NAL header
with `emulation_prevention_three_byte` removed (§7.4.1).

## Semantics used

- `slice_type` (Table 7-6): 0 P, 1 B, 2 I, 3 SP, 4 SI, 5 P, 6 B, 7 I, 8 SP, 9 SI (5..9 equivalent to 0..4).
- `frame_num` is `log2_max_frame_num_minus4 + 4` bits (SPS).
- `pic_order_cnt_lsb` is `log2_max_pic_order_cnt_lsb_minus4 + 4` bits (SPS).
- `field_pic_flag` = 1: slice of a coded field; 0 (or not present): slice of a coded frame.
  `MbaffFrameFlag = mb_adaptive_frame_field_flag && !field_pic_flag` (7-27) — an MBAFF
  picture is a *frame* (field_pic_flag = 0) and is never paired.
- `bottom_field_flag` = 1: bottom field; 0: top field; inferred 0 when not present.
- `idr_pic_id`: ue(v), 0..65535. `delta_pic_order_cnt_bottom`, `delta_pic_order_cnt[0/1]`:
  se(v), inferred 0 when absent. `redundant_pic_cnt`: ue(v), 0 for the primary picture.
- `nal_ref_idc` == 0 in one VCL NAL of an AU implies 0 in all VCL NALs of that AU.

## 7.3.2.2 PPS prefix (read by `slice_header.rs`)

| pic_parameter_set_rbsp() { | C | Descriptor |
|---|---|---|
| pic_parameter_set_id | 1 | ue(v) |
| seq_parameter_set_id | 1 | ue(v) |
| entropy_coding_mode_flag | 1 | u(1) |
| bottom_field_pic_order_in_frame_present_flag | 1 | u(1) |
| num_slice_groups_minus1 | 1 | ue(v) |
| if( num_slice_groups_minus1 > 0) { |  |  |
| slice_group_map_type | 1 | ue(v) |
| if( slice_group_map_type == 0) for( iGroup = 0; iGroup <= num_slice_groups_minus1; iGroup++) run_length_minus1[iGroup] ue(v) |  |  |
| else if( slice_group_map_type == 2) for( iGroup = 0; iGroup < num_slice_groups_minus1; iGroup++) { top_left[iGroup] ue(v); bottom_right[iGroup] ue(v) } |  |  |
| else if( slice_group_map_type == 3 \|\| 4 \|\| 5) { slice_group_change_direction_flag u(1); slice_group_change_rate_minus1 ue(v) } |  |  |
| else if( slice_group_map_type == 6) { pic_size_in_map_units_minus1 ue(v); for( i = 0; i <= pic_size_in_map_units_minus1; i++) slice_group_id[i] u(v) } |  |  |
| } |  |  |
| num_ref_idx_l0_default_active_minus1 | 1 | ue(v) |
| num_ref_idx_l1_default_active_minus1 | 1 | ue(v) |
| weighted_pred_flag | 1 | u(1) |
| weighted_bipred_idc | 1 | u(2) |
| pic_init_qp_minus26 | 1 | se(v) |
| pic_init_qs_minus26 | 1 | se(v) |
| chroma_qp_index_offset | 1 | se(v) |
| deblocking_filter_control_present_flag | 1 | u(1) |
| constrained_intra_pred_flag | 1 | u(1) |
| redundant_pic_cnt_present_flag | 1 | u(1) |

`slice_group_id[i]` is `Ceil(Log2(num_slice_groups_minus1 + 1))` bits (§7.4.2.2).

## SPS fields needed

From `h264-sps.md` (§7.3.2.1.1): `separate_colour_plane_flag`, `log2_max_frame_num_minus4`,
`pic_order_cnt_type`, `log2_max_pic_order_cnt_lsb_minus4`, `delta_pic_order_always_zero_flag`,
`frame_mbs_only_flag`.

## §7.4.1.2.4 first VCL NAL unit of a new primary coded picture

A slice of the current AU differs from the previous AU's primary picture in one or more of:
`frame_num`; `pic_parameter_set_id`; `field_pic_flag`; `bottom_field_flag` (present in both);
`nal_ref_idc` (one of them 0); POC type 0: `pic_order_cnt_lsb` or `delta_pic_order_cnt_bottom`;
POC type 1: `delta_pic_order_cnt[0]` or `[1]`.

## Complementary field pairs

- 3.35 *complementary field pair*: a complementary reference field pair or a complementary
  non-reference field pair.
- 3.36 *complementary non-reference field pair*: two non-reference fields in consecutive
  access units in decoding order as two coded fields of **opposite parity** sharing the same
  `frame_num`, where the first field is not already a paired field.
- 3.37 *complementary reference field pair*: two reference fields in consecutive access units
  in decoding order as two coded fields sharing the same `frame_num`, where the second field in
  decoding order is not an IDR picture and does not include a
  `memory_management_control_operation` equal to 5.

Implementation notes (transmux): the reference-pair definition does not restate "opposite
parity" but the two fields of one frame are by construction of opposite parity, so both kinds
require it. The mmco-5 exclusion needs `dec_ref_pic_marking()`, which sits behind
`ref_pic_list_modification()`/`pred_weight_table()` and is NOT parsed; a second field with
mmco 5 and matching `frame_num` would therefore (non-conformingly rare) be merged. A reference
field followed by a non-reference field (or the reverse) is not a complementary pair by the
definitions above and stays two samples.
