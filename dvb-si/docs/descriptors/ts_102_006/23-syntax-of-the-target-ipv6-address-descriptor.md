## Table 23 — Syntax of the target_IPv6_address_descriptor
_§9.5.2, PDF pp. 27-27_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| target_IPv6_address_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x0A |
| descriptor_length | 8 | uimsbf | |
| IPv6_addr_mask | 128 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| IPv6_addr_match | 128 | uimsbf | |
| } | | | |
| } | | | |

