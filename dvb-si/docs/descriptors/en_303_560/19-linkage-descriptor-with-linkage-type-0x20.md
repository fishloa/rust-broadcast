## Table 19 — Linkage descriptor with linkage_type 0x20
_§5.3.2.2.2, PDF pp. 29-29_

| Syntax | Number of bits | Identifier |
|---|---|---|
| linkage_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| linkage_type | 8 | uimsbf |
| if (linkage_type == 0x20){ | | |
| font_count | 8 | uimsbf |
| for (i=0; i<font_count; i++){ | | |
| essential_font_download_flag | 1 | bslbf |
| font_id | 7 | uimsbf |
| } | | |
| for (i=0; i<N; i++){ | | |
| reserved_zero_future_use | 8 | bslbf |
| } | | |
| } | | |
| } | | |

