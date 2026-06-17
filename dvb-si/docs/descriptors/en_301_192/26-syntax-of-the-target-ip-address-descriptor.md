## Table 26 — Syntax of the target_IP_address_descriptor
_§8.4.5.3, PDF pp. 34-34_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_IP_address_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x09 |
| descriptor_length | 8 | uimsbf |  |
| IPv4_addr_mask | 32 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| IPv4_addr | 32 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

