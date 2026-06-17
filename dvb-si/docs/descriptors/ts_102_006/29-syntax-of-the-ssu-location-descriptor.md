## Table 29 — Syntax of the SSU_location_descriptor
_§9.5.2.8, PDF pp. 29-29_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| SSU_location_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x03 |
| descriptor_length | 8 | uimsbf | |
| data_broadcast_id | 16 | uimsbf | |
| if (data_broadcast_id == 0x000A) { | | | |
| association_tag | 16 | uimsbf | |
| } | | | |
| for (i=0; i<N; i++) { | | | |
| private_data_byte | 8 | uimsbf | |
| } | | | |
| } | | | |

