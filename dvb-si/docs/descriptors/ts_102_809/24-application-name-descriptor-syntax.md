## Table 24 — Application name descriptor syntax
_§5.3.5.6.2, PDF pp. 42-42_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| application_name_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x01 |
| descriptor_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| ISO_639_language_code | 24 | bslbf |  |
| application_name_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| application_name_char | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |
| } |  |  |  |

