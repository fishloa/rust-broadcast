## Table 109 — Related content section
_§10.4.2, PDF pp. 96-96_

| Syntax | No. of bits | Identifier |
|---|---|---|
| related content section() { | | |
| table id | 8 | uimsbf |
| section syntax indicator | 1 | bslbf |
| table id extension flag | 1 | bslbf |
| reserved | 2 | bslbf |
| section length | 12 | uimsbf |
| service id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version number | 5 | uimsbf |
| current next indicator | 1 | bslbf |
| section number | 8 | uimsbf |
| last section number | 8 | uimsbf |
| year offset | 16 | uimsbf |
| link count | 8 | uimsbf |
| for (j=0; j<link count; j++) { | | |
| reserved | 4 | uimsbf |
| link info length | 12 | uimsbf |
| link info() | | |
| } | | |
| reserved future use | 4 | bslbf |
| descriptor loop length | 12 | uimsbf |
| for (k=0; k<descriptor loop length; k++) { | | |
| descriptor() | | |
| } | | |
| CRC 32 | 32 | rpchof |
| } | | |

