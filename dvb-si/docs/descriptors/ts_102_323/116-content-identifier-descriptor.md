## Table 116 — Content identifier descriptor
_§12.1.4, PDF pp. 103-103_

| Syntax | No. of bits | Identifier |
|---|---|---|
| content identifier descriptor() { | | |
| descriptor tag | 8 | uimsbf |
| descriptor length | 8 | uimsbf |
| for (i=0;i<N;i++) { | | |
| crid type | 6 | uimsbf |
| crid location | 2 | uimsbf |
| if (crid location == '00' ) { | | |
| crid length | 8 | uimsbf |
| for (j=0;j<crid length;j++) { | | |
| crid byte | 8 | uimsbf |
| } | | |
| } | | |
| if (crid location == '01' ) { | | |
| crid ref | 16 | uimsbf |
| } | | |
| } | | |
| } | | |

