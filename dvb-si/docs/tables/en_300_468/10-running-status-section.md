## Table 10 — Running status section
_§5.2.8, PDF pp. 38-38_

| Syntax | Number of bits | Identifier |
|---|---|---|
| running_status_section() { |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| reserved_future_use | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| for (i=0;i<N;i++) { |
| transport_stream_id | 16 |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| event_id | 16 | uimsbf |
| reserved_future_use | 5 | bslbf |
| running_status | 3 | uimsbf |
| } |
| } |

