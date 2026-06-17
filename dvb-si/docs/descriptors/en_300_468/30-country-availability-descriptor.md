## Table 30 — Country availability descriptor
_§6.2.10, PDF pp. 70-70_

| Syntax | Number of bits | Identifier |
|---|---|---|
| country_availability_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| country_availability_flag | 1 | bslbf |
| reserved_future_use | 7 | bslbf |
| for (i=0;i<N;i++) { |
| country_code | 24 | bslbf |
| } |
| } |

