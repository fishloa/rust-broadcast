## Table 18 — Generic data broadcast id descriptor syntax
_§5.3.5.2.1, PDF pp. 37-37_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| data_broadcast_id_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x66 |
| descriptor_length | 8 | uimsbf |  |
| data_broadcast_id | 16 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| id specific data | 8 | bslbf |  |
| } |  |  |  |
| } |  |  |  |

