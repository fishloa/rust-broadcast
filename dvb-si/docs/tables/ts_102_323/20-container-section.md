## Table 20 — Container section
_§7.3.1.4, PDF pp. 36-36_

| Syntax | No. of bits | Identifier |
|---|---|---|
| container section() { | | |
| table id | 8 | uimsbf |
| section syntax indicator | 1 | bslbf |
| private indicator | 1 | bslbf |
| reserved | 2 | bslbf |
| private section length | 12 | uimsbf |
| container id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version number | 5 | uimsbf |
| current next indicator | 1 | bslbf |
| section number | 8 | uimsbf |
| last section number | 8 | uimsbf |
| container data() | | |
| CRC32 | 32 | uimsbf |
| } | | |

