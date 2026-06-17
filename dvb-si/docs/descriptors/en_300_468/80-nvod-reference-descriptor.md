## Table 80 — NVOD reference descriptor
_§6.2.28, PDF pp. 96-96_

| Syntax | Number of bits | Identifier |
|---|---|---|
| NVOD_reference_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| transport_stream_id | 16 | uimsbf |
| original_network_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| } |
| } |

