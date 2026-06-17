## Table 148 — Message descriptor
_§6.4.9, PDF pp. 137-137_

| Syntax | Number of bits | Identifier |
|---|---|---|
| message_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| message_id | 8 | uimsbf |
| ISO_639_language_code | 24 | bslbf |
| for (i=0;i<N;i++) { |
| text_char | 8 | uimsbf |
| } |
| } |

