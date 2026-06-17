## Table 2-46 — Video stream descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.2, Table 2-46; PDF p.78._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| video_stream_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;multiple_frame_rate_flag | 1 | bslbf |
| &nbsp;&nbsp;frame_rate_code | 4 | uimsbf |
| &nbsp;&nbsp;MPEG_1_only_flag | 1 | bslbf |
| &nbsp;&nbsp;constrained_parameter_flag | 1 | bslbf |
| &nbsp;&nbsp;still_picture_flag | 1 | bslbf |
| &nbsp;&nbsp;if (MPEG_1_only_flag == '0') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;profile_and_level_indication | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;chroma_format | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;frame_rate_extension_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |
