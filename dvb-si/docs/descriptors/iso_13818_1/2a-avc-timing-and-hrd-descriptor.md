## Table 2-93 — AVC timing and HRD descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.66, Table 2-93; PDF p.107._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| AVC_timing_and_HRD_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;hrd_management_valid_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;picture_and_timing_info_present | 1 | bslbf |
| &nbsp;&nbsp;if (picture_and_timing_info_present) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;90kHz_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;if (90kHz_flag == '0') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;N | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;K | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;num_units_in_tick | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;fixed_frame_rate_flag | 1 | bslbf |
| &nbsp;&nbsp;temporal_poc_flag | 1 | bslbf |
| &nbsp;&nbsp;picture_to_display_conversion_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| } |  |  |
