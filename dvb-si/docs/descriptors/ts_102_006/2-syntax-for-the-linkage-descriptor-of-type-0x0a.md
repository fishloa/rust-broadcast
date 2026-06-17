## Table 2 — Syntax for the Linkage Descriptor of type 0x0A
_§6.1.1, PDF pp. 11-11_

| Syntax | No. of | Identifier |
|---|---|---|
| | bits | |
| linkage_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| linkage_type | 8 | uimsbf |
| if (linkage_type = 0x0A) { | | |
| table_type | 8 | bslbf |
| } | | |
| } | | |

