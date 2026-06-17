## Table 20 — Syntax of the IP/MAC_platform_name_descriptor
_§8.4.5.2, PDF pp. 31-31_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| IP/MAC_platform_name_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x0C |
| descriptor_length | 8 | uimsbf |  |
| ISO_639_language_code | 24 | bslbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| text_char | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

