## Table 98 — Stuffing descriptor
_§6.2.40, PDF pp. 105-105_

| Syntax | Number of bits | Identifier |
|---|---|---|
| stuffing_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| stuffing_byte | 8 | bslbf |
| } |
| } |

