## Table 82 — Parental rating descriptor
_§6.2.30, PDF pp. 97-97_

| Syntax | Number of bits | Identifier |
|---|---|---|
| parental_rating_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| country_code | 24 | bslbf |
| rating | 8 | uimsbf |
| } |
| } |

