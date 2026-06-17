## Table 9 — Syntax for the linkage descriptor of type 0x0B
_§8.2.1, PDF pp. 23-23_

| Syntax | No. of bits | Identifier |
|---|---|---|
| linkage_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| linkage_type | 8 | uimsbf |
| if (linkage_type == 0x0B) { |  |  |
| platform_id_data_length | 8 | uimsbf |
| for (i=0; i<N; i++) { |  |  |
| platform_id | 24 | uimsbf |
| platform_name_loop_length | 8 | uimsbf |
| for (i=0; i<N; i++) { |  |  |
| ISO_639_language_code | 24 | bslbf |
| platform_name_length | 8 | uimsbf |
| for (i=0; i<platform_name_length; i++) |  |  |
| { |  |  |
| text_char | 8 | uimsbf |
| } |  |  |
| } |  |  |
| } |  |  |
| for (i=0; i<N; i++) { |  |  |
| private_data_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |
| } |  |  |

