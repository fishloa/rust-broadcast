## Table 30 — Syntax of the target_IPv6_slash_descriptor
_§8.4.5.3, PDF pp. 36-36_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_IPv6_slash_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x11 |
| descriptor_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| IPv6_addr | 128 | uimsbf |  |
| IPv6_slash_mask | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

