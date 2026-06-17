## Table 28 — Syntax of the target_IP_source slash_descriptor
_§8.4.5.3, PDF pp. 35-35_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| target_IP_source slash_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x10 |
| descriptor_length | 8 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| IPv4_source_addr | 32 | uimsbf |  |
| IPv4_source_slash_mask | 8 | uimsbf |  |
| IPv4_dest_addr | 32 | uimsbf |  |
| IPv4_dest_slash_mask | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

