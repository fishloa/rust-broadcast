## Table 26 — External application authorization descriptor syntax
_§5.3.5.7, PDF pp. 43-43_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| external_application_authorisation_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x05 |
| descriptor_length | 8 | uimsbf |  |
| for(i=0; i<N; i++) { |  |  |  |
| application_identifier() |  |  |  |
| application_priority | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

