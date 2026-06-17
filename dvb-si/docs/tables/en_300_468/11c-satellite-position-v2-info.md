## Table 11c — Satellite position v2 info
_§5.2.11.2, PDF pp. 42-42_

| Syntax | Number of bits | Mnemonic |
|---|---|---|
| satellite_position_v2_info() { |
| for (i=1;i<=N;i++) { |
| satellite_id | 24 |
| reserved_zero_future_use | 7 |
| position_system | 1 |
| if (position_system == 0) { |
| orbital position | 16 | bslbf |
| west_east_flag | 1 | bslbf |
| reserved_zero_future_use | 7 | bslbf |
| } |
| if (position_system == 1) { |
| epoch_year | 8 | uimsbf |
| day_of_the_year | 16 | uimsbf |
| day_fraction | 32 | spfmsbf |
| mean_motion_first_derivative | 32 | spfmsbf |
| mean_motion_second_derivative | 32 | spfmsbf |
| drag_term | 32 | spfmsbf |
| inclination | 32 | spfmsbf |
| right_ascension_of_the_ascending_node | 32 | spfmsbf |
| eccentricity | 32 | spfmsbf |
| argument_of_perigree | 32 | spfmsbf |
| mean_anomaly | 32 | spfmsbf |
| mean_motion | 32 | spfmsbf |
| } |
| } |
| } |

