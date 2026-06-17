## Table 2-64 — System clock descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.20, Table 2-64; PDF p.88._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| system_clock_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;external_clock_reference_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;clock_accuracy_integer | 6 | uimsbf |
| &nbsp;&nbsp;clock_accuracy_exponent | 3 | uimsbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| } |  |  |
