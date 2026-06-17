## Table 21 — Bouquet name descriptor
_§6.2.6, PDF pp. 57-57_

| Syntax | Number of bits | Identifier |
|---|---|---|
| bouquet_name_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| } |

