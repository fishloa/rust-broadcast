## Table 2 — MPE-IFEC section
_§5.2, PDF pp. 17-17_

| Syntax | Number of bits | Identifier |
|---|---|---|
| MPE-IFEC_section () { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | Bslbf |
| Private_indicator | 1 | Bslbf |
| Reserved | 2 | Bslbf |
| section_length | 12 | uimsbf |
| Burst_number | 8 | uimsbf |
| IFEC_burst_size | 8 | uimsbf |
| Reserved | 2 | Bslbf |
| Version | 5 | Uimsbf |
| Current_next_indicator | 1 | Bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 |  |
| real_time_parameters() | 32 | uimsbf |
| for( i=0; i<Nmax; i++ ) { |  |  |
| IFEC_data_byte | 8 | uimsbf |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

