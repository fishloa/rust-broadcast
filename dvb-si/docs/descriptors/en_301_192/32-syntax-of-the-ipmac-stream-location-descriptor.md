## Table 32 — Syntax of the IP/MAC_stream_location_descriptor
_§8.4.5.14, PDF pp. 37-37_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| IP/MAC_stream_location_descriptor () { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x13 |
| descriptor_length | 8 | uimsbf |  |
| network_id | 16 | uimsbf |  |
| original_network_id | 16 | uimsbf |  |
| transport_stream_id | 16 | uimsbf |  |
| service_id | 16 | uimsbf |  |
| component_tag | 8 | uimsbf |  |
| } |  |  |  |

