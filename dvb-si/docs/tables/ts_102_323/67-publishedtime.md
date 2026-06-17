## Table 67 — PublishedTime
_§9.4.3.6.1, PDF pp. 70-70_

| Name | No. of bits | Identifier |
|---|---|---|
| PublishedTime() { | | |
| date flag | 1 | bslbf |
| if (date flag == 1) { | | |
| date | 16 | bslbf |
| } | | |
| time | 11 | uimsbf |
| } | | |

