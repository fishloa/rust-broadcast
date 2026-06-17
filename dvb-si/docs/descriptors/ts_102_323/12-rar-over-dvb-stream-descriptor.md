## Table 12 — RAR over DVB Stream descriptor
_§5.3.5, PDF pp. 24-24_

| Syntax | No. of bits | Identifier |
|---|---|---|
| RAR over DVB stream descriptor() { | | |
| descriptor tag | 8 | uimsbf |
| descriptor length | 8 | uimsbf |
| first valid date | 40 | bslbf |
| last valid date | 40 | bslbf |
| weighting | 6 | uimsbf |
| complete flag | 1 | bslbf |
| scheduled flag | 1 | bslbf |
| transport stream id | 16 | uimsbf |
| original network id | 16 | uimsbf |
| service id | 16 | uimsbf |
| component tag | 8 | uimsbf |
| if (scheduled flag == 1) { | | |
| download start time | 40 | bslbf |
| download period duration | 8 | uimsbf |
| download cycle time | 8 | uimsbf |
| } | | |
| } | | |

