## Table 17 — Application signalling descriptor syntax
_§5.3.5.2.1, PDF pp. 37-37_

|  | No. of bits | Identifier |
|---|---|---|
| application_signalling_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for( i=0; i<N; i++ ){ |  |  |
| reserved_future_use | 1 |  |
| application_type | 15 | uimsbf |
| reserved_future_use | 3 | bslbf |
| AIT_version_number | 5 | uimsbf |
| } |  |  |
| } |  |  |

