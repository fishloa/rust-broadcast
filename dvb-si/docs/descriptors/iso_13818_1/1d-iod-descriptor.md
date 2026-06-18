## Table 2-75 — IOD descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.40, Table 2-75; PDF p.95._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| IOD_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;Scope_of_IOD_label | 8 | uimsbf |
| &nbsp;&nbsp;IOD_label | 8 | uimsbf |
| &nbsp;&nbsp;InitialObjectDescriptor() |  |  |
| } |  |  |

`InitialObjectDescriptor()` is defined in §8.6.3.1 of ISO/IEC 14496-1 — carried here as opaque bytes (the remainder of the descriptor).
