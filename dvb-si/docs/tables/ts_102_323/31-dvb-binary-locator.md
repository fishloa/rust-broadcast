## Table 31 — DVB_binary_locator
_§7.3.2.3.3, PDF pp. 42-42_

| Syntax | No. of bits | Identifier |
|---|---|---|
| dvb binary locator() { | | |
| identifier type | 2 | bslbf |
| scheduled time reliability | 1 | bslbf |
| inline service | 1 | bslbf |
| reserved | 1 | bslbf |
| start date | 9 | uimsbf |
| if (inline service == '0') { | | |
| DVB service triplet ID | 10 | uimsbf |
| } else { | | |
| reserved | 2 | bslbf |
| transport stream id | 16 | uimsbf |
| original network id | 16 | uimsbf |
| service id | 16 | uimsbf |
| } | | |
| start time | 16 | uimsbf |
| duration | 16 | uimsbf |
| if (identifier type == '01') { | | |
| event id | 16 | uimsbf |
| } | | |
| if (identifier type == '10') { | | |
| TVA id | 16 | uimsbf |
| } | | |
| if (identifier type == '11') { | | |
| TVA id | 16 | uimsbf |
| component | 8 | uimsbf |
| } | | |
| if (identifier type == '00' && scheduled time reliability == '1')) { | | |
| early start window | 3 | uimsbf |
| late end window | 5 | uimsbf |
| } | | |
| } | | |

