# H.265 interlace facts: VUI field_seq_flag, st_ref_pic_set walk, pic_timing SEI

Source: ITU-T H.265 (V11, 01/2026) (`private/specs/itu_t_h265_hevc.pdf`), text layer via
`pdftotext -layout`: PDF pages 68 (7.3.7 syntax), 121 (7-61/7-62), 314 (D.2.3 syntax),
345-348 (D.3.3 semantics, Table D.2), 445-448 (E.2.1-E.2.3), 462-463 (E.3.1 semantics).

## 7.3.7 st_ref_pic_set( stRpsIdx ) (needed to reach the SPS VUI)

```
st_ref_pic_set( stRpsIdx ) {
  if( stRpsIdx != 0 ) inter_ref_pic_set_prediction_flag      u(1)
  if( inter_ref_pic_set_prediction_flag ) {
    if( stRpsIdx == num_short_term_ref_pic_sets ) delta_idx_minus1   ue(v)   // slice header only
    delta_rps_sign                                           u(1)
    abs_delta_rps_minus1                                     ue(v)
    for( j = 0; j <= NumDeltaPocs[ RefRpsIdx ]; j++ ) {
      used_by_curr_pic_flag[ j ]                             u(1)
      if( !used_by_curr_pic_flag[ j ] ) use_delta_flag[ j ]  u(1)   // inferred 1 when absent
    }
  } else {
    num_negative_pics ue(v); num_positive_pics ue(v)
    for( i < num_negative_pics ) { delta_poc_s0_minus1[ i ] ue(v); used_by_curr_pic_s0_flag[ i ] u(1) }
    for( i < num_positive_pics ) { delta_poc_s1_minus1[ i ] ue(v); used_by_curr_pic_s1_flag[ i ] u(1) }
  }
}
```

In the SPS, `RefRpsIdx = stRpsIdx - ( delta_idx_minus1 + 1 )` with `delta_idx_minus1` inferred 0,
i.e. the previous set. `NumDeltaPocs[ i ] = NumNegativePics[ i ] + NumPositivePics[ i ]`.
For an inter-predicted set, (7-61)/(7-62) (PDF page 121) derive the new lists. With
`deltaRps = ( 1 - 2 * delta_rps_sign ) * ( abs_delta_rps_minus1 + 1 )`:

- negative list: for j from `NumPositivePics[Ref]-1` down to 0, `dPoc = DeltaPocS1[Ref][j] + deltaRps`, kept when `dPoc < 0 && use_delta_flag[NumNegativePics[Ref]+j]`; then the `deltaRps < 0 && use_delta_flag[NumDeltaPocs[Ref]]` entry (value deltaRps); then for j in 0..NumNegativePics[Ref], `dPoc = DeltaPocS0[Ref][j] + deltaRps`, kept when `dPoc < 0 && use_delta_flag[j]`.
- positive list: for j from `NumNegativePics[Ref]-1` down to 0, `dPoc = DeltaPocS0[Ref][j] + deltaRps`, kept when `dPoc > 0 && use_delta_flag[j]`; then the `deltaRps > 0 && use_delta_flag[NumDeltaPocs[Ref]]` entry; then for j in 0..NumPositivePics[Ref], `dPoc = DeltaPocS1[Ref][j] + deltaRps`, kept when `dPoc > 0 && use_delta_flag[NumNegativePics[Ref]+j]`.

transmux evaluates these exactly (it keeps the delta-POC lists per set; only the final counts
and the lists feeding later sets matter, the `used_by_curr` flags are not needed).

## E.2.1 vui_parameters (relevant part)

```
... chroma_loc_info_present_flag ...
neutral_chroma_indication_flag          u(1)
field_seq_flag                          u(1)
frame_field_info_present_flag           u(1)
default_display_window_flag             u(1) ...
```

E.3.1: `field_seq_flag` = 1: the CVS conveys pictures that represent FIELDS (a pic_timing SEI is
present in every AU); inferred 0 when absent. `frame_field_info_present_flag` = 1: pic_timing SEI
carries pic_struct/source_scan_type/duplicate_flag. When absent it is inferred 1 if
`general_progressive_source_flag == 1 && general_interlaced_source_flag == 1`, otherwise 0
(the two flags that follow `general_profile_compatibility_flag[31]` in profile_tier_level,
7.3.3 / `h265-sps-ptl.md`).

## D.2.3 pic_timing SEI syntax (prefix SEI NAL, payloadType 1, D.2.1)

```
pic_timing( payloadSize ) {
  if( frame_field_info_present_flag ) {
    pic_struct          u(4)
    source_scan_type    u(2)
    duplicate_flag      u(1)
  }
  ... (CpbDpbDelaysPresentFlag fields, not read)
```

`pic_struct` is the FIRST field, so no HRD walk is needed for HEVC.

`source_scan_type` (D.3.3): 0 interlaced, 1 progressive, 2 unknown/unspecified, 3 reserved
(decoders interpret 3 as 2).

## Table D.2 - Interpretation of pic_struct

| value | indicated display | restriction |
|---|---|---|
| 0 | (progressive) frame | field_seq_flag 0 |
| 1 | top field | field_seq_flag 1 |
| 2 | bottom field | field_seq_flag 1 |
| 3 | top field, bottom field, in that order | field_seq_flag 0 |
| 4 | bottom field, top field, in that order | field_seq_flag 0 |
| 5 | top, bottom, top repeated | field_seq_flag 0 |
| 6 | bottom, top, bottom repeated | field_seq_flag 0 |
| 7 | frame doubling | field_seq_flag 0, fixed_pic_rate_within_cvs_flag 1 |
| 8 | frame tripling | field_seq_flag 0, fixed_pic_rate_within_cvs_flag 1 |
| 9 | top field paired with previous bottom field in output order | field_seq_flag 1 |
| 10 | bottom field paired with previous top field in output order | field_seq_flag 1 |
| 11 | top field paired with next bottom field in output order | field_seq_flag 1 |
| 12 | bottom field paired with next top field in output order | field_seq_flag 1 |
| 13..15 | reserved | |

## RASL pictures (used by `live_join.rs`)

H.265 §3.126 (PDF text line "random access skipped leading (RASL) picture") and Table 7-1:
`nal_unit_type` 8 `RASL_N`, 9 `RASL_R`; RADL 6, 7; BLA 16..=18; IDR 19, 20; CRA 21; 22, 23 reserved
IRAP. "All RASL pictures are leading pictures of an associated BLA or CRA picture. When the
associated IRAP picture has NoRaslOutputFlag equal to 1, the RASL picture is not output and can be
non-correctly-decodable, as the RASL picture can contain references to pictures that are not
present in the bitstream ... When present, all RASL pictures precede, in decoding order, all
trailing pictures of the same associated IRAP picture." A join makes the join point the first
picture of the bitstream, so NoRaslOutputFlag = 1.

When `frame_field_info_present_flag` is 0 a decoder "can infer" pic_struct 0 (NOTE 4); transmux
reports `None` instead of synthesising.
