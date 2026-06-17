## Table 22 — Syntax of the target_IP_address_descriptor
_§9.5.2, PDF pp. 26-26_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| target_IP_address_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x09 |
| descriptor_length | 8 | uimsbf | |
| IP_addr_mask | 32 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| IP_addr_match | 32 | uimsbf | |
| } | | | |
| } | | | |

