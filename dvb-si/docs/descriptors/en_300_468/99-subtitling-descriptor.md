## Table 99 — Subtitling descriptor
_§6.2.42, PDF pp. 106-106_

| Syntax | Number of bits | Identifier |
|---|---|---|
| subtitling_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| ISO_639_language_code | 24 | bslbf |
| subtitling_type | 8 | bslbf |
| composition_page_id | 16 | bslbf |
| ancillary_page_id | 16 | bslbf |
| } |
| } |

