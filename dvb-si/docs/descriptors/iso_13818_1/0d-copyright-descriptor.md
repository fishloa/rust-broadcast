## Table 2-66 — Copyright descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.24, Table 2-66; PDF p.89._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| copyright_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;copyright_identifier | 32 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < N; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;additional_copyright_info | 8 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |
