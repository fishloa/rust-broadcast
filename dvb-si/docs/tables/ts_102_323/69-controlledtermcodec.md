## Table 69 — ControlledTermCodec
_§9.4.3.7.3, PDF pp. 72-72_

| Name | No. of bits | Identifier |
|---|---|---|
| dvbControlledTermCodec () { | | |
| encoding flag | 1 | bslbf |
| if (encoding flag == 0) { | | |
| dvbStringCodec() | | |
| } | | |
| if (encoding flag == 1) { | | |
| grouping flag | 1 | bslbf |
| if (grouping flag == 0) { | | |
| ClassificationSchemeID | 7 | uimsbf |
| } else { | | |
| ClassificationSchemeGroupID | 4+ | vluimsbf4 |
| ClassificationSchemeIndex | 8+ | vluimsbf8 |
| } | | |
| termID | 8+ | vluimsbf8 |
| } | | |
| } | | |

