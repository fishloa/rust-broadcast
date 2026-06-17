## Table 34 — Syntax of the SSU_event_name_descriptor
_§9.5.2.11, PDF pp. 32-32_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| SSU_event_name_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x05 |
| descriptor_length | 8 | uimsbf | |
| ISO_639_language_code | 24 | bslbf | |
| name_length | 8 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| name_char | 8 | uimsbf | |
| } | | | |
| text_length | 8 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| text_char | 8 | uimsbf | |
| } | | | |
| } | | | |

