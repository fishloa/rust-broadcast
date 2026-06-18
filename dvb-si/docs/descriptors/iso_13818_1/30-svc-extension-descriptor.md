## Table 2-99 — SVC extension descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.76, Table 2-99; PDF p.111._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| SVC_extension_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;width | 16 | uimsbf |
| &nbsp;&nbsp;height | 16 | uimsbf |
| &nbsp;&nbsp;frame_rate | 16 | uimsbf |
| &nbsp;&nbsp;average_bitrate | 16 | uimsbf |
| &nbsp;&nbsp;maximum_bitrate | 16 | uimsbf |
| &nbsp;&nbsp;dependency_id | 3 | bslbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;quality_id_start | 4 | bslbf |
| &nbsp;&nbsp;quality_id_end | 4 | bslbf |
| &nbsp;&nbsp;temporal_id_start | 3 | bslbf |
| &nbsp;&nbsp;temporal_id_end | 3 | bslbf |
| &nbsp;&nbsp;no_sei_nal_unit_present | 1 | bslbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| } |  |  |
