## Table 106 — Namespace map structure
_§9.6, PDF pp. 91-93_

| Syntax | No. of bits | Identifier |
|---|---|---|
| namespace map structure() { | | |
| num prefixes; | 8 | uimsbf |
| for (i=0; i<num prefixes; i++) { | | |
| prefix length | 8 | uimsbf |
| for (j=0; j< prefix length; j++) { | | uimsbf |
| prefix char | 8 | uimsbf |
| } | | |
| namespace length | 8 | uimsbf |
| for (j=0; j< namespace length; j++) { | | |
| namespace char | 8 | uimsbf |
| } | | |
| } | | |
| } | | |

