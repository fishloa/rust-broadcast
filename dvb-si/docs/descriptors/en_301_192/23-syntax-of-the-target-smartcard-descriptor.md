## Table 23 — Syntax of the target_smartcard_descriptor
_§8.4.5.3, PDF pp. 33-33_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_smartcard_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x06 |
| descriptor_length | 8 | uimsbf |  |
| super_CA_system_id | 32 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| private_data_byte | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

