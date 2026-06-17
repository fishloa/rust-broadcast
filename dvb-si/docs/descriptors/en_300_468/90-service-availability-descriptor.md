## Table 90 — Service availability descriptor
_§6.2.36, PDF pp. 101-101_

| Syntax | Number of bits | Identifier |
|---|---|---|
| service_availability_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| availability_flag | 1 | bslbf |
| reserved_future_use | 7 | bslbf |
| for (i=0;i<N;i++) { |
| cell_id | 16 | uimsbf |
| } |
| } |

