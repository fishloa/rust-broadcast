## Table 24 — Syntax of the serial_number_descriptor
_§9.5.2, PDF pp. 27-27_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| target_serial_number_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x08 |
| descriptor_length | 8 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| serial_data_byte | 8 | uimsbf | |
| } | | | |
| } | | | |

