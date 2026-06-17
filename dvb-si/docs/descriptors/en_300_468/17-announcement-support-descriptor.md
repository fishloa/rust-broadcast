## Table 17 — Announcement support descriptor
_§6.2.3, PDF pp. 56-56_

| Syntax | Number of bits | Identifier |
|---|---|---|
| announcement_support_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| announcement_support_indicator | 16 | bslbf |
| for (i=0;i<N;i++) { |
| announcement_type | 4 | uimsbf |
| reserved_future_use | 1 | bslbf |
| reference_type | 3 | uimsbf |
| if (reference_type == 0x01 \|\| reference_type == 0x02 \|\| reference_type == 0x03) { |
| original_network_id | 16 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| component_tag | 8 | uimsbf |
| } |
| } |
| } |

