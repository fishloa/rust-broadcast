## Table 2-91 — Metadata STD descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.62, Table 2-91; PDF p.105._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Metadata_STD_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;metadata_input_leak_rate | 22 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;metadata_buffer_size | 22 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;metadata_output_leak_rate | 22 | uimsbf |
| } |  |  |

Leak rates in units of 400 bits/s; buffer size in units of 1024 bytes (§2.6.63).
