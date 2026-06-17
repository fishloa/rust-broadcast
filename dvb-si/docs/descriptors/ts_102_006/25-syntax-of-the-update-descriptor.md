## Table 25 — Syntax of the update_descriptor
_§9.5.2, PDF pp. 28-28_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| update_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x02 |
| descriptor_length | 8 | uimsbf | |
| update_flag | 2 | bslbf | see Table 26 |
| update_method | 4 | bslbf | see Table 27 |
| update_priority | 2 | bslbf | 11 |
| for (i=0; i<N; i++) { | | | |
| private_data_byte | 8 | uimsbf | |
| } | | | |
| } | | | |

