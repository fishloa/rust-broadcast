## Table 2-65 — Multiplex buffer utilization descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.22, Table 2-65; PDF p.88._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Multiplex_buffer_utilization_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;bound_valid_flag | 1 | bslbf |
| &nbsp;&nbsp;LTW_offset_lower_bound | 15 | uimsbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;LTW_offset_upper_bound | 15 | uimsbf |
| } |  |  |
