## Table 39 — Service identifier descriptor
_§7.2, PDF pp. 62-62_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| service_identifier_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x71 |
| descriptor_length | 8 | uimsbf |  |
| for (i = 0; i < descriptor_length; i++) { |  |  |  |
| textual_service_identifier_bytes | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

