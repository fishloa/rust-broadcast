## Table 35 — Extended On-demand_decomposed_binary_locator
_§7.3.2.3.6, PDF pp. 46-46_

| Syntax | No. of bits | Identifier |
|---|---|---|
| extended_on-demand_decomposed_binary_locator() { | | |
| availability_start_date | 9 | uimsbf |
| availability_end_date | 9 | uimsbf |
| availability_start_time | 16 | uimsbf |
| availability_end_time | 16 | uimsbf |
| delivery_mode | 1 | bslbf |
| content_version | 8 | uimsbf |
| Early_playout | 1 | bslbf |
| expiry_time | 16 | uimsbf |
| expiry_date | 9 | uimsbf |
| URI_length | 12 | uimsbf |
| for (i=0; i<URI_length; i++) { | | |
| URI_byte | 8 | uimsbf |
| } | | |
| } | | |

