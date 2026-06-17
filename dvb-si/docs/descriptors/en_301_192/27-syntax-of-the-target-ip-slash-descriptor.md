## Table 27 — Syntax of the target_IP_slash_descriptor
_§8.4.5.3, PDF pp. 35-35_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_IP_slash_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x0F |
| descriptor_length | 8 | uimsbf |  |
| for (I=0; i<N; i++) { |  |  |  |
| IPv4_addr | 32 | uimsbf |  |
| IPv4_slash_mask | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

