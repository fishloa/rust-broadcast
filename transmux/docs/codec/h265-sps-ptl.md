| vps_max_layer_id                                                     | u(6)  |
|----------------------------------------------------------------------|-------|
| vps_num_layer_sets_minus1                                            | ue(v) |
| for( i = 1; i <= vps_num_layer_sets_minus1; i++ )                    |       |
| for( j = 0; j <= vps_max_layer_id; j++ )                             |       |
| layer_id_included_flag[ i ][ j ]                                     | u(1)  |
| vps_timing_info_present_flag                                         | u(1)  |
| if( vps_timing_info_present_flag ) {                                 |       |
| vps_num_units_in_tick                                                | u(32) |
| vps_time_scale                                                       | u(32) |
| vps_poc_proportional_to_timing_flag                                  | u(1)  |
| if( vps_poc_proportional_to_timing_flag )                            |       |
| vps_num_ticks_poc_diff_one_minus1                                    | ue(v) |
| vps_num_hrd_parameters                                               | ue(v) |
| for( i = 0; i < vps_num_hrd_parameters; i++ ) {                      |       |
| hrd_layer_set_idx[ i ]                                               | ue(v) |
| if( i > 0 )                                                          |       |
| cprms_present_flag[ i ]                                              | u(1)  |
| hrd_parameters( cprms_present_flag[ i ], vps_max_sub_layers_minus1 ) |       |
| }                                                                    |       |
| }                                                                    |       |
| vps_extension_flag                                                   | u(1)  |
| if( vps_extension_flag )                                             |       |
| while( more_rbsp_data( ) )                                           |       |
| vps_extension_data_flag                                              | u(1)  |
| rbsp_trailing_bits( )                                                |       |
| }                                                                    |       |

## **7.3.2.2 Sequence parameter set RBSP syntax**

# **7.3.2.2.1 General sequence parameter set RBSP syntax**

