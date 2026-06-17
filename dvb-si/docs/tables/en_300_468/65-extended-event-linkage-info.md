## Table 65 — Extended event linkage info
_§6.2.19.4, PDF pp. 87-87_

| Syntax | Number of bits | Identifier |
|---|---|---|
| extended_event_linkage_info() { |
| loop_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| target_event_id | 16 | uimsbf |
| target_listed | 1 | bslbf |
| event_simulcast | 1 | bslbf |
| link_type | 2 | uimsbf |
| target_id_type | 2 | uimsbf |
| original_network_id_flag | 1 | bslbf |
| service_id_flag | 1 | bslbf |
| if (target_id_type == 3) { |
| user_defined_id | 16 | uimsbf |
| } else { |
| if (target_id_type == 1) { |
| target_transport_stream_id | 16 | uimsbf |
| } |
| if (original_network_id_flag == 0b1) { |
| target_original_network_id | 16 | uimsbf |
| } |
| if (service_id_flag == 0b1) { |
| target_service_id | 16 | uimsbf |
| } |
| } |
| } |
| } |

