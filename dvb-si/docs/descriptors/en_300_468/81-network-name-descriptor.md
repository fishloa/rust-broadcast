## Table 81 — Network name descriptor
_§6.2.28, PDF pp. 96-96_

| Syntax | Number of bits | Identifier |
|---|---|---|
| network_name_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| } |

