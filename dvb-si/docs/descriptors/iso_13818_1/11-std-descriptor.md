## Table 2-70 — STD descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.32, Table 2-70; PDF p.91._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| STD_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;leak_valid_flag | 1 | bslbf |
| } |  |  |
