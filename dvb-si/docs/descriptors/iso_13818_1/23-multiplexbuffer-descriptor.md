## Table 2-81 — MultiplexBuffer descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.52, Table 2-81; PDF p.98._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MultiplexBuffer_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;MB_buffer_size | 24 | uimsbf |
| &nbsp;&nbsp;TB_leak_rate | 24 | uimsbf |
| } |  |  |

`TB_leak_rate` is in units of 400 bits/s; `MB_buffer_size` in bytes (§2.6.53).
