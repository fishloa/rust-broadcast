## Table 11d — Cell fragment info
_§5.2.11.3, PDF pp. 43-43_

| Syntax | Number of bits | Mnemonic |
|---|---|---|
| cell_fragment_info(){ |
| for (i=1;i<=N;i++) { |
| cell_fragment_id | 32 | uimsbf |
| first_occurence | 1 | bsblf |
| last_occurence | 1 | bsblf |
| if (first_occurence == 1) { |
| reserved_zero_future_use | 4 | bsblf |
| center_latitude | 18 | tcimsbf |
| reserved_zero_future_use | 5 | bsblf |
| center_longitude | 19 | tcimsbf |
| max_distance | 24 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| } else { |
| reserved_zero_future_use | 4 | bsblf |
| } |
| delivery_system_id_loop_count | 10 | uimsbf |
| for (j=0;j<delivery_system_id_loop_count;j++) { |
| delivery_system_id | 32 | uimsbf |
| } |
| reserved_zero_future_use | 6 | bsblf |
| new_delivery_system_id_loop_count | 10 | uimsbf |
| for (k=0;k<new_delivery_system_id_loop_count;k++) { |
| new_delivery_system_id | 32 | uimsbf |
| time_of_application_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| time_of_application_ext | 9 | uimsbf |
| } |
| reserved_zero_future_use | 6 | bsblf |
| obsolescent_delivery_system_id_loop_count | 10 | uimsbf |
| for (l=0;l<obsolescent_delivery_system_id_loop_count;l++) { |
| obsolescent_delivery_system_id | 32 | uimsbf |
| time_of_obsolescence_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| time_of_obsolescence_ext | 9 | uimsbf |
| } |
| } |
| } |

