## Table 114 — TVA_id descriptor
_§11.2.4, PDF pp. 101-101_

| Syntax | No. of bits | Identifier |
|---|---|---|
| TVA id descriptor() { | | |
| descriptor tag | 8 | uimsbf |
| descriptor length | 8 | uimsbf |
| for (i=0; i<N; i++) { | | |
| TVA id | 16 | uimsbf |
| Reserved | 5 | uimsbf |
| running status | 3 | uimsbf |
| } | | |
| } | | |

