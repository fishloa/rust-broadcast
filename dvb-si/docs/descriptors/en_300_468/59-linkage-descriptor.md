## Table 59 — Linkage descriptor
_§6.2.19.1, PDF pp. 84-84_

| Syntax | Number of bits | Identifier |
|---|---|---|
| linkage_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| linkage_type | 8 | uimsbf |
| if (linkage_type == 0x08) { |
| mobile_hand-over_info() |
| } else if (linkage_type == 0x0D) { |
| event_linkage_info() |
| } else if (linkage_type >= 0x0E && linkage_type <= 0x1F) { |
| extended_event_linkage_info() |
| } |
| for (i=0;i<N;i++) { |
| private_data_byte | 8 | bslbf |
| } |
| } |

