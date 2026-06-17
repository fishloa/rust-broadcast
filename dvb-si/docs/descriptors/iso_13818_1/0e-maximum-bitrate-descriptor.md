## Table 2-67 — Maximum bitrate descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.26, Table 2-67; PDF p.89._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| maximum_bitrate_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;maximum_bitrate | 22 | uimsbf |
| } |  |  |

`maximum_bitrate` is in units of 50 bytes/second (§2.6.27).
