## Table 54 — Extension descriptor
_§6.2.18.1, PDF pp. 80-80_

| Syntax | Number of bits | Identifier |
|---|---|---|
| extension_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| selector_byte | 8 | bslbf |
| } |
| } |

