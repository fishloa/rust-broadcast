## Table 31 — Data broadcast descriptor
_§6.2.12, PDF pp. 71-71_

| Syntax | Number of bits | Identifier |
|---|---|---|
| data_broadcast_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| data_broadcast_id | 16 | uimsbf |
| component_tag | 8 | uimsbf |
| selector_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| selector_byte | 8 | uimsbf |
| } |
| ISO_639_language_code | 24 | bslbf |
| text_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| } |

