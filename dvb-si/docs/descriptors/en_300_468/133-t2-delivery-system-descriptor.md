## Table 133 — T2 delivery system descriptor
_§6.4.6.3, PDF pp. 124-124_

| Syntax | Number of bits | Identifier |
|---|---|---|
| T2_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| plp_id | 8 | uimsbf |
| T2_system_id | 16 | uimsbf |
| if (descriptor_length > 4) { |
| SISO_MISO | 2 | bslbf |
| bandwidth | 4 | bslbf |
| reserved_future_use | 2 | bslbf |
| guard_interval | 3 | bslbf |
| transmission_mode | 3 | bslbf |
| other_frequency_flag | 1 | bslbf |
| tfs_flag | 1 | bslbf |
| for (i=0;i<N;i++) { |
| cell_id | 16 | uimsbf |
| if (tfs_flag == 0b1) { |
| frequency_loop_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| centre_frequency | 32 | uimsbf |
| } |
| } else { |
| centre_frequency | 32 | uimsbf |
| } |
| subcell_info_loop_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| cell_id_extension | 8 | uimsbf |
| transposer_frequency | 32 | uimsbf |
| } |
| } |
| } |
| } |

