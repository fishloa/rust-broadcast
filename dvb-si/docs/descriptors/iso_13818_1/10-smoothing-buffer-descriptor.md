## Table 2-69 — Smoothing buffer descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.30, Table 2-69; PDF p.90._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| smoothing_buffer_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;sb_leak_rate | 22 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;sb_size | 22 | uimsbf |
| } |  |  |

`sb_leak_rate` units: 400 bits/s. `sb_size` units: 1 byte (§2.6.31).
