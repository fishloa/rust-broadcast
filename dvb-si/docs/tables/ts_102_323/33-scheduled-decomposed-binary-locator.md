## Table 33 — Scheduled_decomposed_binary_locator
_§7.3.2.3.4, PDF pp. 44-44_

| Syntax | No. of bits | Identifier |
|---|---|---|
| scheduled decomposed binary locator() { | | |
| scheduled time reliability | 1 | bslbf |
| reserved | 6 | bslbf |
| start date | 9 | uimsbf |
| start time | 16 | uimsbf |
| duration | 16 | uimsbf |
| if (scheduled time reliability == '1') { | | |
| early start window | 3 | uimsbf |
| late end window | 5 | uimsbf |
| } | | |
| reserved | 4 | bslbf |
| URI length | 12 | uimsbf |
| for (i=0; i<URI length; i++) { | | |
| URI byte | 8 | uimsbf |
| } | | |
| } | | |

