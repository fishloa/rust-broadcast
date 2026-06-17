## Table 25 — Component descriptor
_§6.2.8, PDF pp. 60-60_

| Syntax | Number of bits | Identifier |
|---|---|---|
| component_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| stream_content_ext | 4 | uimsbf |
| stream_content | 4 | uimsbf |
| component_type | 8 | uimsbf |
| component_tag | 8 | uimsbf |
| ISO_639_language_code | 24 | bslbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| } |

