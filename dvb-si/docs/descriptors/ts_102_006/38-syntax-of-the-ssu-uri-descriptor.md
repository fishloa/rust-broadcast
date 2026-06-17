## Table 38 — Syntax of the ssu_uri_descriptor
_§9.5.2.11, PDF pp. 35-35_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| ssu_uri_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x0D |
| descriptor_length | 8 | uimsbf | |
| max_holdoff_time | 8 | uimsbf | |
| min_polling_interval | 8 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| uri_char | 8 | bslbf | |
| } | | | |
| } | | | |

