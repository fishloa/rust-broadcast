## Table 21 — Syntax of the IP/MAC_platform_provider_name_descriptor
_§8.4.5.3, PDF pp. 32-32_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| IP/MAC_platform_provider_name_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x0D |
| descriptor_length | 8 | uimsbf |  |
| ISO_639_language_code | 24 | bslbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| text_char | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

