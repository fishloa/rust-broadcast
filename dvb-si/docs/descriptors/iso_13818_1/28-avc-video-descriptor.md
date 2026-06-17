## Table 2-92 — AVC video descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.64, Table 2-92; PDF pp.105-106._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| AVC_video_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;profile_idc | 8 | uimsbf |
| &nbsp;&nbsp;constraint_set0_flag | 1 | bslbf |
| &nbsp;&nbsp;constraint_set1_flag | 1 | bslbf |
| &nbsp;&nbsp;constraint_set2_flag | 1 | bslbf |
| &nbsp;&nbsp;constraint_set3_flag | 1 | bslbf |
| &nbsp;&nbsp;constraint_set4_flag | 1 | bslbf |
| &nbsp;&nbsp;constraint_set5_flag | 1 | bslbf |
| &nbsp;&nbsp;AVC_compatible_flags | 2 | bslbf |
| &nbsp;&nbsp;level_idc | 8 | uimsbf |
| &nbsp;&nbsp;AVC_still_present | 1 | bslbf |
| &nbsp;&nbsp;AVC_24_hour_picture_flag | 1 | bslbf |
| &nbsp;&nbsp;Frame_Packing_SEI_not_present_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| } |  |  |
