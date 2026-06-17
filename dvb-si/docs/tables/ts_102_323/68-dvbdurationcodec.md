## Table 68 — dvbDurationCodec
_§9.4.3.7.2, PDF pp. 71-71_

| Name | No. of bits | Identifier |
|---|---|---|
| dvbDurationCodec() { | | |
| encoding flag | 1 | bslbf |
| if (encoding flag == 0) { | | |
| dvbStringCodec() | | |
| } | | |
| if (encoding flag == 1) { | | |
| minutes | 11 | uimsbf |
| } | | |
| } | | |

