## Table 2-133 — VVC video descriptor (tag 0x39)
_Rec. ITU-T H.222.0 (08/2023) §2.6.129–2.6.130, Tables 2-133..2-138; printed p.151 (PDF p.173). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| VVC_video_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;profile_idc | 7 | uimsbf |
| &nbsp;&nbsp;tier_flag | 1 | bslbf |
| &nbsp;&nbsp;num_sub_profiles | 8 | uimsbf |
| &nbsp;&nbsp;for (i=0; i<num_sub_profiles; i++) { sub_profile_idc[i] | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;progressive_source_flag | 1 | bslbf |
| &nbsp;&nbsp;interlaced_source_flag | 1 | bslbf |
| &nbsp;&nbsp;non_packed_constraint_flag | 1 | bslbf |
| &nbsp;&nbsp;frame_only_constraint_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved_zero_4bits | 4 | bslbf |
| &nbsp;&nbsp;level_idc | 8 | uimsbf |
| &nbsp;&nbsp;temporal_layer_subset_flag | 1 | bslbf |
| &nbsp;&nbsp;VVC_still_present_flag | 1 | bslbf |
| &nbsp;&nbsp;VVC_24hr_picture_present_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;HDR_WCG_idc | 2 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;video_properties_tag | 4 | uimsbf |
| &nbsp;&nbsp;if (temporal_layer_subset_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;temporal_id_min | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;temporal_id_max | 3 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

`reserved_zero_4bits` shall be `0000`. `video_properties_tag` (4 bits) is interpreted through Tables 2-135 (SDR), 2-136 (WCG), 2-137 (HDR/WCG) or 2-138 (no indication) selected by `HDR_WCG_idc`.

## Table 2-140 — EVC video descriptor (tag 0x3A)
_§2.6.133–2.6.134, Table 2-140; printed p.156 (PDF p.178). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| EVC_video_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 16 | uimsbf |
| &nbsp;&nbsp;profile_idc | 8 | uimsbf |
| &nbsp;&nbsp;level_idc | 8 | uimsbf |
| &nbsp;&nbsp;toolset_idc_h | 32 | uimsbf |
| &nbsp;&nbsp;toolset_idc_l | 32 | uimsbf |
| &nbsp;&nbsp;progressive_source_flag | 1 | bslbf |
| &nbsp;&nbsp;interlaced_source_flag | 1 | bslbf |
| &nbsp;&nbsp;non_packed_constraint_flag | 1 | bslbf |
| &nbsp;&nbsp;frame_only_constraint_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;temporal_layer_subset_flag | 1 | bslbf |
| &nbsp;&nbsp;EVC_still_present_flag | 1 | bslbf |
| &nbsp;&nbsp;EVC_24hr_picture_present_flag | 1 | bslbf |
| &nbsp;&nbsp;HDR_WCG_idc | 2 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;video_properties_tag | 4 | uimsbf |
| &nbsp;&nbsp;if (temporal_layer_subset_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;temporal_id_min | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;temporal_id_max | 3 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

> **Spec erratum:** Table 2-140 (printed p.156, PDF p.178) prints `descriptor_length` as "16" bits for `EVC_video_descriptor()`, whereas every other descriptor (including `VVC_video_descriptor()` in Table 2-133) uses the 8-bit `descriptor_length` of the common descriptor framing. The table above reproduces the page verbatim. The crate keeps the standard 8-bit descriptor framing and types the body fields only; TSDuck 3.44 also encodes an 8-bit length.
