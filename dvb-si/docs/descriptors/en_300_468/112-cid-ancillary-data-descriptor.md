## Table 112 — CID ancillary data descriptor
_§6.4.3, PDF pp. 116-116_

| Syntax | Number of bits | Identifier |
|---|---|---|
| CI_ancillary_data_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| ancillary_data_byte | 8 | uimsbf |
| } |
| } |

