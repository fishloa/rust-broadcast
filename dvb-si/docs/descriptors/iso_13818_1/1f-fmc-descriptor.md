## Table 2-77 — FMC descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.44, Table 2-77; PDF p.96._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| FMC_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < descriptor_length; i += 3) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;ES_ID | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;FlexMuxChannel | 8 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |
