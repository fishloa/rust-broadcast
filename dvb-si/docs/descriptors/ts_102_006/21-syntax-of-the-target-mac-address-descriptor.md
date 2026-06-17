## Table 21 — Syntax of the target_MAC_address_descriptor
_§9.5.2, PDF pp. 26-26_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| target_MAC_address_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x07 |
| descriptor_length | 8 | uimsbf | |
| MAC_addr_mask | 48 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| MAC_addr_match | 48 | uimsbf | |
| } | | | |
| } | | | |

