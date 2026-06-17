## Table 2-71 — IBP descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.34, Table 2-71; PDF p.91._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| ibp_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;closed_gop_flag | 1 | uimsbf |
| &nbsp;&nbsp;identical_gop_flag | 1 | uimsbf |
| &nbsp;&nbsp;max_gop-length | 14 | uimsbf |
| } |  |  |

`max_gop-length` value '0' is forbidden (§2.6.35).