| seq_parameter_set_rbsp( ) {                        | Descriptor |
|----------------------------------------------------|------------|
| sps_video_parameter_set_id                         | u(4)       |
| sps_max_sub_layers_minus1                          | u(3)       |
| sps_temporal_id_nesting_flag                       | u(1)       |
| profile_tier_level( 1, sps_max_sub_layers_minus1 ) |            |
| sps_seq_parameter_set_id                           | ue(v)      |
| chroma_format_idc                                  | ue(v)      |
| if( chroma_format_idc = = 3 )                      |            |
| separate_colour_plane_flag                         | u(1)       |
| pic_width_in_luma_samples                          | ue(v)      |
| pic_height_in_luma_samples                         | ue(v)      |
| conformance_window_flag                            | u(1)       |
| if( conformance_window_flag ) {                    |            |
| conf_win_left_offset                               | ue(v)      |
| conf_win_right_offset                              | ue(v)      |
| conf_win_top_offset                                | ue(v)      |

| conf_win_bottom_offset                                                                                                            | ue(v) |
|-----------------------------------------------------------------------------------------------------------------------------------|-------|
| }                                                                                                                                 |       |
| bit_depth_luma_minus8                                                                                                             | ue(v) |
| bit_depth_chroma_minus8                                                                                                           | ue(v) |
| log2_max_pic_order_cnt_lsb_minus4                                                                                                 | ue(v) |
| sps_sub_layer_ordering_info_present_flag                                                                                          | u(1)  |
| for( i = ( sps_sub_layer_ordering_info_present_flag ? 0 : sps_max_sub_layers_minus1 );<br>i <= sps_max_sub_layers_minus1; i++ ) { |       |
| sps_max_dec_pic_buffering_minus1[ i ]                                                                                             | ue(v) |
| sps_max_num_reorder_pics[ i ]                                                                                                     | ue(v) |
| sps_max_latency_increase_plus1[ i ]                                                                                               | ue(v) |
| }                                                                                                                                 |       |
| log2_min_luma_coding_block_size_minus3                                                                                            | ue(v) |
| log2_diff_max_min_luma_coding_block_size                                                                                          | ue(v) |
| log2_min_luma_transform_block_size_minus2                                                                                         | ue(v) |
| log2_diff_max_min_luma_transform_block_size                                                                                       | ue(v) |
| max_transform_hierarchy_depth_inter                                                                                               | ue(v) |
| max_transform_hierarchy_depth_intra                                                                                               | ue(v) |
| scaling_list_enabled_flag                                                                                                         | u(1)  |
| if( scaling_list_enabled_flag ) {                                                                                                 |       |
| sps_scaling_list_data_present_flag                                                                                                | u(1)  |
| if( sps_scaling_list_data_present_flag )                                                                                          |       |
| scaling_list_data( )                                                                                                              |       |
| }                                                                                                                                 |       |
| amp_enabled_flag                                                                                                                  | u(1)  |
| sample_adaptive_offset_enabled_flag                                                                                               | u(1)  |
| pcm_enabled_flag                                                                                                                  | u(1)  |
| if( pcm_enabled_flag ) {                                                                                                          |       |
| pcm_sample_bit_depth_luma_minus1                                                                                                  | u(4)  |
| pcm_sample_bit_depth_chroma_minus1                                                                                                | u(4)  |
| log2_min_pcm_luma_coding_block_size_minus3                                                                                        | ue(v) |
| log2_diff_max_min_pcm_luma_coding_block_size                                                                                      | ue(v) |
| pcm_loop_filter_disabled_flag                                                                                                     | u(1)  |
| }                                                                                                                                 |       |
| num_short_term_ref_pic_sets                                                                                                       | ue(v) |
| for( i = 0; i < num_short_term_ref_pic_sets; i++)                                                                                 |       |
| st_ref_pic_set( i )                                                                                                               |       |
| long_term_ref_pics_present_flag                                                                                                   | u(1)  |
| if( long_term_ref_pics_present_flag ) {                                                                                           |       |
| num_long_term_ref_pics_sps                                                                                                        | ue(v) |
| for( i = 0; i < num_long_term_ref_pics_sps; i++ ) {                                                                               |       |
| lt_ref_pic_poc_lsb_sps[ i ]                                                                                                       | u(v)  |
| used_by_curr_pic_lt_sps_flag[ i ]                                                                                                 | u(1)  |
| }                                                                                                                                 |       |
| }                                                                                                                                 |       |
| sps_temporal_mvp_enabled_flag                                                                                                     | u(1)  |
| strong_intra_smoothing_enabled_flag                                                                                               | u(1)  |
|                                                                                                                                   |       |

| vui_parameters_present_flag                            | u(1) |
|--------------------------------------------------------|------|
| if( vui_parameters_present_flag )                      |      |
| vui_parameters( )                                      |      |
| sps_extension_present_flag                             | u(1) |
| if( sps_extension_present_flag ) {                     |      |
| sps_range_extension_flag                               | u(1) |
| sps_multilayer_extension_flag                          | u(1) |
| sps_3d_extension_flag                                  | u(1) |
| sps_scc_extension_flag                                 | u(1) |
| sps_extension_4bits                                    | u(4) |
| }                                                      |      |
| if( sps_range_extension_flag )                         |      |
| sps_range_extension( )                                 |      |
| if( sps_multilayer_extension_flag )                    |      |
| sps_multilayer_extension( ) /* specified in Annex F */ |      |
| if( sps_3d_extension_flag )                            |      |
| sps_3d_extension( ) /* specified in Annex I */         |      |
| if( sps_scc_extension_flag )                           |      |
| sps_scc_extension( )                                   |      |
| if( sps_extension_4bits )                              |      |
| while( more_rbsp_data( ) )                             |      |
| sps_extension_data_flag                                | u(1) |
| rbsp_trailing_bits( )                                  |      |
| }                                                      |      |

#### **7.3.2.2.2 Sequence parameter set range extension syntax**

