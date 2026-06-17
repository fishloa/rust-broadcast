## Table 25 — Application icons descriptor syntax
_§5.3.5.7, PDF pp. 43-43_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| application_icons_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x0B |
| descriptor_length | 8 | uimsbf |  |
| icon_locator_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| icon_locator_byte | 8 | uimsbf |  |
| } |  |  |  |
| icon_flags | 16 | bslbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| reserved_future_use | 8 | bslbf |  |
| } |  |  |  |
| } |  |  |  |

