## Table 28 — Transport protocol descriptor syntax
_§5.3.6.1, PDF pp. 45-45_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| transport_protocol_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x02 |
| descriptor_length | 8 | uimsbf |  |
| protocol_id | 16 | uimsbf |  |
| transport_protocol_label | 8 | uimsbf |  |
| for(i=0; i<N; i++) { |  |  |  |
| selector_byte | 8 | uimsbf | N1 |
| } |  |  |  |
| } |  |  |  |

