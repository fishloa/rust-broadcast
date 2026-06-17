## Table 25 — Syntax of the target_MAC_address_range_descriptor
_§8.4.5.3, PDF pp. 34-34_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_MAC_address_range_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x0E |
| descriptor_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| MAC_addr_low | 48 | uimsbf |  |
| MAC_addr_high | 48 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

