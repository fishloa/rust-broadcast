## Table 14 — RNT scan descriptor
_§5.3.8.1, PDF pp. 26-26_

| Syntax | No. of bits | Identifier |
|---|---|---|
| RNT scan descriptor() { | | |
| descriptor tag | 8 | uimsbf |
| descriptor length | 8 | uimsbf |
| for (i=0; i<N; i++) { | | |
| transport stream id | 16 | uimsbf |
| original network id | 16 | uimsbf |
| scan weighting | 8 | uimsbf |
| } | | |
| } | | |

