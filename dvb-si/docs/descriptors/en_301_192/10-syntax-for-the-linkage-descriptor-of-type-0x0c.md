## Table 10 — Syntax for the Linkage Descriptor of type 0x0C
_§8.2.2, PDF pp. 24-24_

| Syntax | No. of bits | Identifier |
|---|---|---|
| linkage_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| linkage_type | 8 | uimsbf |
| if (linkage_type == 0x0C){ |  |  |
| table_type | 8 | bslbf |
| if (table_type == 0x02){ |  |  |
| bouquet_id | 16 | uimsbf |
| } |  |  |
| } |  |  |
| } |  |  |

