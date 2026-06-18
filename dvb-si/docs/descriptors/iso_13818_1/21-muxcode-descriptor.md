## Table 2-79 — Muxcode descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.48, Table 2-79; PDF p.97._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Muxcode_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < N; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;MuxCodeTableEntry() |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

`MuxCodeTableEntry()` is defined in §11.2.4.3 of ISO/IEC 14496-1 — carried here as opaque bytes.