| sps_range_extension( ) {                | Descriptor |
|-----------------------------------------|------------|
| transform_skip_rotation_enabled_flag    | u(1)       |
| transform_skip_context_enabled_flag     | u(1)       |
| implicit_rdpcm_enabled_flag             | u(1)       |
| explicit_rdpcm_enabled_flag             | u(1)       |
| extended_precision_processing_flag      | u(1)       |
| intra_smoothing_disabled_flag           | u(1)       |
| high_precision_offsets_enabled_flag     | u(1)       |
| persistent_rice_adaptation_enabled_flag | u(1)       |
| cabac_bypass_alignment_enabled_flag     | u(1)       |
| }                                       |            |

## **7.3.2.2.3 Sequence parameter set screen content coding extension syntax**

| sps_scc_extension( ) {            | Descriptor |
|-----------------------------------|------------|
| sps_curr_pic_ref_enabled_flag     | u(1)       |
| palette_mode_enabled_flag         | u(1)       |
| if( palette_mode_enabled_flag ) { |            |
| palette_max_size                  | ue(v)      |

| delta_palette_max_predictor_size                                      | ue(v) |
|-----------------------------------------------------------------------|-------|
| sps_palette_predictor_initializers_present_flag                       | u(1)  |
| if( sps_palette_predictor_initializers_present_flag ) {               |       |
| sps_num_palette_predictor_initializers_minus1                         | ue(v) |
| numComps = ( chroma_format_idc = = 0 ) ? 1 : 3                        |       |
| for( comp = 0; comp < numComps; comp++ )                              |       |
| for( i = 0; i <= sps_num_palette_predictor_initializers_minus1; i++ ) |       |
| sps_palette_predictor_initializer[ comp ][ i ]                        | u(v)  |
| }                                                                     |       |
| }                                                                     |       |
| motion_vector_resolution_control_idc                                  | u(2)  |
| intra_boundary_filtering_disabled_flag                                | u(1)  |
| }                                                                     |       |

#### **7.3.2.3 Picture parameter set RBSP syntax**

## **7.3.2.3.1 General picture parameter set RBSP syntax**

| pic_parameter_set_rbsp( ) {                    | Descriptor |
|------------------------------------------------|------------|
| pps_pic_parameter_set_id                       | ue(v)      |
| pps_seq_parameter_set_id                       | ue(v)      |
| dependent_slice_segments_enabled_flag          | u(1)       |
| output_flag_present_flag                       | u(1)       |
| num_extra_slice_header_bits                    | u(3)       |
| sign_data_hiding_enabled_flag                  | u(1)       |
| cabac_init_present_flag                        | u(1)       |
| num_ref_idx_l0_default_active_minus1           | ue(v)      |
| num_ref_idx_l1_default_active_minus1           | ue(v)      |
| init_qp_minus26                                | se(v)      |
| constrained_intra_pred_flag                    | u(1)       |
| transform_skip_enabled_flag                    | u(1)       |
| cu_qp_delta_enabled_flag                       | u(1)       |
| if( cu_qp_delta_enabled_flag )                 |            |
| diff_cu_qp_delta_depth                         | ue(v)      |
| pps_cb_qp_offset                               | se(v)      |
| pps_cr_qp_offset                               | se(v)      |
| pps_slice_chroma_qp_offsets_present_flag       | u(1)       |
| weighted_pred_flag                             | u(1)       |
| weighted_bipred_flag                           | u(1)       |
| transquant_bypass_enabled_flag                 | u(1)       |
| tiles_enabled_flag                             | u(1)       |
| entropy_coding_sync_enabled_flag               | u(1)       |
| if( tiles_enabled_flag ) {                     |            |
| num_tile_columns_minus1                        | ue(v)      |
| num_tile_rows_minus1                           | ue(v)      |
| uniform_spacing_flag                           | u(1)       |
| if( !uniform_spacing_flag ) {                  |            |
| for( i = 0; i < num_tile_columns_minus1; i++ ) |            |

| column_width_minus1[ i ]                               | ue(v) |
|--------------------------------------------------------|-------|
| for( i = 0; i < num_tile_rows_minus1; i++ )            |       |
| row_height_minus1[ i ]                                 | ue(v) |
| }                                                      |       |
| loop_filter_across_tiles_enabled_flag                  | u(1)  |
| }                                                      |       |
| pps_loop_filter_across_slices_enabled_flag             | u(1)  |
| deblocking_filter_control_present_flag                 | u(1)  |
| if( deblocking_filter_control_present_flag ) {         |       |
| deblocking_filter_override_enabled_flag                | u(1)  |
| pps_deblocking_filter_disabled_flag                    | u(1)  |
| if( !pps_deblocking_filter_disabled_flag ) {           |       |
| pps_beta_offset_div2                                   | se(v) |
| pps_tc_offset_div2                                     | se(v) |
| }                                                      |       |
| }                                                      |       |
| pps_scaling_list_data_present_flag                     | u(1)  |
| if( pps_scaling_list_data_present_flag )               |       |
| scaling_list_data( )                                   |       |
| lists_modification_present_flag                        | u(1)  |
| log2_parallel_merge_level_minus2                       | ue(v) |
| slice_segment_header_extension_present_flag            | u(1)  |
| pps_extension_present_flag                             | u(1)  |
| if( pps_extension_present_flag ) {                     |       |
| pps_range_extension_flag                               | u(1)  |
| pps_multilayer_extension_flag                          | u(1)  |
| pps_3d_extension_flag                                  | u(1)  |
| pps_scc_extension_flag                                 | u(1)  |
| pps_extension_4bits                                    | u(4)  |
| }                                                      |       |
| if( pps_range_extension_flag )                         |       |
| pps_range_extension( )                                 |       |
| if( pps_multilayer_extension_flag )                    |       |
| pps_multilayer_extension( ) /* specified in Annex F */ |       |
| if( pps_3d_extension_flag )                            |       |
| pps_3d_extension( ) /* specified in Annex I */         |       |
| if( pps_scc_extension_flag )                           |       |
| pps_scc_extension( )                                   |       |
| if( pps_extension_4bits )                              |       |
| while( more_rbsp_data( ) )                             |       |
| pps_extension_data_flag                                | u(1)  |
| rbsp_trailing_bits( )                                  |       |
| }                                                      |       |

