## Table 28 — Content descriptor
_§6.2.9, PDF pp. 68-68_

| Syntax | Number of bits | Identifier |
|---|---|---|
| content_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| content_nibble_level_1 | 4 | uimsbf |
| content_nibble_level_2 | 4 | uimsbf |
| user_byte | 8 | uimsbf |
| } |
| } |

