## Table 105 — Type list structure
_§9.5.2.2, PDF pp. 90-90_

| Syntax | No. of bits | Identifier |
|---|---|---|
| type list structure() { | | |
| num types; | 16 | uimsbf |
| for (i=0; i<num types; i++) { | | |
| reserved | 4 | uimsbf |
| type description length | 12 | uimsbf |
| fragment type | 16 | uimsbf |
| if (fragment type == 0xFFFF) { | | |
| fragment xpath ptr | 16 | uimsbf |
| } | | |
| num containers | 8 | uimsbf |
| for (j=0; j< num containers; j++) { | | |
| container id | 16 | uimsbf |
| } | | |
| } | | |
| } | | |

