## Table 31 — Syntax of the target_IPv6_source_slash_descriptor
_§8.4.5.14, PDF pp. 37-37_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_IPv6_source_slash_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x12 |
| descriptor_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| IPv6_source_addr | 128 | uimsbf |  |
| IPv6_source_slash_mask | 8 | uimsbf |  |
| IPv6_dest_addr | 128 | uimsbf |  |
| IPv6_dest_slash_mask | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

