## Table 1 — Resolution Provider Notification Section
_§5.2.2, PDF pp. 17-17_

| Name | No. of bits | Identifier |
|---|---|---|
| resolution authority notification section() { | | |
| table id | 8 | uimsbf |
| section syntax indicator | 1 | bslbf |
| reserved | 1 | bslbf |
| reserved | 2 | bslbf |
| section length | 12 | uimsbf |
| context id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version number | 5 | uimsbf |
| current next indicator | 1 | bslbf |
| section number | 8 | uimsbf |
| last section number | 8 | uimsbf |
| context id type | 8 | uimsbf |
| reserved | 4 | bslbf |
| common descriptors length | 12 | uimsbf |
| for (i=0; i<N1; i++) { | | |
| descriptor() | | |
| } | | |
| for (i<0; i<N2; i++) { | | |
| reserved | 4 | bslbf |
| resolution provider info length | 12 | uimsbf |
| resolution provider name length | 8 | uimsbf |
| for (j<0; j<resolution provider name length; j++) { | | |
| resolution provider name byte | 8 | uimsbf |
| } | | |
| reserved | 4 | bslbf |
| resolution provider descriptors length | 12 | uimsbf |
| for (j=0; j<N3; j++) { | | |
| descriptor() | | |
| } | | |
| for (j=0; j<N4; j++) { | | |
| CRID authority name length | 8 | uimsbf |
| for (k<0; k<CRID authority name length; k++) { | | |
| CRID authority name byte | 8 | uimsbf |
| } | | |
| reserved | 2 | bslbf |
| CRID authority policy | 2 | bslbf |
| CRID authority descriptors length | 12 | uimsbf |
| for (k=0; k<N5; k++) { | | |
| CRID authority descriptor() | | |
| } | | |
| } | | |
| } | | |
| CRC 32 | 32 | rpchof |
| } | | |

