## Table 113 — CP descriptor
_§6.4.3, PDF pp. 116-116_

| Syntax | Number of bits | Identifier |
|---|---|---|
| CP_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| CP_system_id | 16 | uimsbf |
| reserved_future_use | 3 | bslbf |
| CP_PID | 13 | uimsbf |
| for (i=0;i<N;i++) { |
| private_data_byte | 8 | uimsbf |
| } |
| } |

