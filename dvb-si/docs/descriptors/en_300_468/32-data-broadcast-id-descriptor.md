## Table 32 — Data broadcast id descriptor
_§6.2.13.1, PDF pp. 72-72_

| Syntax | Number of bits | Identifier |
|---|---|---|
| data_broadcast_id_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| data_broadcast_id | 16 | uimsbf |
| for (i=0;i<N;i++) { |
| selector_byte | 8 | uimsbf |
| } |
| } |

