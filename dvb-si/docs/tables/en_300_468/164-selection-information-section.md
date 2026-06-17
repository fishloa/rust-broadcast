## Table 164 — Selection information section
_§7.1.2, PDF pp. 154-154_

| Syntax | Number of bits | Identifier |
|---|---|---|
| selection_information_section() { |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| reserved_future_use | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| reserved_future_use | 16 | bslbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| reserved_future_use | 4 | bslbf |
| transmission_info_descriptors_length | 12 | uimsbf |
| for (i=0;i<N;i++) { |
| descriptor() |
| } |
| for (i=0;i<N;i++) { |
| service_id | 16 | uimsbf |
| reserved_future_use | 1 | bslbf |
| running_status | 3 | bslbf |
| service_descriptors_length | 12 | uimsbf |
| for (j=0;j<N;j++) { |
| descriptor() |
| } |
| } |
| CRC_32 | 32 | rpchof |
| } |

