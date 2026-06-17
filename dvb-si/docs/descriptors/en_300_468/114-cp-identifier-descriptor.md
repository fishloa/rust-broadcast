## Table 114 — CP identifier descriptor
_§6.4.6.1, PDF pp. 117-117_

| Syntax | Number of bits | Identifier |
|---|---|---|
| CP_identifier_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| CP_system_id | 16 | uimsbf |
| } |
| } |

