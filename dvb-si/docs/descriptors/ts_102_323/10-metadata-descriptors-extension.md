## Table 10 — Metadata descriptors extension
_§5.3.4.2, PDF pp. 23-23_

| Name | No. of bits | Identifier |
|---|---|---|
| metadata descriptors extension() { | | |
| DVB carriage format | 4 | uimsbf |
| reserved | 2 | uimsbf |
| metadata service identifier flag | 1 | bslbf |
| fragment types flag | 1 | bslbf |
| if (fragment types flag == '0') { | | |
| number of types | 8 | uimsbf |
| for (i=0; i<number of types; i++) { | | |
| fragment type | 16 | uimsbf |
| } | | |
| } | | |
| if (metadata service identifier flag == '0') { | | |
| metadata service identifier length | 8 | uimsbf |
| for (i=0; i<metadata service identifier length; i++) { | | |
| metadata service identifier byte | 8 | uimsbf |
| } | | |
| } | | |
| for (i=0; i<N; i++) { | | |
| user data byte | 8 | bslbf |
| } | | |
| } | | |

