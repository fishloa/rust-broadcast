Bodies selected by `extension_descriptor_tag` (Table 2-110, tag 0x3F). The `descriptor_tag`/`descriptor_length`/`extension_descriptor_tag` header is not repeated in the per-body tables below (the spec omits it too).

## 0x02 — ODUpdate_descriptor body (Table 2-109: `ObjectDescriptorUpdate()`)
_Rec. ITU-T H.222.0 (08/2023) §2.6.90 Table 2-109, §2.6.92. The `ObjectDescriptorUpdate()` command is defined in ISO/IEC 14496-1 §7.2.5.5.2 (not vendored): carried opaque._

## Table 2-115 — HEVC timing and HRD descriptor (ext 0x03)
_§2.6.97–2.6.98; printed p.128 (PDF p.150). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| HEVC_timing_and_HRD_descriptor() { |  |  |
| &nbsp;&nbsp;hrd_management_valid_flag | 1 | bslbf |
| &nbsp;&nbsp;target_schedule_idx_not_present_flag | 1 | bslbf |
| &nbsp;&nbsp;target_schedule_idx | 5 | uimsbf |
| &nbsp;&nbsp;picture_and_timing_info_present_flag | 1 | bslbf |
| &nbsp;&nbsp;if (picture_and_timing_info_present_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;90kHz_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;if (90kHz_flag == '0') { N (32, uimsbf); K (32, uimsbf) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;num_units_in_tick | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

When `target_schedule_idx_not_present_flag` is `1` the following 5 bits are "unspecified" (§2.6.98).

## Table 2-116 — af_extensions_descriptor (ext 0x04)
_§2.6.99; printed p.129 (PDF p.151). Verified: the syntax is the empty `af_extensions_descriptor() { }`._

## Table 2-117 — HEVC operation point descriptor (ext 0x05)
_§2.6.100–2.6.101; printed p.130 (PDF p.152). Verified against the rendered page. **md correction:** the converted text printed the profile_tier_level loop as `for (i = 0; i < num_ptl; i++, i++)`; the rendered page reads `for (i = 0; i < num_ptl; i++)`._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| HEVC_operation_point_descriptor() { |  |  |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;num_ptl | 6 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < num_ptl; i++) { profile_tier_level_info[i] | 96 | bslbf } |  |  |
| &nbsp;&nbsp;operation_points_count | 8 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < operation_points_count; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;target_ols[i] | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ES_count[i] | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (j = 0; j < ES_count[i]; j++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;prepend_dependencies[i][j] | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ES_reference[i][j] | 6 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;numEsInOp[i] | 6 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (k = 0; k < NumESinOP[i]; k++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;necessary_layer_flag[i][k] | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;output_layer_flag[i][k] | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ptl_ref_idx[i][k] | 6 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;avg_bit_rate_info_flag[i] | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;max_bit_rate_info_flag[i] | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;constant_frame_rate_info_idc[i] | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;applicable_temporal_id[i] | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;if (constant_frame_rate_info_idc[i] > 0) { reserved (4, bslbf); frame_rate_indicator[i] (12, uimsbf) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;if (avg_bit_rate_info_flag[i] == '1') { avg_bit_rate[i] | 24 | uimsbf } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;if (max_bit_rate_info_flag[i] == '1') { max_bit_rate[i] | 24 | uimsbf } |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

> **Spec quirk (verbatim):** the page declares the count field as `numEsInOp[i]` but the loop bound reads `NumESinOP[i]`; they are the same quantity (the field is defined as `numEsInOp[i]` in §2.6.101 and the loop uses it).

`profile_tier_level_info[i]` is typed as `ProfileTierLevelInfo` (general_profile_space 2, general_tier_flag 1, general_profile_idc 5, 32 compatibility flags, progressive/interlaced/non_packed/frame_only 1 each, `copied_44bits` 44, general_level_idc 8 = 96 bits). `ES_count` 0xFF is reserved.

> **Spec note:** H.222.0 §2.6.101 (printed p.130, PDF p.152; verified on rendered page) fixes `profile_tier_level_info[i]` at 96 bits, "coded according to the syntax structure of profile_tier_level defined in 7.3.3 of Rec. ITU-T H.265 | ISO/IEC 23008-2 with the value of profilePresentFlag set equal to '1' and maxNumSubLayersMinus1 set equal to 6". H.265 §7.3.3 (V11 01/2026 edition, printed pp.42-43, PDF pp.60-61; verified on rendered pages) puts general_profile_space(2), tier(1), idc(5), compatibility flags(32) and four source/constraint flags(4), then 43 profile-dependent bits (`general_max_*` flags / `general_reserved_zero_*bits`, selected by `general_profile_idc` and the compatibility flags; the page states the total width is unaffected), one bit `general_inbld_flag` / `general_reserved_zero_bit`, and `general_level_idc`(8): 2+1+5+32+4+43+1+8 = 96 bits. With `maxNumSubLayersMinus1 = 6` the full H.265 syntax would continue with six `sub_layer_profile_present_flag`/`sub_layer_level_present_flag` pairs, `reserved_zero_2bits[6..8)` and the per-sub-layer structures, which exceed 96 bits; H.222.0 nevertheless fixes the field at 96 bits. This crate reproduces H.222.0 verbatim (the general part only) and does not invent the sub-layer part. The 44 bits after `general_frame_only_constraint_flag` are carried as `copied_44bits`, as for `HEVC_video_descriptor` (Table 2-112).

## Table 2-118 — HEVC hierarchy extension descriptor (ext 0x06)
_§2.6.102–2.6.103, Table 2-119; printed p.132 (PDF p.154). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| HEVC_hierarchy_extension_descriptor() { |  |  |
| &nbsp;&nbsp;extension_dimension_bits | 16 | bslbf |
| &nbsp;&nbsp;hierarchy_layer_index | 6 | uimsbf |
| &nbsp;&nbsp;temporal_id | 3 | uimsbf |
| &nbsp;&nbsp;nuh_layer_id | 6 | uimsbf |
| &nbsp;&nbsp;tref_present_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;num_embedded_layers | 6 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;hierarchy_channel | 6 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < num_embedded_layers; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;hierarchy_ext_embedded_layer_index[i] | 6 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

Table 2-119 `extension_dimension_bits` bit index: 0 multi-view, 1 spatial/SNR scalability, 2 depth, 3 temporal, 4 auxiliary, 5..15 reserved.

## Table 2-120 — Green extension descriptor (ext 0x07)
_§2.6.104–2.6.105; printed p.133 (PDF p.155). Verified against the rendered page. Semantics: ISO/IEC 23001-11 §6.4 (not vendored)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Green_extension_descriptor() { |  |  |
| &nbsp;&nbsp;num_constant_backlight_voltage_time_intervals | 2 | uimsbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;for (i=0; i < num_constant_backlight_voltage_time_intervals; i++) { constant_backlight_voltage_time_interval[i] | 16 | uimsbf } |  |  |
| &nbsp;&nbsp;num_max_variations | 2 | uimsbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;for (j=0; j < num_max_variations; j++) { max_variation[j] | 16 | uimsbf } |  |  |
| } |  |  |
