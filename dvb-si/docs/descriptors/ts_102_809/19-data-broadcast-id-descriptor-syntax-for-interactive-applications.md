## Table 19 — data_broadcast_id_descriptor syntax for interactive applications
_§5.3.5.3, PDF pp. 38-38_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| data_broadcast_id_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x66 |
| descriptor_length | 8 | uimsbf |  |
| data_broadcast_id | 16 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| reserved_future_use | 1 |  |  |
| application_type | 15 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

