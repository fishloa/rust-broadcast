## Table 119 — Content identifier section
_§12.2, PDF pp. 105-105_

| Syntax | No. of bits | Identifier |
|---|---|---|
| Content identifier section() { | | |
| table id | 8 | uimsbf |
| section syntax indicator | 1 | bslbf |
| private indicator | 1 | bslbf |
| reserved | 2 | bslbf |
| section length | 12 | uimsbf |
| service id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version number | 5 | uimsbf |
| current next indicator | 1 | bslbf |
| section number | 8 | uimsbf |
| last section number | 8 | uimsbf |
| transport stream id | 16 | uimsbf |
| original network id | 16 | uimsbf |
| prepend strings length | 8 | uimsbf |
| for (i=0; i< prepend strings length ; i++) { | | |
| prepend strings byte | 8 | uimsbf |
| } | | |
| for (j=0; j<N; j++) { | | |
| crid ref | 16 | uimsbf |
| prepend string index | 8 | uimsbf |
| unique string length | 8 | uimsbf |
| for (k=0; k<unique string length; k++) { | | |
| unique string byte | 8 | uimsbf |
| } | | |
| } | | |
| CRC32 | 32 | rpchof |
| } | | |
