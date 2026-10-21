# H.265 VPS timing (`video_parameter_set_rbsp`) — page-verified transcription

Source: ITU-T H.265 (V11) 01/2026, `private/specs/itu_t_h265_hevc.pdf`.
Converted with the `pdf2md` tool (`--engine hybrid --pages 52-53,91-93 --report`; exit code 0, empty
verification report), then the syntax table was checked row by row against the rendered pages
(pdf 52-53, printed 34-35; `pdftoppm -r 85`). Semantics: pdf 91-93. `profile_tier_level( )` is
transcribed in `h265-sps-ptl.md`; `hrd_parameters( )` in `vui-timing-frame-rate.md` §2.1.
Written BEFORE the code (rule), with an independent fidelity pass (see the PR).

## 1. Syntax — 7.3.2.1 (pdf 52-53; checked on the rendered pages)

```
video_parameter_set_rbsp( ) {                                                       Descriptor
    vps_video_parameter_set_id                                                      u(4)
    vps_base_layer_internal_flag                                                    u(1)
    vps_base_layer_available_flag                                                   u(1)
    vps_max_layers_minus1                                                           u(6)
    vps_max_sub_layers_minus1                                                       u(3)
    vps_temporal_id_nesting_flag                                                    u(1)
    vps_reserved_0xffff_16bits                                                      u(16)
    profile_tier_level( 1, vps_max_sub_layers_minus1 )
    vps_sub_layer_ordering_info_present_flag                                        u(1)
    for( i = ( vps_sub_layer_ordering_info_present_flag ? 0 : vps_max_sub_layers_minus1 );
         i <= vps_max_sub_layers_minus1; i++ ) {
        vps_max_dec_pic_buffering_minus1[ i ]                                       ue(v)
        vps_max_num_reorder_pics[ i ]                                               ue(v)
        vps_max_latency_increase_plus1[ i ]                                         ue(v)
    }
    vps_max_layer_id                                                                u(6)
    vps_num_layer_sets_minus1                                                       ue(v)
    for( i = 1; i <= vps_num_layer_sets_minus1; i++ )
        for( j = 0; j <= vps_max_layer_id; j++ )
            layer_id_included_flag[ i ][ j ]                                        u(1)
    vps_timing_info_present_flag                                                    u(1)
    if( vps_timing_info_present_flag ) {
        vps_num_units_in_tick                                                       u(32)
        vps_time_scale                                                              u(32)
        vps_poc_proportional_to_timing_flag                                         u(1)
        if( vps_poc_proportional_to_timing_flag )
            vps_num_ticks_poc_diff_one_minus1                                       ue(v)
        vps_num_hrd_parameters                                                      ue(v)
        for( i = 0; i < vps_num_hrd_parameters; i++ ) {
            hrd_layer_set_idx[ i ]                                                  ue(v)
            if( i > 0 )
                cprms_present_flag[ i ]                                             u(1)
            hrd_parameters( cprms_present_flag[ i ], vps_max_sub_layers_minus1 )
        }
    }
    vps_extension_flag                                                              u(1)
    if( vps_extension_flag )
        while( more_rbsp_data( ) )
            vps_extension_data_flag                                                 u(1)
    rbsp_trailing_bits( )
}
```

(The raw pdf2md table for this page has a mangled first column and a broken `for` condition — the
skill's known failure mode for tall syntax tables — so the block above is the page, retyped and
checked against the rendered image; the pdf2md output was used for the prose of §2.)

## 2. Semantics (pdf 91-93; pdf2md output with three text-layer slips corrected against the page: the `2^32` superscript and two spaces inside parentheses)

Only the elements this work depends on, plus the VPS HRD/extension semantics that bound what is NOT read. The rest of the VPS semantics (layer sets, ordering info) are not needed for timing and are not transcribed.

vps_max_sub_layers_minus1 plus 1 specifies the maximum number of temporal sub-layers that may be present in each CVS referring to the VPS. The value of vps_max_sub_layers_minus1 shall be in the range of 0 to 6, inclusive.

vps_timing_info_present_flag equal to 1 specifies that vps_num_units_in_tick, vps_time_scale, vps_poc_proportional_to_timing_flag and vps_num_hrd_parameters are present in the VPS.

vps_timing_info_present_flag equal to 0 specifies that vps_num_units_in_tick, vps_time_scale, vps_poc_proportional_to_timing_flag and vps_num_hrd_parameters are not present in the VPS.

vps_num_units_in_tick is the number of time units of a clock operating at the frequency vps_time_scale Hz that corresponds to one increment (called a clock tick) of a clock tick counter. The value of vps_num_units_in_tick shall be greater than 0. A clock tick, in units of seconds, is equal to the quotient of vps_num_units_in_tick divided by vps_time_scale. For example, when the picture rate of a video signal is 25 Hz, vps_time_scale may be equal to 27 000 000 and vps_num_units_in_tick may be equal to 1 080 000, and consequently a clock tick may be 0.04 seconds.

vps_time_scale is the number of time units that pass in one second. For example, a time coordinate system that measures time using a 27 MHz clock has a vps_time_scale of 27 000 000. The value of vps_time_scale shall be greater than 0.

