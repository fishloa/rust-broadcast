## Table 24 — Syntax of the target_MAC_address_descriptor
_§8.4.5.3, PDF pp. 33-33_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_MAC_address_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x07 |
| descriptor_length | 8 | uimsbf |  |
| MAC_addr_mask | 48 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| MAC_addr | 48 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

