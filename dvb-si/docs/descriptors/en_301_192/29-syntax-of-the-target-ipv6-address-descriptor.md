## Table 29 — Syntax of the target_IPv6_address_descriptor
_§8.4.5.3, PDF pp. 36-36_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_IPv6_address_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x0A |
| descriptor_length | 8 | uimsbf |  |
| IPv6_addr_mask | 128 | uimsbf |  |
| for (I=0;i<N;I++) { |  |  |  |
| IPv6_addr | 128 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