## **7.3.2.3.2 Picture parameter set range extension syntax**

| pps_range_extension( ) {                                   | Descriptor |
|------------------------------------------------------------|------------|
| if( transform_skip_enabled_flag )                          |            |
| log2_max_transform_skip_block_size_minus2                  | ue(v)      |
| cross_component_prediction_enabled_flag                    | u(1)       |
| chroma_qp_offset_list_enabled_flag                         | u(1)       |
| if( chroma_qp_offset_list_enabled_flag ) {                 |            |
| diff_cu_chroma_qp_offset_depth                             | ue(v)      |
| chroma_qp_offset_list_len_minus1                           | ue(v)      |
| for( i = 0; i <= chroma_qp_offset_list_len_minus1; i++ ) { |            |
| cb_qp_offset_list[ i ]                                     | se(v)      |
| cr_qp_offset_list[ i ]                                     | se(v)      |
| }                                                          |            |
| }                                                          |            |
| log2_sao_offset_scale_luma                                 | ue(v)      |
| log2_sao_offset_scale_chroma                               | ue(v)      |
| }                                                          |            |

## **7.3.2.3.3 Picture parameter set screen content coding extension syntax**

| pps_scc_extension( ) {                                        | Descriptor |
|---------------------------------------------------------------|------------|
| pps_curr_pic_ref_enabled_flag                                 | u(1)       |
| residual_adaptive_colour_transform_enabled_flag               | u(1)       |
| if( residual_adaptive_colour_transform_enabled_flag ) {       |            |
| pps_slice_act_qp_offsets_present_flag                         | u(1)       |
| pps_act_y_qp_offset_plus5                                     | se(v)      |
| pps_act_cb_qp_offset_plus5                                    | se(v)      |
| pps_act_cr_qp_offset_plus3                                    | se(v)      |
| }                                                             |            |
| pps_palette_predictor_initializers_present_flag               | u(1)       |
| if( pps_palette_predictor_initializers_present_flag ) {       |            |
| pps_num_palette_predictor_initializers                        | ue(v)      |
| if( pps_num_palette_predictor_initializers > 0 ) {            |            |
| monochrome_palette_flag                                       | u(1)       |
| luma_bit_depth_entry_minus8                                   | ue(v)      |
| if( !monochrome_palette_flag )                                |            |
| chroma_bit_depth_entry_minus8                                 | ue(v)      |
| numComps = monochrome_palette_flag ? 1 : 3                    |            |
| for( comp = 0; comp < numComps; comp++ )                      |            |
| for( i = 0; i < pps_num_palette_predictor_initializers; i++ ) |            |
| pps_palette_predictor_initializer[ comp ][ i ]                | u(v)       |
| }                                                             |            |
| }                                                             |            |
| }                                                             |            |

