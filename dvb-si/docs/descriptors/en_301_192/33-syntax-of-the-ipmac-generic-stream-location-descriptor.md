## Table 33 — Syntax of the IP/MAC_generic_stream_location_descriptor
_§8.4.5.15, PDF pp. 38-38_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| IP/MAC_generic_stream_location_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x15 |
| descriptor_length | 8 | uimsbf |  |
| interactive_network_id | 16 | uimsbf |  |
| modulation_system_type | 8 | uimsbf |  |
| modulation_system_id | 16 | uimsbf |  |
| PHY_stream_id | 16 | uimsbf |  |
| selector_length_flag | 1 | uimsbf |  |
| if (selector_length_flag == 0) { |  |  |  |
| selector_flags | 7 | bslbf |  |
| } |  |  |  |
| else { |  |  |  |
| selector_length | 7 | uimsbf |  |
| for (i=0; i<N; i++) { |  |  |  |
| selector_byte | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |
| } |  |  |  |

