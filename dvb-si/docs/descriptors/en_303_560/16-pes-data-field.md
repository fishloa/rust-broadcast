## Table 16 — PES data field
_§5.2.2.2.1, PDF pp. 20-20_

| Syntax | Number of bits | Identifier |
|---|---|---|
| PES_data_field() { | | |
| segment_mediatime | 48 | uimsbf |
| num_of_segments | 8 | uimsbf |
| for(i=1; i<=num_of_segments; i++){ | | |
| segment_type | 8 | uimsbf |
| segment_length | 16 | uimsbf |
| segment_data_field() | | |
| } | | |
| CRC_32 | 32 | uimsbf |
| } | | |

