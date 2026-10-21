# VUI timing, frame rate and `RESOLUTION` — spec transcriptions (H.264 E, H.265 E/C, RFC 8216bis)

Verbatim text-layer transcriptions with PDF page numbers (1-based file pages of
`private/specs/itu_t_h264_avc.pdf` V15 08/2024 and `itu_t_h265_hevc.pdf` V11 01/2026). The two
syntax tables were additionally checked against rendered pages (`pdftoppm`, viewed): H.264 E.1.1
(pdf 454, printed 430), H.265 vui_parameters tail (pdf 446, printed 428), hrd_parameters
(pdf 447, printed 429), sub_layer_hrd_parameters (pdf 448, printed 430); Table E-6 (H.264 pdf 468, printed 444) and Table E.7 (H.265 pdf 468,
printed 450) were checked row by row against the rendered pages. Written 2026-10-21;
the H.264 part was written AFTER the `FRAME-RATE` code merged (#1409) — a process failure
(transcription must precede implementation) — and the code was then audited against it.

## 1. H.264 (ITU-T H.264 V15)

### 1.1 Syntax — E.1.1 (pdf 454, rendered and checked)

```
vui_parameters( ) { ...
    timing_info_present_flag                                   u(1)
    if( timing_info_present_flag ) {
        num_units_in_tick                                      u(32)
        time_scale                                             u(32)
        fixed_frame_rate_flag                                  u(1)
    }
    nal_hrd_parameters_present_flag                            u(1)
    if( nal_hrd_parameters_present_flag ) hrd_parameters( )
    vcl_hrd_parameters_present_flag                            u(1)
    if( vcl_hrd_parameters_present_flag ) hrd_parameters( )
    if( nal_hrd_parameters_present_flag || vcl_hrd_parameters_present_flag ) low_delay_hrd_flag   u(1)
    pic_struct_present_flag                                    u(1)
    bitstream_restriction_flag                                 u(1)  ...
}
```

### 1.2 Semantics — E.2.1 (pdf 467-469, printed 443-444)

The rendered pages show `Δt` (delta) where the text layer prints `t`: `Δt_fi,dpb(n)`, `Δt_o,dpb(n)` (restored below; subscripts are subscripts on the page).

```
timing_info_present_flag equal to 1 specifies that num_units_in_tick, time_scale and fixed_frame_rate_flag are present
in the bitstream. timing_info_present_flag equal to 0 specifies that num_units_in_tick, time_scale and
fixed_frame_rate_flag are not present in the bitstream.
num_units_in_tick is the number of time units of a clock operating at the frequency time_scale Hz that corresponds to
one increment (called a clock tick) of a clock tick counter. num_units_in_tick shall be greater than 0. A clock tick is the
minimum interval of time that can be represented in the coded data. For example, when the frame rate of a video signal
is 30 000 ÷ 1001 Hz, time_scale may be equal to 60 000 and num_units_in_tick may be equal to 1001. See Equation C-
1.
time_scale is the number of time units that pass in one second. For example, a time coordinate system that measures time
using a 27 MHz clock has a time_scale of 27 000 000. time_scale shall be greater than 0.
fixed_frame_rate_flag equal to 1 indicates that the temporal distance between the HRD output times of any two
consecutive pictures in output order is constrained as follows. fixed_frame_rate_flag equal to 0 indicates that no such
constraints apply to the temporal distance between the HRD output times of any two consecutive pictures in output order.
When fixed_frame_rate_flag is not present, it shall be inferred to be equal to 0.
For each picture n where n indicates the n-th picture (in output order) that is output and picture n is not the last picture in
the bitstream (in output order) that is output, the value of Δtfi,dpb( n ) is specified by

                  Δtfi,dpb( n ) = Δto,dpb( n ) ÷ DeltaTfiDivisor                                                        (E-78)



                                                                           Rec. ITU-T H.264 (V15) (08/2024)               443

where Δto,dpb( n ) is specified in Equation C-13 and DeltaTfiDivisor is specified by Table E-6 based on the value of
pic_struct_present_flag, field_pic_flag, and pic_struct for the coded video sequence containing picture n. Entries marked
"-" in Table E-6 indicate a lack of dependence of DeltaTfiDivisor on the corresponding syntax element.
When fixed_frame_rate_flag is equal to 1 for a coded video sequence containing picture n, the value computed for
Δtfi,dpb( n ) shall be equal to tc as specified in Equation C-1 (using the value of tc for the coded video sequence containing
picture n) when either or both of the following conditions are true for the following picture n n that is specified for use in
Equation C-13:
–     picture nn is in the same coded video sequence as picture n.
–     picture nn is in a different coded video sequence and fixed_frame_rate_flag is equal to 1 in the coded video sequence
      containing picture nn and the value of num_units_in_tick ÷ time_scale is the same for both coded video sequences.

                                     Table E-6 – Divisor for computation of Δtfi,dpb( n )

                          pic_struct_present_flag field_pic_flag pic_struct DeltaTfiDivisor
                          0                             1                 -              1
                          1                             -                 1              1
                          1                             -                 2              1
                          0                             0                 -              2
                          1                             -                 0              2
                          1                             -                 3              2
                          1                             -                 4              2
                          1                             -                 5              3
                          1                             -                 6              3
                          1                             -                 7              4
                          1                             -                 8              6
    NOTE 13 – In order to produce a DeltaTfiDivisor other than 2 for a picture with field_pic_flag equal to 0, pic_struct_present_flag
    must be equal to 1.
```

### 1.3 Equation C-1 (pdf 341)

```
The variable tc is derived as follows and is called a clock tick :

          tc = num_units_in_tick ÷ time_scale                                                                                   (C-1)
```

### 1.3a pic_struct — D.2.3 text (pdf 381, printed 357) and Table D-1 (pdf 382, printed 358; rows checked on the rendered page)

```
pic_struct indicates whether a picture should be displayed as a frame or one or more fields, according to Table D-1.
Frame doubling (pic_struct equal to 7) indicates that the frame should be displayed two times consecutively, and frame
tripling (pic_struct equal to 8) indicates that the frame should be displayed three times consecutively.
   NOTE 5 – Frame doubling can facilitate the display, for example, of 25p video on a 50p display and 29.97p video on a 59.94p
   display. Using frame doubling and frame tripling in combination on every other frame can facilitate the display of 23.98p video on
   a 59.94p display.
When pic_struct is present (pic_struct_present_flag is equal to 1), the constraints specified in the third column of
Table D-1 shall be obeyed.
   NOTE 6 – When pic_struct_present_flag is equal to 0, then in many cases default values may be inferred. In the absence of other
   indications of the intended display type of a picture, the decoder should infer the value of pic_struct as follows:
     – If field_pic_flag is equal to 1, pic_struct should be inferred to be equal to (1 + bottom_field_flag).
     – Otherwise, if TopFieldOrderCnt is equal to BottomFieldOrderCnt, pic_struct should be inferred to be equal to 0.
     – Otherwise, if TopFieldOrderCnt is less than BottomFieldOrderCnt, pic_struct should be inferred to be equal to 3.
     – Otherwise (field_pic_flag is equal to 0 and TopFieldOrderCnt is greater than BottomFieldOrderCnt), pic_struct should be
        inferred to be equal to 4.
   pic_struct is only a hint as to how the decoded video should be displayed on an assumed display type (e.g., interlaced or progressive)
   at an assumed display rate. When another display type or display rate is used by the decoder, then pic_struct does not indicate the
   display method, but may aid in processing the decoded video for the alternative display. When it is desired for pic_struct to have
   an effective value in the range of 5 to 8, inclusive, pic_struct_present_flag should be equal to 1, as the above inference rule will
   not produce these values.
                                                                               Rec. ITU-T H.264 (V15) (08/2024)                     357
```

Table D-1 – Interpretation of pic_struct

| Value | Indicated display of picture | Restrictions | NumClockTS |
|---|---|---|---|
| 0 | (progressive) frame | field_pic_flag shall be 0, TopFieldOrderCnt shall be equal to BottomFieldOrderCnt | 1 |
| 1 | top field | field_pic_flag shall be 1, bottom_field_flag shall be 0 | 1 |
| 2 | bottom field | field_pic_flag shall be 1, bottom_field_flag shall be 1 | 1 |
| 3 | top field, bottom field, in that order | field_pic_flag shall be 0, TopFieldOrderCnt shall be less than or equal to BottomFieldOrderCnt | 2 |
| 4 | bottom field, top field, in that order | field_pic_flag shall be 0, BottomFieldOrderCnt shall be less than or equal to TopFieldOrderCnt | 2 |
| 5 | top field, bottom field, top field repeated, in that order | field_pic_flag shall be 0, TopFieldOrderCnt shall be less than or equal to BottomFieldOrderCnt | 3 |
| 6 | bottom field, top field, bottom field repeated, in that order | field_pic_flag shall be 0, BottomFieldOrderCnt shall be less than or equal to TopFieldOrderCnt | 3 |
| 7 | frame doubling | field_pic_flag shall be 0, fixed_frame_rate_flag shall be 1, TopFieldOrderCnt shall be equal to BottomFieldOrderCnt | 2 |
| 8 | frame tripling | field_pic_flag shall be 0, fixed_frame_rate_flag shall be 1, TopFieldOrderCnt shall be equal to BottomFieldOrderCnt | 3 |
| 9..15 | reserved | | |

### 1.4 What follows (derived — not spec text)

* With `fixed_frame_rate_flag = 1` the output interval between consecutive pictures equals
  `t_fi,dpb(n) * DeltaTfiDivisor` with `t_fi,dpb(n) = tc` (E-78 + Table E-6). Table E-6: a frame
  picture (`field_pic_flag = 0`, no `pic_struct`) has divisor 2, a field picture
  (`field_pic_flag = 1`) divisor 1, so BOTH a frame and a field PAIR advance by `2 * tc`:
  nominal frame rate = `time_scale / (2 * num_units_in_tick)`. `pic_struct` (picture timing SEI,
  only when `pic_struct_present_flag = 1`) can change the divisor per picture (0:2, 1:1, 2:1,
  3:2, 4:2, 5:3, 6:3, 7:4, 8:6). So per-picture output intervals are 2*tc (pic_struct 0, 3, 4),
  1*tc (a field picture: pic_struct 1, 2, or `field_pic_flag = 1` without pic_struct), 3*tc (5, 6), 4*tc (7, frame doubling) or 6*tc (8, frame tripling); `2 * tc` is the NOMINAL frame
  interval only (the cadence of repeated/doubled pictures differs). The SPS alone cannot say which
  pic_struct a picture has (it is in the picture timing SEI), so the nominal `2 * tc` is what an
  SPS-only derivation can report; Table D-1 / E-6 above are the supporting text.
* With `fixed_frame_rate_flag = 0` (or absent) the spec states NO constraint on the output times,
  so the frame rate is **not normatively derivable**; `time_scale / (2 * num_units_in_tick)` is
  then an implementation-defined convention (common practice, not part of the Recommendation).

## 2. H.265 (ITU-T H.265 V11)

### 2.1 Syntax (pdf 446-448; rendered and checked)

```
vui_parameters( ) { ...
    neutral_chroma_indication_flag      u(1)
    field_seq_flag                      u(1)
    frame_field_info_present_flag       u(1)
    default_display_window_flag         u(1)
    if( default_display_window_flag ) { def_disp_win_{left,right,top,bottom}_offset  ue(v) }
    vui_timing_info_present_flag        u(1)
    if( vui_timing_info_present_flag ) {
        vui_num_units_in_tick           u(32)
        vui_time_scale                  u(32)
        vui_poc_proportional_to_timing_flag   u(1)
        if( vui_poc_proportional_to_timing_flag )
            vui_num_ticks_poc_diff_one_minus1 ue(v)
        vui_hrd_parameters_present_flag u(1)
        if( vui_hrd_parameters_present_flag )
            hrd_parameters( 1, sps_max_sub_layers_minus1 )
    }
    bitstream_restriction_flag          u(1) ...
}

hrd_parameters( commonInfPresentFlag, maxNumSubLayersMinus1 ) {
    if( commonInfPresentFlag ) {
        nal_hrd_parameters_present_flag         u(1)
        vcl_hrd_parameters_present_flag         u(1)
        if( nal_hrd_parameters_present_flag || vcl_hrd_parameters_present_flag ) {
            sub_pic_hrd_params_present_flag     u(1)
            if( sub_pic_hrd_params_present_flag ) {
                tick_divisor_minus2                       u(8)
                du_cpb_removal_delay_increment_length_minus1  u(5)
                sub_pic_cpb_params_in_pic_timing_sei_flag u(1)
                dpb_output_delay_du_length_minus1         u(5)
            }
            bit_rate_scale                      u(4)
            cpb_size_scale                      u(4)
            if( sub_pic_hrd_params_present_flag ) cpb_size_du_scale  u(4)
            initial_cpb_removal_delay_length_minus1 u(5)
            au_cpb_removal_delay_length_minus1      u(5)
            dpb_output_delay_length_minus1          u(5)
        }
    }
    for( i = 0; i <= maxNumSubLayersMinus1; i++ ) {
        fixed_pic_rate_general_flag[ i ]        u(1)
        if( !fixed_pic_rate_general_flag[ i ] )
            fixed_pic_rate_within_cvs_flag[ i ] u(1)
        if( fixed_pic_rate_within_cvs_flag[ i ] )
            elemental_duration_in_tc_minus1[ i ] ue(v)
        else
            low_delay_hrd_flag[ i ]             u(1)
        if( !low_delay_hrd_flag[ i ] )
            cpb_cnt_minus1[ i ]                 ue(v)
        if( nal_hrd_parameters_present_flag )  sub_layer_hrd_parameters( i )
        if( vcl_hrd_parameters_present_flag )  sub_layer_hrd_parameters( i )
    }
}

sub_layer_hrd_parameters( subLayerId ) {
    for( i = 0; i < CpbCnt; i++ ) {                     /* CpbCnt = cpb_cnt_minus1[ subLayerId ] + 1 */
        bit_rate_value_minus1[ i ]              ue(v)
        cpb_size_value_minus1[ i ]              ue(v)
        if( sub_pic_hrd_params_present_flag ) {
            cpb_size_du_value_minus1[ i ]       ue(v)
            bit_rate_du_value_minus1[ i ]       ue(v)
        }
        cbr_flag[ i ]                           u(1)
    }
}
```

(`fixed_pic_rate_within_cvs_flag[i]` inferred 1 when `fixed_pic_rate_general_flag[i]` is 1: §2.5;
`cpb_cnt_minus1[i]` and `low_delay_hrd_flag[i]` inferred 0 when absent, `CpbCnt = cpb_cnt_minus1 + 1`: §2.5a.)

### 2.2 field_seq_flag (pdf 462-463, printed 444-445)

```
field_seq_flag equal to 1 indicates that the CVS conveys pictures that represent fields, and specifies that a picture timing
SEI message shall be present in every access unit of the current CVS. field_seq_flag equal to 0 indicates that the CVS
conveys pictures that represent frames and that a picture timing SEI message may or may not be present in any access
unit of the current CVS. When field_seq_flag is not present, it is inferred to be equal to 0. When
general_frame_only_constraint_flag is equal to 1, the value of field_seq_flag shall be equal to 0.
   NOTE 18 – The specified decoding process does not treat access units conveying pictures that represent fields or frames differently.
   A sequence of pictures that represent fields would therefore be coded with the picture dimensions of an individual field. For
   example, access units containing pictures that represent 1080i fields would commonly have cropped output dimensions of
   1 920x540, while the sequence picture rate would commonly express the rate of the source fields (typically between 50 Hz and 60 Hz), instead of the source frame rate (typically between 25 Hz and 30 Hz).
```

### 2.3 frame_field_info_present_flag (pdf 463, printed 445)

```
frame_field_info_present_flag equal to 1 specifies that picture timing SEI messages are present for every picture and
include the pic_struct, source_scan_type and duplicate_flag syntax elements. frame_field_info_present_flag equal to 0
specifies that the pic_struct syntax element is not present in picture timing SEI messages.
When frame_field_info_present_flag is present and either or both of the following conditions are true,
frame_field_info_present_flag shall be equal to 1:
–    field_seq_flag is equal to 1.
–    general_progressive_source_flag is equal to 1 and general_interlaced_source_flag is equal to 1.
When frame_field_info_present_flag is not present, its value is inferred as follows:
–    If general_progressive_source_flag is equal to 1 and general_interlaced_source_flag is equal to 1,
     frame_field_info_present_flag is inferred to be equal to 1.
–    Otherwise, frame_field_info_present_flag is inferred to be equal to 0.
```

### 2.4 VUI timing semantics (pdf 463-464, printed 445-446)

```
vui_timing_info_present_flag     equal    to   1    specifies   that   vui_num_units_in_tick,      vui_time_scale,
vui_poc_proportional_to_timing_flag and vui_hrd_parameters_present_flag are present in the vui_parameters( ) syntax
structure. vui_timing_info_present_flag equal to 0 specifies that vui_num_units_in_tick, vui_time_scale,
vui_poc_proportional_to_timing_flag and vui_hrd_parameters_present_flag are not present in the vui_parameters( )
syntax structure.
vui_num_units_in_tick is the number of time units of a clock operating at the frequency vui_time_scale Hz that
corresponds to one increment (called a clock tick) of a clock tick counter. vui_num_units_in_tick shall be greater than 0.
A clock tick, in units of seconds, is equal to the quotient of vui_num_units_in_tick divided by vui_time_scale. For
example, when the picture rate of a video signal is 25 Hz, vui_time_scale may be equal to 27 000 000 and
vui_num_units_in_tick may be equal to 1 080 000 and consequently a clock tick may be equal to 0.04 seconds.
When vps_num_units_in_tick is present in the VPS referred to by the SPS, vui_num_units_in_tick, when present, shall
be equal to vps_num_units_in_tick, and when not present, is inferred to be equal to vps_num_units_in_tick.



                                                                                Rec. ITU-T H.265 (V11) (01/2026)              445

vui_time_scale is the number of time units that pass in one second. For example, a time coordinate system that measures
time using a 27 MHz clock has a vui_time_scale of 27 000 000. The value of vui_time_scale shall be greater than 0.
When vps_time_scale is present in the VPS referred to by the SPS, vui_time_scale, when present, shall be equal to
vps_time_scale, and when not present, is inferred to be equal to vps_time_scale.
vui_poc_proportional_to_timing_flag equal to 1 indicates that the picture order count value for each picture in the CVS
that is not the first picture in the CVS, in decoding order, is proportional to the output time of the picture relative to the
output time of the first picture in the CVS. vui_poc_proportional_to_timing_flag equal to 0 indicates that the picture order
count value for each picture in the CVS that is not the first picture in the CVS, in decoding order, may or may not be
proportional to the output time of the picture relative to the output time of the first picture in the CVS.
When vps_poc_proportional_to_timing_flag is present in the VPS referred to by the SPS and the value is equal to 1,
vui_poc_proportional_to_timing_flag, when present, shall be equal to 1.
vui_num_ticks_poc_diff_one_minus1 plus 1 specifies the number of clock ticks corresponding to a difference of picture
order count values equal to 1. The value of vui_num_ticks_poc_diff_one_minus1 shall be in the range of 0 to 2^32 − 2,
inclusive.
When vps_num_ticks_poc_diff_one_minus1 is present in the VPS referred to by the                                                 SPS,
vui_num_ticks_poc_diff_one_minus1, when present, shall be equal to vps_num_ticks_poc_diff_one_minus1.
vui_hrd_parameters_present_flag equal to 1 specifies that the syntax structure hrd_parameters( ) is present in the
vui_parameters( ) syntax structure. vui_hrd_parameters_present_flag equal to 0 specifies that the syntax structure
hrd_parameters( ) is not present in the vui_parameters( ) syntax structure.
```

Note on §2.4: `2^32` is a superscript on the page (the text layer prints `232`); it is written `2^32` here.

### 2.5 HRD semantics incl. Table E.7 (pdf 467-468, printed 449-450)

```
fixed_pic_rate_general_flag[ i ] equal to 1 indicates that, when HighestTid is equal to i, the temporal distance between
the HRD output times of consecutive pictures in output order is constrained as specified below.
fixed_pic_rate_general_flag[ i ] equal to 0 indicates that this constraint may not apply.
When fixed_pic_rate_general_flag[ i ] is not present, it is inferred to be equal to 0.
fixed_pic_rate_within_cvs_flag[ i ] equal to 1 indicates that, when HighestTid is equal to i, the temporal distance
between the HRD output times of consecutive pictures in output order is constrained as specified below.
fixed_pic_rate_within_cvs_flag[ i ] equal to 0 indicates that this constraint may not apply.
When fixed_pic_rate_general_flag[ i ] is equal to 1, the value of fixed_pic_rate_within_cvs_flag[ i ] is inferred to be
equal to 1.
elemental_duration_in_tc_minus1[ i ] plus 1 (when present) specifies, when HighestTid is equal to i, the temporal
distance, in clock ticks, between the elemental units that specify the HRD output times of consecutive pictures in output
order as specified below. The value of elemental_duration_in_tc_minus1[ i ] shall be in the range of 0 to 2 047, inclusive.
When HighestTid is equal to i and fixed_pic_rate_within_cvs_flag[ i ] is equal to 1 for a CVS containing picture n, and
for each picture n that is output and not the last picture in the bitstream (in output order) that is output, the value of the
variable DpbOutputElementalInterval[ n ] is specified by:

       DpbOutputElementalInterval[ n ] = DpbOutputInterval[ n ] ÷ DeltaToDivisor                                       (E-86)

where DpbOutputInterval[ n ] is specified in Equation C-16 and DeltaToDivisor is specified in Table E.7 based on the
value of frame_field_info_present_flag and pic_struct for the CVS containing picture n. Entries marked "-" in Table E.7
indicate a lack of dependence of DeltaToDivisor on the corresponding syntax element.
When HighestTid is equal to i and fixed_pic_rate_within_cvs_flag[ i ] is equal to 1 for a CVS containing picture n, the
value computed for DpbOutputElementalInterval[ n ] shall be equal to ClockTick * ( elemental_duration_in_tc_
minus1[ i ] + 1 ), wherein ClockTick is as specified in Equation C-1 (using the value of ClockTick for the CVS containing
picture n) when one of the following conditions is true for the following picture in output order nextPicInOutputOrder
that is specified for use in Equation C-16:


                                                                             Rec. ITU-T H.265 (V11) (01/2026)             449

–     picture nextPicInOutputOrder is in the same CVS as picture n.
–     picture nextPicInOutputOrder is in a different CVS and fixed_pic_rate_general_flag[ i ] is equal to 1 in the CVS
      containing picture nextPicInOutputOrder, the value of ClockTick is the same for both CVSs and the value of
      elemental_duration_in_tc_minus1[ i ] is the same for both CVSs.

                        Table E.7 – Divisor for computation of DpbOutputElementalInterval[ n ]

                             frame_field_info_present_flag              pic_struct    DeltaToDivisor
                                               0                             -                1
                                               1                            1                 1
                                               1                            2                 1
                                               1                            0                 1
                                               1                            3                 2
                                               1                            4                 2
                                               1                            5                 3
                                               1                            6                 3
                                               1                            7                 2
                                               1                            8                 3
                                               1                            9                 1
                                               1                            10                1
                                               1                            11                1
                                               1                            12                1
```

### 2.5a Inferences and `CpbCnt` (pdf 468, printed 450; checked on the rendered page)

```
low_delay_hrd_flag[ i ] specifies the HRD operational mode, when HighestTid is equal to i, as specified in Annex C or
clause F.13. When not present, the value of low_delay_hrd_flag[ i ] is inferred to be equal to 0.
cpb_cnt_minus1[ i ] plus 1 specifies the number of alternative CPB specifications in the bitstream of the CVS when
HighestTid is equal to i. The value of cpb_cnt_minus1[ i ] shall be in the range of 0 to 31, inclusive. When not present,
the value of cpb_cnt_minus1[ i ] is inferred to be equal to 0.
E.3.3 Sub-layer HRD parameters semantics
The variable CpbCnt is set equal to cpb_cnt_minus1[ subLayerId ] + 1.
```

### 2.6 Equation C-1 (pdf 297) and C-16 (pdf 303)

```
The variable ClockTick is derived as follows and is called a clock tick:

          ClockTick = vui_num_units_in_tick ÷ vui_time_scale                                                                    (C-1)
```

```
When picture n is a picture that is output and is not the last picture of the bitstream that is output, the value of the variable
DpbOutputInterval[ n ] is derived as follows:

        DpbOutputInterval[ n ] = DpbOutputTime[ nextPicInOutputOrder ] − DpbOutputTime[ n ]                                    (C-16)

where nextPicInOutputOrder is the picture that follows picture n in output order and has PicOutputFlag equal to 1.
```

### 2.6a Table D.2 – Interpretation of pic_struct (pdf 348, printed 330; rows checked on the rendered page)

| Value | Indicated display of picture | Restrictions |
|---|---|---|
| 0 | (progressive) Frame | field_seq_flag shall be equal to 0 |
| 1 | Top field | field_seq_flag shall be equal to 1 |
| 2 | Bottom field | field_seq_flag shall be equal to 1 |
| 3 | Top field, bottom field, in that order | field_seq_flag shall be equal to 0 |
| 4 | Bottom field, top field, in that order | field_seq_flag shall be equal to 0 |
| 5 | Top field, bottom field, top field repeated, in that order | field_seq_flag shall be equal to 0 |
| 6 | Bottom field, top field, bottom field repeated, in that order | field_seq_flag shall be equal to 0 |
| 7 | Frame doubling | field_seq_flag shall be equal to 0; fixed_pic_rate_within_cvs_flag shall be equal to 1 |
| 8 | Frame tripling | field_seq_flag shall be equal to 0; fixed_pic_rate_within_cvs_flag shall be equal to 1 |
| 9 | Top field paired with previous bottom field in output order | field_seq_flag shall be equal to 1 |
| 10 | Bottom field paired with previous top field in output order | field_seq_flag shall be equal to 1 |
| 11 | Top field paired with next bottom field in output order | field_seq_flag shall be equal to 1 |
| 12 | Bottom field paired with next top field in output order | field_seq_flag shall be equal to 1 |

### 2.6b HighestTid (clause 8.1.2, pdf 135, printed 117; checked on the rendered page)

```
The variable HighestTid, which identifies the highest temporal sub-layer to be decoded, is specified as follows:
– If some external means, not specified in this Specification, is available to set HighestTid, HighestTid is set by the
  external means.
– Otherwise, if the decoding process is invoked in a bitstream conformance test as specified in clause C.1, HighestTid
  is set as specified in clause C.1.
– Otherwise, HighestTid is set equal to sps_max_sub_layers_minus1 in the SPS referenced by the first coded picture
  in decoding order in the CVSG.
```

### 2.7 What follows (derived — not spec text)

* Without external means or a conformance test, `HighestTid = sps_max_sub_layers_minus1` (§2.6b), i.e. all temporal sub-layers are decoded, so index `i = sps_max_sub_layers_minus1` of the HRD arrays describes the full picture rate.
* **Normative tier**: `fixed_pic_rate_within_cvs_flag[HighestTid] = 1` (so `elemental_duration_in_tc_minus1`
  present): `DpbOutputElementalInterval = ClockTick * (elemental_duration_in_tc_minus1 + 1)`,
  `ClockTick = vui_num_units_in_tick / vui_time_scale`. Per Table E.7 the divisor is 1 when
  `frame_field_info_present_flag = 0` (picture interval = elemental interval). When
  `field_seq_flag = 1` the pictures are fields (§2.2) and every picture carries `pic_struct` in
  {1,2,9,10,11,12} (Table D.2: those are the values whose restriction is `field_seq_flag = 1`; Table E.7
  divisor 1), so the **picture** interval is the elemental interval. NOTE 18 (informative) says the
  sequence picture rate "would commonly express the rate of the source fields". Taking a frame as a
  complementary field PAIR (pic_struct 9-12 say "paired"; 1/2 alone do not guarantee pairing), the
  frame interval is twice the picture interval, i.e. frame rate = half the picture rate. That
  pairing step is an ASSUMPTION of this derivation, not spec text.
  When `field_seq_flag = 0` and `frame_field_info_present_flag = 1`
  the per-picture `pic_struct` (picture timing SEI, not in the SPS) can be 0 (divisor 1),
  3/4/7 (2) or 5/6/8 (3): the frame rate is not derivable from the SPS → no value.
* `vui_poc_proportional_to_timing_flag` / `vui_num_ticks_poc_diff_one_minus1` give time per POC
  unit, but nothing in the quoted text says consecutive output pictures differ by exactly 1 in POC, so a picture rate cannot be concluded from it: **not used**.
* **Convention tier (implementation-defined, owner-approved 2026-10-21)**: when the normative
  tier yields nothing because no HRD fixed-rate information is present
  (`vui_hrd_parameters_present_flag = 0`, or `fixed_pic_rate_within_cvs_flag = 0`), and
  `field_seq_flag = 0` and `frame_field_info_present_flag = 0`: frame rate =
  `vui_time_scale / vui_num_units_in_tick` — H.265's own informative example ("picture rate
  25 Hz ... clock tick 0.04 s", E.3.1) and common practice; it is NOT normative. Omitted for
  `field_seq_flag = 1` without HRD info and for `frame_field_info_present_flag = 1` (cadence
  unknown). VPS timing (`vps_timing_info`, inherited by the SPS VUI when absent there) is NOT
  parsed: absent in the SPS VUI → no value.

## 3. HLS `RESOLUTION` (IETF draft-pantos-hls-rfc8216bis; `specs/ietf_draft_pantos_hls_rfc8216bis.txt`, line numbers by `\n`)

§4.4.6.2 RESOLUTION attribute (lines 2707-2714):

```
      RESOLUTION

      The value is a decimal-resolution describing the optimal pixel
      resolution at which to display all the video in the Variant
      Stream.

      The RESOLUTION attribute is OPTIONAL but is recommended if the
      Variant Stream includes video.
```

§4.4.6.2.1 Alternative Renditions, rule (lines 3001-3004):

```
   *  If an EXT-X-STREAM-INF tag contains a RESOLUTION attribute and a
      VIDEO attribute, then every alternative video Rendition MUST have
      an optimal display resolution matching the value of the RESOLUTION
      attribute.
```

Client behaviour, §6.3.3 (lines 4019-4023):

```
   resolution change.  If the Variant Stream includes a RESOLUTION
   attribute, clients SHOULD display all video within a rectangle with
   the same proportions as that resolution.

   Clients SHOULD be prepared to handle multiple tracks of a particular
```

Decision recorded in `docs/display-geometry/DESIGN.md` Q8: `RESOLUTION` = square-pixel
presentation size (display size, not coded size).