# **7.3.2.4 Supplemental enhancement information RBSP syntax**

| sei_rbsp( ) {              | Descriptor |
|----------------------------|------------|
| do                         |            |
| sei_message( )             |            |
| while( more_rbsp_data( ) ) |            |
| rbsp_trailing_bits( )      |            |
| }                          |            |

## **7.3.2.5 Access unit delimiter RBSP syntax**

| access_unit_delimiter_rbsp( ) { | Descriptor |
|---------------------------------|------------|
| pic_type                        | u(3)       |
| rbsp_trailing_bits( )           |            |
| }                               |            |

# **7.3.2.6 End of sequence RBSP syntax**

| end_of_seq_rbsp( ) { | Descriptor |
|----------------------|------------|
| }                    |            |

#### **7.3.2.7 End of bitstream RBSP syntax**

| end_of_bitstream_rbsp( ) { | Descriptor |
|----------------------------|------------|
| }                          |            |

## **7.3.2.8 Filler data RBSP syntax**

| filler_data_rbsp( ) {            | Descriptor |
|----------------------------------|------------|
| while( next_bits( 8 ) = = 0xFF ) |            |
| ff_byte /* equal to 0xFF */      | f(8)       |
| rbsp_trailing_bits( )            |            |
| }                                |            |

# **7.3.2.9 Slice segment layer RBSP syntax**

| slice_segment_layer_rbsp( ) {       | Descriptor |
|-------------------------------------|------------|
| slice_segment_header( )             |            |
| slice_segment_data( )               |            |
| rbsp_slice_segment_trailing_bits( ) |            |
| }                                   |            |

## **7.3.2.10 RBSP slice segment trailing bits syntax**

| rbsp_slice_segment_trailing_bits( ) { | Descriptor |
|---------------------------------------|------------|
| rbsp_trailing_bits( )                 |            |
| while( more_rbsp_trailing_data( ) )   |            |
| cabac_zero_word /* equal to 0x0000 */ | f(16)      |
| }                                     |            |

# **7.3.2.11 RBSP trailing bits syntax**

| rbsp_trailing_bits( ) {                  | Descriptor |
|------------------------------------------|------------|
| rbsp_stop_one_bit /* equal to 1 */       | f(1)       |
| while( !byte_aligned( ) )                |            |
| rbsp_alignment_zero_bit /* equal to 0 */ | f(1)       |
| }                                        |            |

## **7.3.2.12 Byte alignment syntax**

| byte_alignment( ) {                          | Descriptor |
|----------------------------------------------|------------|
| alignment_bit_equal_to_one /* equal to 1 */  | f(1)       |
| while( !byte_aligned( ) )                    |            |
| alignment_bit_equal_to_zero /* equal to 0 */ | f(1)       |
| }                                            |            |

## **7.3.3 Profile, tier and level syntax**

