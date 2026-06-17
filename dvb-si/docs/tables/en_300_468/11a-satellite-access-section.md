## Table 11a — Satellite access section
_§5.2.11.1, PDF pp. 40-40_

| Syntax | Number of bits | Identifier |
|---|---|---|
| satellite_access_section() { |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| private_indicator | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| satellite_table_id | 6 | uimsbf |
| table_count | 10 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| reserved_zero_future_use | 8 | bslbf |
| if (satellite_table_id == 0) { |
| satellite_position_v2_info() |
| } |
| else if (satellite_table_id == 1) { |
| cell_fragment_info() |
| } |
| else if (satellite_table_id == 2) { |
| time_association_info() |
| } |
| else if (satellite_table_id == 3) { |
| beamhopping_time_plan_info() |
| } |
| else if (satellite_table_id == 4) { |
| satellite_position_v3_info() |
| } |
| else { |
| for (i=0;i<N;i++) { |
| reserved_zero_future_use | 8 | bslbf |
| } |
| } |
| CRC_32 | 32 | rpchof |
| } |