vps_poc_proportional_to_timing_flag equal to 1 indicates that the picture order count value for each picture in the CVS that is not the first picture in the CVS, in decoding order, is proportional to the output time of the picture relative to the output time of the first picture in the CVS. vps_poc_proportional_to_timing_flag equal to 0 indicates that the picture order count value for each picture in the CVS that is not the first picture in the CVS, in decoding order, may or may not be proportional to the output time of the picture relative to the output time of the first picture in the CVS.

vps_num_ticks_poc_diff_one_minus1 plus 1 specifies the number of clock ticks corresponding to a difference of picture order count values equal to 1. The value of vps_num_ticks_poc_diff_one_minus1 shall be in the range of 0 to 2^32 − 2, inclusive.

vps_num_hrd_parameters specifies the number of hrd_parameters() syntax structures present in the VPS RBSP before the vps_extension_flag syntax element. The value of vps_num_hrd_parameters shall be in the range of 0 to vps_num_layer_sets_minus1 + 1, inclusive.

hrd_layer_set_idx[i] specifies the index, into the list of layer sets specified by the VPS, of the layer set to which the i-th hrd_parameters() syntax structure in the VPS applies. The value of hrd_layer_set_idx[i] shall be in the range of ( vps_base_layer_internal_flag ? 0 : 1 ) to vps_num_layer_sets_minus1, inclusive.

It is a requirement of bitstream conformance that the value of hrd_layer_set_idx[i] shall not be equal to the value of hrd_layer_set_idx[j] for any value of j not equal to i.

cprms_present_flag[i] equal to 1 specifies that the HRD parameters that are common for all sub-layers are present in the i-th hrd_parameters() syntax structure in the VPS. cprms_present_flag[i] equal to 0 specifies that the HRD parameters that are common for all sub-layers are not present in the i-th hrd_parameters() syntax structure in the VPS and are derived to be the same as the ( i − 1)-th hrd_parameters() syntax structure in the VPS. cprms_present_flag[0] is inferred to be equal to 1.

vps_extension_flag equal to 0 specifies that no vps_extension_data_flag syntax elements are present in the VPS RBSP syntax structure. vps_extension_flag equal to 1 specifies that there may be vps_extension_data_flag syntax elements present in the VPS RBSP syntax structure. Decoders conforming to a profile specified in Annex A but not supporting the INBLD capability specified in Annex F shall ignore all data that follow the value 1 for vps_extension_flag in a VPS NAL unit.

vps_extension_data_flag may have any value. Its presence and value do not affect the decoding process of the profiles specified in Annex A. Decoders conforming to a profile specified in Annex A but not supporting the INBLD capability specified in Annex F shall ignore all vps_extension_data_flag syntax elements.

## 3. Inheritance into the SPS VUI (pdf 463, printed 445; already quoted in
`vui-timing-frame-rate.md` §2.4)

"When vps_num_units_in_tick is present in the VPS referred to by the SPS, vui_num_units_in_tick,
when present, shall be equal to vps_num_units_in_tick, and when not present, is inferred to be
equal to vps_num_units_in_tick." and the same for `vui_time_scale` / `vps_time_scale`.

## 4. What follows (derived — not spec text)

* **Conditional on the VPS carrying timing.** §3 applies only when `vps_timing_info_present_flag = 1`
  ("When vps_num_units_in_tick is present in the VPS referred to by the SPS"). Then `ClockTick =
  vps_num_units_in_tick / vps_time_scale` (§2), and an SPS VUI that does not carry `vui_num_units_in_tick`
  / `vui_time_scale` has them inferred from it (§3). The §3 sentence sits in the VUI semantics ("when not
  present, is inferred"); applying it when the SPS has **no VUI at all** (`vui_parameters_present_flag = 0`)
  is an **inference** of this derivation (the text never says that case explicitly), taken because the
  timing syntax elements are then equally "not present".
* `field_seq_flag` and `frame_field_info_present_flag` still come from the SPS VUI (or their inferences
  when no VUI is present); the rules of `vui-timing-frame-rate.md` §2.7 apply unchanged with the inherited
  timing pair.
* **VPS HRD is not used by this derivation.** No quoted text links `hrd_parameters( )` in the VPS to the
  SPS HRD fixed-rate inference of E.3.2: `hrd_layer_set_idx` says only that a VPS HRD structure applies to a
  layer set (§2). This is a design choice (no tie found in the quoted text; the rest of the spec was not
  searched for one). With only VPS timing the derivation can therefore reach tier B (convention) but never
  tier A.
* **What a parser must read.** `vps_timing_info_present_flag` is reached after: the 4+1+1+6+3+1+16-bit header
  fields, `profile_tier_level( 1, vps_max_sub_layers_minus1 )` (`h265-sps-ptl.md`),
  `vps_sub_layer_ordering_info_present_flag` and its per-sub-layer ue(v) triple (loop start
  `vps_sub_layer_ordering_info_present_flag ? 0 : vps_max_sub_layers_minus1`), `vps_max_layer_id`,
  `vps_num_layer_sets_minus1`, and the `layer_id_included_flag[i][j]` double loop (bits only). Immediately
  after `vps_timing_info_present_flag` come `vps_num_units_in_tick` u(32) and `vps_time_scale` u(32).
  `vps_poc_proportional_to_timing_flag` / `vps_num_ticks_poc_diff_one_minus1`, the VPS HRD loop and
  `vps_extension_data_flag` (which `vps_extension_flag` says decoders not supporting INBLD ignore) are not
  needed by this derivation and are not read.
