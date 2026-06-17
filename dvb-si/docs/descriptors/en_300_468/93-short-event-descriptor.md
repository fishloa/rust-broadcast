## Table 93 — Short event descriptor
_§6.2.37, PDF pp. 102-102_

| Syntax | Number of bits | Identifier |
|---|---|---|
| short_event_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| ISO_639_language_code | 24 | bslbf |
| name_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| text_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| text_char | 8 | uimsbf |
| } |
| } |

