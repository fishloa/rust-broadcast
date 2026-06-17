## Table 22 — Syntax of the target_serial_number_descriptor
_§8.4.5.3, PDF pp. 32-32_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_serial_number_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x08 |
| descriptor_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| serial_data_byte | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