| profile_tier_level( profilePresentFlag, maxNumSubLayersMinus1 ) {                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Descriptor |
|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------|
| if( profilePresentFlag ) {                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |            |
| general_profile_space                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | u(2)       |
| general_tier_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | u(1)       |
| general_profile_idc                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | u(5)       |
| for( j = 0; j < 32; j++ )                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |            |
| general_profile_compatibility_flag[ j ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | u(1)       |
| general_progressive_source_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | u(1)       |
| general_interlaced_source_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | u(1)       |
| general_non_packed_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | u(1)       |
| general_frame_only_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | u(1)       |
| if( general_profile_idc = = 4     general_profile_compatibility_flag[ 4 ]    <br>general_profile_idc = = 5     general_profile_compatibility_flag[ 5 ]    <br>general_profile_idc = = 6     general_profile_compatibility_flag[ 6 ]    <br>general_profile_idc = = 7     general_profile_compatibility_flag[ 7 ]    <br>general_profile_idc = = 8     general_profile_compatibility_flag[ 8 ]    <br>general_profile_idc = = 9     general_profile_compatibility_flag[ 9 ]    <br>general_profile_idc = = 10     general_profile_compatibility_flag[ 10 ]    <br>general_profile_idc = = 11     general_profile_compatibility_flag[ 11 ] ) {<br>/* The number of bits in this syntax structure is not affected by this condition */ |            |
| general_max_12bit_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | u(1)       |

| general_max_10bit_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | u(1)  |
|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------|
| general_max_8bit_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | u(1)  |
| general_max_422chroma_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | u(1)  |
| general_max_420chroma_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | u(1)  |
| general_max_monochrome_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | u(1)  |
| general_intra_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | u(1)  |
| general_one_picture_only_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | u(1)  |
| general_lower_bit_rate_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | u(1)  |
| if( general_profile_idc = = 5     general_profile_compatibility_flag[ 5 ]    <br>general_profile_idc = = 9     general_profile_compatibility_flag[ 9 ]    <br>general_profile_idc = = 10     general_profile_compatibility_flag[ 10 ]    <br>general_profile_idc = = 11     general_profile_compatibility_flag[ 11 ] ) {                                                                                                                                                                                                                                          |       |
| general_max_14bit_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | u(1)  |
| general_reserved_zero_33bits                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | u(33) |
| } else                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |       |
| general_reserved_zero_34bits                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | u(34) |
| } else if( general_profile_idc = = 2     general_profile_compatibility_flag[ 2 ] ) {                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |       |
| general_reserved_zero_7bits                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | u(7)  |
| general_one_picture_only_constraint_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | u(1)  |
| general_reserved_zero_35bits                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | u(35) |
| } else                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |       |
| general_reserved_zero_43bits                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | u(43) |
| general_profile_idc = = 2     general_profile_compatibility_flag[ 2 ]    <br>general_profile_idc = = 3     general_profile_compatibility_flag[ 3 ]    <br>general_profile_idc = = 4     general_profile_compatibility_flag[ 4 ]    <br>general_profile_idc = = 5     general_profile_compatibility_flag[ 5 ]    <br>general_profile_idc = = 9     general_profile_compatibility_flag[ 9 ]    <br>general_profile_idc = = 11     general_profile_compatibility_flag[ 11 ] )<br>/* The number of bits in this syntax structure is not affected by this condition */ |       |
| general_inbld_flag                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | u(1)  |
| else                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |       |
| general_reserved_zero_bit                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | u(1)  |
| }                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |       |
| general_level_idc                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | u(8)  |
| for( i = 0; i < maxNumSubLayersMinus1; i++ ) {                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |       |
| sub_layer_profile_present_flag[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | u(1)  |
| sub_layer_level_present_flag[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | u(1)  |
| }                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |       |
| if( maxNumSubLayersMinus1 > 0 )                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |       |
| for( i = maxNumSubLayersMinus1; i < 8; i++ )                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |       |
| reserved_zero_2bits[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | u(2)  |
| for( i = 0; i < maxNumSubLayersMinus1; i++ ) {                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |       |
| if( sub_layer_profile_present_flag[ i ] ) {                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |       |
| sub_layer_profile_space[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | u(2)  |
| sub_layer_tier_flag[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | u(1)  |
| sub_layer_profile_idc[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | u(5)  |
| for( j = 0; j < 32; j++ )                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |       |
| sub_layer_profile_compatibility_flag[ i ][ j ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | u(1)  |
| sub_layer_progressive_source_flag[ i ]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | u(1)  |
|                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |       |