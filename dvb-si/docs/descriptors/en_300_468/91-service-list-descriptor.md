## Table 91 — Service list descriptor
_§6.2.36, PDF pp. 101-101_

| Syntax | Number of bits | Identifier |
|---|---|---|
| service_list_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| service_id | 16 | uimsbf |
| service_type | 8 | uimsbf |
| } |
| } |

