## Table 31 — Syntax of selector bytes for OC transport
_§5.3.6.1, PDF pp. 45-45_

| Syntax | Bits | Identifier |
|---|---|---|
| remote_connection | 1 | bslbf |
| reserved_future_use | 7 | bslbf |
| if( remote_connection == "1" ) { |  |  |
| original_network_id | 16 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| } |  |  |
| component_tag | 8 | uimsbf |

