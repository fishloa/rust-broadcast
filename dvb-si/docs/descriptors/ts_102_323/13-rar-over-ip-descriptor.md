## Table 13 — RAR over IP descriptor
_§5.3.6, PDF pp. 25-25_

| Syntax | No. of bits | Identifier |
|---|---|---|
| RAR over IP descriptor() { | | |
| descriptor tag | 8 | uimsbf |
| descriptor length | 8 | uimsbf |
| first valid date | 40 | bslbf |
| last valid date | 40 | bslbf |
| weighting | 6 | uimsbf |
| complete flag | 1 | bslbf |
| reserved | 1 | bslbf |
| url length | 8 | uimsbf |
| for (i=0; i < url length; i++) { | | |
| url char | 8 | uimsbf |
| } | | |
| } | | |

