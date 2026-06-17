## Table 76 — Multilingual bouquet name descriptor
_§6.2.22, PDF pp. 93-93_

| Syntax | Number of bits | Identifier |
|---|---|---|
| multilingual_bouquet_name_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| ISO_639_language_code | 24 | bslbf |
| name_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| char | 8 | uimsbf |
| } |
| } |
| } |

