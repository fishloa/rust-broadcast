## Table 22 — CA identifier descriptor
_§6.2.6, PDF pp. 57-57_

| Syntax | Number of bits | Identifier |
|---|---|---|
| CA_identifier_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| CA_system_id | 16 | uimsbf |
| } |
| } |

