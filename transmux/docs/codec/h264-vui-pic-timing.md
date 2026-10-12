# H.264 interlace facts: SPS field flags, VUI HRD walk, pic_timing SEI

Source: ITU-T H.264 (V15, 08/2024) (`private/specs/itu_t_h264_avc.pdf`), text layer
extracted with `pdftotext -layout`: PDF pages 380-383 (D.2.3 semantics, Table D-1), 361
(D.1.2/D.1.3 syntax), 453-454 (E.1.1, E.1.2), 469 (E.2.1 semantics of CpbDpbDelaysPresentFlag /
pic_struct_present_flag). Only what `transmux` reads is transcribed.

## 7.3.2.1.1 (SPS, relevant fields)

`frame_mbs_only_flag` u(1); `if( !frame_mbs_only_flag ) mb_adaptive_frame_field_flag u(1)`.
`mb_adaptive_frame_field_flag` is only present when `frame_mbs_only_flag == 0`; semantics:
`MbaffFrameFlag = mb_adaptive_frame_field_flag && !field_pic_flag`.

Derived scan classes (transmux definitions, not spec terms):

| frame_mbs_only_flag | mb_adaptive_frame_field_flag | class |
|---|---|---|
| 1 | n/a | progressive: every picture is a frame, no `field_pic_flag` in the slice header |
| 0 | 0 | field-capable (PAFF): each picture is a frame or a field, chosen per picture by `field_pic_flag` |
| 0 | 1 | MBAFF-capable: frames use MBAFF (`field_pic_flag = 0`); pictures may still be fields (`field_pic_flag = 1`) |

## E.1.1 VUI parameters syntax (tail; the prefix through timing_info is walked in `sps.rs`)

```
timing_info_present_flag                u(1)
if( timing_info_present_flag ) { num_units_in_tick u(32); time_scale u(32); fixed_frame_rate_flag u(1) }
nal_hrd_parameters_present_flag         u(1)
if( nal_hrd_parameters_present_flag )  hrd_parameters( )
vcl_hrd_parameters_present_flag         u(1)
if( vcl_hrd_parameters_present_flag )  hrd_parameters( )
if( nal_hrd_parameters_present_flag || vcl_hrd_parameters_present_flag ) low_delay_hrd_flag u(1)
pic_struct_present_flag                 u(1)
bitstream_restriction_flag              u(1)
```

## E.1.2 HRD parameters syntax

```
hrd_parameters( ) {
  cpb_cnt_minus1                         ue(v)
  bit_rate_scale                         u(4)
  cpb_size_scale                         u(4)
  for( SchedSelIdx = 0; SchedSelIdx <= cpb_cnt_minus1; SchedSelIdx++ ) {
    bit_rate_value_minus1[ ]             ue(v)
    cpb_size_value_minus1[ ]             ue(v)
    cbr_flag[ ]                          u(1)
  }
  initial_cpb_removal_delay_length_minus1 u(5)
  cpb_removal_delay_length_minus1        u(5)
  dpb_output_delay_length_minus1         u(5)
  time_offset_length                     u(5)
}
```

## E.2.1 semantics

`CpbDpbDelaysPresentFlag` = 1 if `nal_hrd_parameters_present_flag` or
`vcl_hrd_parameters_present_flag` is 1 (or the application says so by other means, which
transmux cannot see), else 0. `pic_struct_present_flag` is inferred 0 when absent.

## D.1.3 Picture timing SEI message syntax (SEI payloadType 1, D.1.1)

```
pic_timing( payloadSize ) {
  if( CpbDpbDelaysPresentFlag ) {
    cpb_removal_delay                    u(v)   // cpb_removal_delay_length_minus1 + 1 bits
    dpb_output_delay                     u(v)   // dpb_output_delay_length_minus1 + 1 bits
  }
  if( pic_struct_present_flag ) {
    pic_struct                           u(4)
    ... clock timestamps (not read)
```

D.2.3 NOTE 2: the lengths are those "coded in the sequence parameter set that is active for the
primary coded picture". The spec does not say which of the NAL/VCL `hrd_parameters()` supplies
them when both are present; transmux reads `pic_struct` only when both agree (or only one is
present) and reports no `pic_struct` otherwise.

## Table D-1 - Interpretation of pic_struct

| value | indicated display | restrictions |
|---|---|---|
| 0 | (progressive) frame | field_pic_flag 0, TopFieldOrderCnt == BottomFieldOrderCnt |
| 1 | top field | field_pic_flag 1, bottom_field_flag 0 |
| 2 | bottom field | field_pic_flag 1, bottom_field_flag 1 |
| 3 | top field, bottom field, in that order | field_pic_flag 0, Top <= Bottom |
| 4 | bottom field, top field, in that order | field_pic_flag 0, Bottom <= Top |
| 5 | top field, bottom field, top field repeated | field_pic_flag 0, Top <= Bottom |
| 6 | bottom field, top field, bottom field repeated | field_pic_flag 0, Bottom <= Top |
| 7 | frame doubling | field_pic_flag 0, fixed_frame_rate_flag 1, Top == Bottom |
| 8 | frame tripling | field_pic_flag 0, fixed_frame_rate_flag 1, Top == Bottom |
| 9..15 | reserved | |

NOTE 6 (D.2.3): when `pic_struct_present_flag` is 0 a decoder "should infer" pic_struct from
`field_pic_flag`/`bottom_field_flag`/POC; transmux does NOT synthesise that. It reports `None`
and exposes `field_pic_flag`/`bottom_field_flag` directly.
