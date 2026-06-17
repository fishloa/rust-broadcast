## Table 20 — Syntax of the target_smartcard_descriptor
_§9.5.2, PDF pp. 26-26_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | Bits | | |
| target_smartcard_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x06 |
| descriptor_length | 8 | uimsbf | |
| super_CA_system_id | 32 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| private_data_byte | 8 | uimsbf | |
| } | | | |
| } | | | |

