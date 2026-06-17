## Table 35 — Syntax of the message_descriptor
_§9.5.2.11, PDF pp. 33-33_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| message_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x04 |
| descriptor_length | 8 | uimsbf | |
| descriptor_number | 4 | uimsbf | |
| last_descriptor_number | 4 | uimsbf | |
| ISO_639_language_code | 24 | bslbf | |
| for (i=0; i<N; i++) { | | | |
| text_char | 8 | uimsbf | |
| } | | | |
| } | | | |

