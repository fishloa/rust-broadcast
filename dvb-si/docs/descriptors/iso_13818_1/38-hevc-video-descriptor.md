## Table 2-113 — HEVC video descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.95, Table 2-113; PDF p.125. HDR_WCG_idc values per Table 2-114._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| HEVC_video_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;profile_space | 2 | uimsbf |
| &nbsp;&nbsp;tier_flag | 1 | bslbf |
| &nbsp;&nbsp;profile_idc | 5 | uimsbf |
| &nbsp;&nbsp;profile_compatibility_indication | 32 | bslbf |
| &nbsp;&nbsp;progressive_source_flag | 1 | bslbf |
| &nbsp;&nbsp;interlaced_source_flag | 1 | bslbf |
| &nbsp;&nbsp;non_packed_constraint_flag | 1 | bslbf |
| &nbsp;&nbsp;frame_only_constraint_flag | 1 | bslbf |
| &nbsp;&nbsp;copied_44bits | 44 | bslbf |
| &nbsp;&nbsp;level_idc | 8 | uimsbf |
| &nbsp;&nbsp;temporal_layer_subset_flag | 1 | bslbf |
| &nbsp;&nbsp;HEVC_still_present_flag | 1 | bslbf |
| &nbsp;&nbsp;HEVC_24hr_picture_present_flag | 1 | bslbf |
| &nbsp;&nbsp;sub_pic_hrd_params_not_present_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;HDR_WCG_idc | 2 | bslbf |
| &nbsp;&nbsp;if (temporal_layer_subset_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;temporal_id_min | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;temporal_id_max | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |
