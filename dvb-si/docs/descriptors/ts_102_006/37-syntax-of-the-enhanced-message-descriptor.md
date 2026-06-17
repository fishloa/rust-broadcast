## Table 37 — Syntax of the enhanced_message_descriptor
_§9.5.2.11, PDF pp. 34-34_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| enhanced_message_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x0C |
| descriptor_length | 8 | uimsbf | |
| descriptor_number | 4 | uimsbf | |
| last_descriptor_number | 4 | uimsbf | |
| ISO_639_language_code | 24 | bslbf | |
| reserved_for_future_use | 3 | bslbf | |
| message_index | 5 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| text_char | 8 | uimsbf | |
| } | | | |
| } | | | |

