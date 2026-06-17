## Table 34 — On-demand_decomposed_binary_locator
_§7.3.2.3.5, PDF pp. 45-45_

| Syntax | No. of bits | Identifier |
|---|---|---|
| on-demand decomposed binary locator() { | | |
| reserved | 6 | bslbf |
| availability_start_date | 9 | uimsbf |
| availability_end_date | 9 | uimsbf |
| availability_start_time | 16 | uimsbf |
| availability_end_time | 16 | uimsbf |
| reserved | 4 | bslbf |
| URI_length | 12 | uimsbf |
| for (i=0; i<URI_length; i++) { | | |
| URI_byte | 8 | uimsbf |
| } | | |
| } | | |

