## Table 11g — Beamhopping time plan info
_§5.2.11.5, PDF pp. 47-47_

| Syntax | Number of bits | Mnemonic |
|---|---|---|
| beamhopping_time_plan_info() { |
| for (i=1;i<=N;i++) { |
| beamhopping_time_plan_id | 32 | uimsbf |
| reserved_zero_future_use | 4 | bsblf |
| beamhopping_time_plan_length | 12 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| time_plan_mode | 2 | uimsbf |
| time_of_application_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| time_of_application_ext | 9 | uimsbf |
| cycle_duration_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| cycle_duration_ext | 9 | uimsbf |
| if time_plan_mode == 0 { |
| dwell_duration_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| dwell_duration_ext | 9 | uimsbf |
| on_time_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| on_time_ext | 9 | uimsbf |
| } |
| if (time_plan_mode == 1) { |
| reserved_zero_future_use | 1 | bsblf |
| bit_map_size | 15 | uimsbf |
| reserved_zero_future_use | 1 | bsblf |
| current_slot | 15 | uimsbf |
| for (j=1;j<=bit_map_size;j++) { |
| slot_transmission_on | 1 | bsblf |
| } |
| for (k=1;k<=J;k++) { |
| padding_bit | 1 | bsblf |
| } |
| } |
| if (time_plan_mode == 2) { |
| grid_size_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| grid_size_ext | 9 | uimsbf |
| revisit_duration_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| revisit_duration_ext | 9 | uimsbf |
| sleep_time_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | uimsbf |
| sleep_time_ext | 9 | bsblf |
| sleep_duration_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| sleep_duration_ext | 9 | uimsbf |
| } |
| } |
| } |

