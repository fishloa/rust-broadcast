## Table 78 — Multilingual network name descriptor
_§6.2.24, PDF pp. 94-94_

| Syntax | Number of bits | Identifier |
|---|---|---|
| multilingual_network_name_descriptor() { |
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

