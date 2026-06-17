## Table 45 — MPE-FEC section
_§9.9, PDF pp. 57-57_

| Syntax | No. of bits | Identifier |
|---|---|---|
| MPE-FEC_section () { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| private_indicator | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| padding_columns | 8 | uimsbf |
| reserved_for_future_use | 8 | bslbf |
| reserved | 2 | bslbf |
| reserved_for_future_use | 5 | bslbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| real_time_parameters() |  |  |
| for( i=0; i<N; i++ ) { |  |  |
| rs_data_byte | 8 | uimsbf |
| } |  |  |
| CRC_32 | 32 | uimsbf |
| } |  |  |

