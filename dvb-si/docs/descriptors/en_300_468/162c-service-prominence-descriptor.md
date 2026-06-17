## Table 162c — service_prominence_descriptor
_§6.4.18, PDF pp. 150-150_

| Syntax | Number of bits | Identifier |
|---|---|---|
| service_prominence_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| SOGI_list_length | 8 | uimsbf |
| if (SOGI_list_length > 0) { |
| for (i=0;i<N;i++) { |
| SOGI_flag | 1 | bslbf |
| target_region_flag | 1 | bslbf |
| service_flag | 1 | bslbf |
| reserved_future_use | 1 | bslbf |
| SOGI_priority | 12 | uimsbf |
| if (service_flag == 0b1) { |
| service_id | 16 | uimsbf |
| } |
| if (target_region_flag == 0b1) { |
| target_region_loop_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| reserved_future_use | 5 | bslbf |
| country_code_flag | 1 | bslbf |
| region_depth | 2 | uimsbf |
| if (country_code_flag == 0b1) { |
| country_code | 24 | bslbf |
| } |
| if (region_depth >= 1) { |
| primary_region_code | 8 | uimsbf |
| if (region_depth >= 2) { |
| secondary_region_code | 8 | uimsbf |
| if (region_depth == 3) { |
| tertiary_region_code | 16 | uimsbf |
| } |
| } |
| } |
| } |
| } |
| } |
| } |
| for (i=0; i<N; i++) { |
| private_data_byte | 8 | bslbf |
| } |
| } |

